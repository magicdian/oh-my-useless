# refactor: event-driven sensor architecture

## Goal

Replace the current polling-first supervisor with a layered, event-driven sensor architecture. Kernel pressure and session changes arrive as file-descriptor events through one reactor; polling survives only as a bounded fallback for values the kernel cannot notify on. Memory actuation is gated by live headroom, so the daemon cannot push the host into OOM even when an unmodelled workload is present.

The user-visible outcome: `useless` runs while the host is genuinely quiet, yields within milliseconds when real work starts, and never causes a business process to be OOM-killed.

## Background

### Incident

The 2026-11-13 analysis of `VM-0-10-debian` (K3s single node, 1.9 GiB RAM, no swap) recorded 21 OOM kills in under eight hours. The 02:27:23 scan was invoked by the `useless-memory` thread, and Flux controllers restarted 15-16 times each.

### Code evidence

* `src/workers/memory.rs:20-30` — the memory target is `MemTotal × max_percent × duty_percent`; `MemAvailable` is never consulted.
* `src/workers/memory.rs:48-50` — `Vec::resize(target, 0)` commits and zero-fills the whole target in one step.
* `src/policy.rs:84-95` — pause is driven only by interactive sessions, `load1 / cores ≥ pause_load_percent`, or `MemAvailable ≤ pause_mem_available_percent`.
* `src/sensor.rs:127-145` — interactive detection is `who -u --ips` plus `ps -eo args`; only TTY sessions count.
* `src/daemon.rs:81-83` — the supervisor samples every `check_interval_secs` (5 s by default).
* `src/install.rs:162-181` — the systemd unit sets `Nice=19` only; no OOM score adjustment and no cgroup memory bounds.

### Measured arithmetic on the affected host

MemTotal ≈ 1946 MB, baseline `MemAvailable` 600-650 MB, configured `memory.max_percent = 35` ⇒ burst target ≈ 681 MB. The pause threshold (10% ≈ 195 MB) and caution threshold (20% ≈ 389 MB) are both smaller than a single burst, so the guard cannot fire before the first OOM regardless of sampling speed. This is a structural defect, not a tuning problem.

### Prior decisions

* Archived PRD `.phronia/tasks/archive/2026-04/04-15-useless-rust-daemon/prd.md`: no eBPF in the MVP; keep the sensor layer abstract enough for a future eBPF-backed observer.
* User decision 2026-11-14: no eBPF now; proceed with the layered event-driven design; no point solutions that leave future technical debt.

## Requirements

**R1 — Layered boundaries.** Policy must be a pure, deterministic function of signals and time. OS access must live behind adapter traits. Workers must consume directives only. No adapter may call policy directly, and no domain type may read the filesystem, clock, or config file.

**R2 — Event-first sensing.** Host and cgroup pressure must arrive through PSI trigger file descriptors and `poll`/`epoll`. Login/logout must arrive through inotify. Signals and timers must use `signalfd` and `timerfd`. No fixed high-frequency loop may remain as the primary control path.

**R3 — Bounded polling fallback.** Polling is allowed only for values without a kernel notification: absolute `MemAvailable`, swap counters, cgroup topology discovery, tty idle time for existing sessions, and external network throughput. Every fallback must have an adaptive cadence tied to policy mode and must stop when there is nothing to observe.

**R4 — Headroom safety invariant.** At any instant, the memory worker's resident synthetic allocation plus one pending chunk must not exceed `MemAvailable − reserve`, where `reserve = max(reserve_mb, MemTotal × reserve_percent / 100)`. The check must run immediately before each growth step, never against a cached sample. Pause must release memory immediately, not on the next worker tick.

**R5 — Ambient workload detection.** Any sustained PSI pressure in the host or in watched business cgroups must suspend synthetic work without requiring a human session. Interactive sessions remain a high-priority signal but are no longer the only proxy for real work.

**R6 — Self-containment.** The installed systemd unit must make `useless` the first casualty of memory exhaustion (`OOMScoreAdjust=1000`), bound its own cgroup (`MemoryHigh`, `MemoryMax`), and deprioritize its I/O (`IOSchedulingClass=idle`). A runaway allocator must be contained inside the unit's cgroup.

**R7 — Observability.** Every mode transition must be journaled with the triggering signal, scope, reason, budget, and headroom. `useless status` must report active sensor backends, current mode and reason, and the last transition. Journal storage must be bounded.

**R8 — Compatibility.** Existing config keys keep their meaning and defaults. New keys are optional. `install`, `uninstall`, `reload`, `status`, `print-default-config`, and the init.d fallback keep working. A host that lacks PSI, cgroup v2, or inotify must degrade gracefully and report the degradation, never abort the daemon.

**R9 — No new heavyweight runtime.** No eBPF, no proc connector, no async runtime, no logging framework. The only permitted new dependency is `libc`, for ABI-correct syscall bindings; the justification is recorded as an ADR in `design.md`.

## Task map

| Child | Deliverable | Depends on |
|---|---|---|
| `11-14-domain-policy-ports` | Pure domain (`Signal`, `PolicyState`, reducer, budget math, burst schedule), port traits, store with generation, in-memory journal; behavior-preserving rewiring. | — |
| `11-14-reactor-push-sensors` | `libc`-backed sys layer, single reactor, `signalfd`/`timerfd`, host PSI trigger watcher, adaptive sampler fallback, capability reporting, journal file and state snapshot. | domain-policy-ports |
| `11-14-cgroup-memory-budget` | Per-cgroup PSI scopes, headroom-derived memory budget, chunked mmap ledger with JIT recheck, new memory/reserve config keys, systemd self-containment. | reactor-push-sensors |
| `11-14-event-driven-sessions` | inotify login/logout source, tty idle probe for existing sessions only, `who`/`ps` demoted to fallback adapter. | reactor-push-sensors |
| `11-14-network-jit-and-docs` | Just-in-time external-traffic gate for network bursts, README/spec/default-config updates. | reactor-push-sensors, cgroup-memory-budget |

Children are ordered for execution because several of them touch `daemon.rs` and `config.rs`; the tree does not encode dependencies. Serial order: domain → reactor → cgroup-memory → sessions → network-docs.

## Cross-child acceptance criteria

* [ ] AC1 `cargo fmt --check`, `cargo check`, `cargo test` pass at the end of every child and at integration.
* [ ] AC2 The reducer is pure: the same `(state, signal, now)` sequence produces identical directives with no filesystem, clock, or environment access. A behavior-preservation test compares it against the pre-refactor policy for a table of samples.
* [ ] AC3 With PSI available, a synthetic pressure event causes `Suspended` in under one reactor wake-up (measured < 100 ms on the test VM), without waiting for `check_interval_secs`.
* [ ] AC4 On a 2 vCPU / 2 GiB VM under a scripted workload, `useless` never allocates beyond `MemAvailable − reserve`; no OOM kill attributable to `useless` occurs during a 4-hour soak.
* [ ] AC5 Suspend releases the synthetic memory allocation within one reactor wake-up; no multi-second tail remains.
* [ ] AC6 Login pauses within one event; logout resumes only after `interactive_grace_secs` plus `resume_quiet_secs` of clean signals.
* [ ] AC7 The installed unit contains `OOMScoreAdjust=1000`, `MemoryHigh`/`MemoryMax`, and `IOSchedulingClass=idle`; `systemctl show useless` reflects them.
* [ ] AC8 `useless status` reports sensor backends and the last transition, and distinguishes "pressure detected", "interactive session", "collector degraded", and "running".
* [ ] AC9 Degradation paths are exercised: no PSI, no cgroup v2, no inotify, unwritable journal. Each falls back without panic and records the reason.
* [ ] AC10 Existing config files parse unchanged; `print-default-config` includes the new keys with documented defaults.

## Out of scope

* eBPF, proc connector, fanotify, or per-packet capture.
* Per-process attribution or a dashboard.
* Non-Linux platforms.
* Changing the product's purpose or default aggressiveness presets.
* Automatic regeneration of the systemd unit on `reload`; `install` regenerates it, and this is documented.

## Open questions

None. All product and scope decisions are resolved; remaining choices are recorded as ADRs in `design.md`.

## Technical notes

* Kernel capability matrix: PSI and per-cgroup `*.pressure` require `CONFIG_PSI`; PSI triggers and `poll` support require Linux 5.8+; cgroup v2 events require unified hierarchy; inotify on `/var/run/utmp` and `/run/systemd/sessions` requires the respective paths. Debian 12 (Linux 6.1) satisfies all of them.
* The daemon remains a single process with one reactor thread and one thread per resource worker.
* No behavior change is allowed in child 1; it exists to make every later change testable.
