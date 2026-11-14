# Design: event-driven sensor architecture

## 1. Design goal

One reactor thread owns every kernel notification source. Adapters translate kernel facts into a small `Signal` vocabulary. A pure reducer turns signals into state and directives. Workers apply directives and never make policy decisions. Polling exists only where the kernel cannot notify, and its cadence follows the policy mode.

```text
kernel fds ──► runtime::Reactor ──► Signal ──► domain::reduce ──► Store ──► workers
(PSI, inotify,      (epoll + timerfd)            (pure)          (generation
 signalfd, pipes)                                                  + condvar)
                                                       │
                                                       ├──► journal ring + /run/useless/journal.jsonl
                                                       └──► state snapshot /run/useless/state.json
```

## 2. Layer boundaries

| Layer | May do | Must not do |
|---|---|---|
| `domain` | Hold `Signal`, `PolicyState`, `Directive`, budget and schedule math; run under an injected `now`. | Read files, call `Instant::now()` internally, spawn threads, log, parse config. |
| `ports` | Declare traits (`EventSource`, `Sampler`, `Clock`, `StatePublisher`, `JournalSink`). | Depend on concrete adapters. |
| `adapters` | Touch `/proc`, `/sys`, `/dev`, sockets, files; emit `Signal`. | Call `reduce`, mutate policy, decide directives. |
| `runtime` | Own epoll/timerfd/signalfd, deadlines, store publication, journal I/O. | Contain resource-specific business rules. |
| `workers` | Apply directives to CPU/memory/network. | Read config directly, compute policy, decide when to pause. |
| `daemon` | Compose adapters, reactor, reducer, workers; own lifecycle and pid file. | Contain policy branches or parsing heuristics. |

`domain` and `ports` are the stable contract. A future eBPF adapter would implement the same traits and emit the same signals without touching policy.

## 3. Core contracts

### 3.1 Signal

```rust
pub enum Signal {
    Pressure { class: PressureClass, scope: PressureScope, reading: PressureReading },
    Sample(SampleSnapshot),                 // fallback sampler: load, MemAvailable, swap, net
    Sessions(SessionSet),
    SessionIdle { tty: TtyId, idle: Duration },
    ExternalTraffic { ingress_bps: u64, egress_bps: u64 },
    TimerFired { kind: TimerKind },
    ConfigReloaded(Box<Config>),
    SourceHealth { source: SourceId, health: SourceHealth },
}
```

`PressureClass = { Cpu, Memory, Io }`. `PressureScope = Host | Cgroup(String)`. `PressureReading` carries `some`/`full` windows (`avg10`, `avg60`, `avg300`, `total`) as parsed from the PSI file. Signals are facts with timestamps, not decisions. Child 1 implements `Sample`, `Sessions`, and a precursor `SourceDegraded { source, message }`; child 2 adds `Pressure` and keeps `SourceDegraded` for adapter failures. The richer `SourceHealth` and `ExternalTraffic`/`SessionIdle` variants land with the children that need them.

### 3.2 Reducer

```rust
pub fn reduce(state: &mut PolicyState, signal: &Signal, config: &Config, now: Instant) -> PolicyOutput;
```

The reducer is total (no panics), deterministic, and allocation-light. It owns:

* mode transitions (`Running`, `Cooling`, `Suspended`) and reasons;
* hysteresis (immediate enter, quiet-window exit, slow start);
* session grace and idle classification;
* budget computation;
* burst/rest schedule advancement.

`Clock` is only needed by the runtime; `now` is always passed in.

### 3.3 Directives and store

```rust
pub struct PolicyOutput {
    pub mode: Mode,
    pub reasons: Vec<ReasonCode>,
    pub summary: String,
    pub cpu: ModuleDirective,       // allowed + duty
    pub memory: ModuleDirective,
    pub network: ModuleDirective,
    pub budget: Budget,             // computed for the child 3 ledger
}
```

`Store` = `Mutex<StoreState>` + `Condvar`; `StoreState` holds `Arc<Config>`, the latest `PolicyOutput`, and the generation counter, which `publish` increments. Workers wait on the condvar with a 1 s safety timeout and apply the latest generation idempotently. This removes the current 1 s polling loop and gives sub-millisecond pause propagation.

## 4. Event sources and why each is (or is not) event-driven

| Fact | Mechanism | Event | Fallback |
|---|---|---|---|
| CPU/memory/IO contention (host) | `/proc/pressure/{cpu,memory,io}` trigger + `poll(POLLPRI|POLLERR)` | yes | sampler when PSI is absent |
| Contention inside business cgroups | `<cgroup>/{cpu,memory,io}.pressure` with the same trigger interface | yes | sampler |
| Login/logout | inotify on `/run/systemd/sessions`; fallback `/var/run/utmp` | yes | `who -u --ips` sampler |
| Session idle time | tty atime via `stat("/dev/pts/N")` | no | probe only while sessions exist, 5 s → 30 s |
| Absolute memory headroom | `/proc/meminfo` `MemAvailable` | no | adaptive sampler + JIT check before growth |
| Swap | `/proc/vmstat` counters | no | sampled with meminfo |
| cgroup membership change | `cgroup.events` (`populated`) is pollable | yes (file level) | 60 s topology enumeration; cgroupfs has no directory inotify |
| External network throughput | interface counters | no | two reads around the burst start, minus own bytes |
| Process fork/exec | proc connector is deprecated and a firehose | no | intentionally not used |
| Signals | `signalfd` | yes | — |
| Deadlines | `timerfd` with the earliest deadline | yes | — |

### 4.1 PSI trigger rules

* One trigger per open file description. Open the pressure file once per `(class, scope, threshold)`.
* Register by writing `"some <stall_us> <window_us>"`; `full` uses a separate fd.
* Wait with `poll`/`epoll` for `POLLPRI`; treat `POLLERR` as a health event.
* A second write on an already-armed fd fails with `EBUSY`; to change a threshold, close and reopen.
* The trigger reports entering the stall condition, not leaving it. Exit is detected by the quiet-window timer plus the next fallback sample (`MemAvailable`, load); the PSI file is not re-read because triggers report entry only.
* Notifications are rate-limited to one per tracking window; the reducer must not assume one event per incident.

### 4.2 cgroup scopes

Discover slices at startup and every 60 s (or SIGHUP). Default watch list: `kubepods.slice`, `user.slice`; `system.slice` is opt-in. Resolve the daemon's own cgroup from `/proc/self/cgroup` and exclude it, and exclude its descendants. Missing slices are a health note, not a failure. On a non-Kubernetes host the list simply matches fewer paths.

## 5. State machine

```text
              pressure/session event
   Running ────────────────────────────► Suspended
      ▲                                     │
      │  quiet window + clean probe         │ quiet timer
      └────────────── Cooling ◄─────────────┘
                       │ pressure again
                       └──────────────► Suspended
```

* **Enter** is immediate and unconditional for any pressure or session signal. No debounce.
* **Exit** requires all of: no pressure event for `resume_quiet_secs`, a fresh fallback sample at the end of the window (`MemAvailable > reserve + chunk`, load below the pause threshold), and no active session. The fallback sample is the positive probe because the trigger does not report exits; "absence of events" is not evidence of quiet.
* **Slow start**: after a resume, memory and network grow in three steps (25%, 50%, 100%) spaced across `resume_ramp_secs`; CPU follows the existing burst schedule.
* `ReasonCode` is an ordered set; the highest-severity active reason is reported, all are journaled.

## 6. Memory model

### 6.1 Budget

```text
reserve       = max(reserve_mb, MemTotal * reserve_percent / 100)
ratio_cap     = MemTotal * memory.max_percent / 100 * duty_percent / 100
headroom_cap  = max(0, MemAvailable - reserve)
absolute_cap  = memory.max_absolute_mb  (0 = disabled)
budget        = min(ratio_cap, headroom_cap, absolute_cap?)
```

`budget` is recomputed on every sample and on every growth step. A stale budget is never used for actuation.

### 6.2 Ledger

* Allocation unit: `chunk_mb` (default 16 MiB) of anonymous private memory via `mmap`.
* Grow: allocate one chunk only if `MemAvailable - reserve >= current + chunk`. PSI pressure is enforced by the directive gate (`Suspended` ⇒ `allowed=false` ⇒ immediate release). Touch one byte per page so the pages are resident.
* Shrink: `madvise(MADV_DONTNEED)` then `munmap` per chunk, newest first.
* Suspend: release every chunk immediately from the worker thread woken by the condvar.
* No `Vec::resize` of a multi-hundred-MB region anywhere. No single allocation larger than `chunk_mb`.

This replaces the current single-shot commit that produced the `__GFP_ZERO` OOM trace.

## 7. Configuration (canonical)

Existing keys keep their names, meaning, and defaults. New keys are optional.

| Key | Default | Meaning |
|---|---|---|
| `pressure_enter_ms` | `50` | PSI stall milliseconds inside the window that triggers `Suspended`. |
| `pressure_window_ms` | `1000` | PSI tracking window. |
| `resume_quiet_secs` | `90` | Clean time required before leaving `Suspended`. |
| `resume_ramp_secs` | `90` | Duration of the 25/50/100% slow start. |
| `reserve_mb` | `512` | Absolute reserve floor. |
| `reserve_percent` | `10` | Reserve as a share of MemTotal; `reserve` is the larger of the two. |
| `[memory] max_absolute_mb` | `0` | Absolute cap; `0` disables it. |
| `[memory] chunk_mb` | `16` | Growth/shrink granularity. |
| `[sensor] watch_slices` | `["kubepods.slice", "user.slice"]` | cgroup pressure scopes. |
| `[network] external_traffic_kbps` | `1024` | Skip a burst when estimated external throughput exceeds this. |
| `[network] probe_window_ms` | `1000` | Two-sample window for the external-traffic estimate. |

Validation additions: `pressure_window_ms > 0`; `pressure_enter_ms <= pressure_window_ms`; `reserve_percent < 100`; `chunk_mb > 0`; `resume_quiet_secs` and `resume_ramp_secs` may be `0` for testing.

`check_interval_secs` keeps its name but becomes the *base* sampler cadence: `Running` = base, `Cooling` = 2×, `Suspended` = min(4×, 60 s).

## 8. Self-containment (systemd)

The installer derives the cgroup ceiling from the effective config and `MemTotal` at install time:

```text
ceiling = min(MemTotal * memory.max_percent / 100,
              memory.max_absolute_mb if > 0 else infinity)
MemoryHigh = ceiling + 64M
MemoryMax  = ceiling + 128M
```

Unit additions: `OOMScoreAdjust=1000`, `IOSchedulingClass=idle`, `CPUWeight=10`, `Restart=on-failure`, `RestartSec=10`, `StartLimitIntervalSec=300`, `StartLimitBurst=5`. `Nice=19` stays.

`reload` does not rewrite the unit; `install` does. This is documented in the contract spec.

## 9. Observability

* In-memory ring: last 512 transitions, always available to `status` through the state snapshot file.
* `/run/useless/journal.jsonl`: transition-only JSON Lines, bounded to ~256 KiB by keeping the last half. `/run` is tmpfs, so the file is capped by design and cleared on boot.
* `/run/useless/state`: current mode, reasons, budget, available memory, capabilities, summary, and last transition as line-based `key=value`. Written on transition and at most once per 30 s heartbeat. Line-based rather than JSON so `status` needs no parser in a std-only binary.
* `status` prints capabilities (`psi`, `cgroup_v2`, `inotify`, `trigger_write`), mode and reason, budget/headroom, and the last transition. Diagnostics stay on stderr; the status block stays on stdout.
* Log-on-transition is preserved; no per-tick logs.

## 10. Failure and degradation

| Failure | Behavior |
|---|---|
| PSI missing or `trigger` write fails | `SourceDegraded`; sampler-only mode; log once; `status` shows it. |
| No cgroup v2 | Host PSI plus `MemAvailable` only. |
| inotify unavailable | `who -u --ips` sampler at base cadence; behavior matches today. |
| `MemAvailable` unreadable | `Suspended` (fail closed), same as today's sensor-error path. |
| Journal file unwritable | Keep the in-memory ring; log the failure once; never crash. |
| A watched cgroup disappears | Drop its fds, emit a health note, re-discover on the next pass. |
| Reactor `poll` error | Rebuild the fd set once; if it repeats, degrade to sampler mode. |

## 11. ADRs

**ADR-1 — Add `libc` as the only new dependency.** Hand-rolled FFI must reproduce `epoll_event` packing (packed on x86_64, naturally aligned elsewhere), `itimerspec`, `signalfd_siginfo`, and dozens of constants. A subtle ABI mistake is precisely the kind of latent defect this task is meant to avoid. `libc` is small, stable, and audited. Rejected: a hand-written `sys` module, and shelling out to `inotifywait`.

**ADR-2 — One reactor thread, not one per module.** All notification fds share one deadline queue and one store. This keeps shutdown deterministic and prevents three independent polling loops from reappearing. Workers remain separate threads because they are long-running actuators.

**ADR-3 — PSI is the primary pressure signal; `load1` is demoted.** PSI measures contention (stalled tasks), not utilization, and is already aggregated by the kernel. `load1` is retained only in the degraded sampler because it is available everywhere.

**ADR-4 — Journal to `/run`, bounded.** Transition history is diagnostic, not durable audit data. `/run` is tmpfs, avoids unbounded disk writes on a small host, and journald already captures stderr.

**ADR-5 — Watch business cgroups, not every process.** `kubepods.slice` and `user.slice` capture real work on the two deployment shapes this project sees, without per-process accounting or eBPF. `system.slice` is opt-in because the daemon itself lives there.

**ADR-6 — No eBPF.** Recorded in the parent PRD and the prior archived PRD. The architecture leaves an adapter-shaped hole for it.

## 12. Rollout and rollback

* Child 1 is behavior-preserving; it can be reverted without changing runtime behavior.
* Child 2 introduces the reactor but keeps every existing fallback, so PSI absence is not a regression.
* Child 3 contains the incident fix. If it must be reverted, the deployment must also set `memory.enabled=false` on multi-workload hosts until a replacement lands.
* Child 4 and child 5 are additive and independently revertible.
* Each child updates the same contract spec; run them serially to avoid spec and `config.rs` conflicts.
