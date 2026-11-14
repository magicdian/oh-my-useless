# feat: epoll reactor with push-based host pressure

## Goal

Replace the supervisor's fixed sleep loop with one epoll reactor. Host PSI triggers, signals, and deadlines arrive as fd events; a bounded adaptive sampler remains for values without kernel notification. Add capability reporting and a transition journal so the daemon can explain why it stopped.

## Background

* Today `src/daemon.rs:45-84` samples every `check_interval_secs` and sleeps in 200 ms slices to notice signals.
* Today only `load1` and `MemAvailable` decide pressure; `/proc/pressure/*` is not read anywhere.
* The memory worker's actuation can lag a decision by up to one second, and the decision itself can lag reality by `check_interval_secs`.
* Child 1 provides the pure reducer, store, and journal ring this child drives.

## Requirements

* R2.1 Add `libc` as the only new dependency, behind a single `src/sys.rs` module. `sys` exposes `epoll_create`/`epoll_ctl`/`epoll_wait`, `timerfd_create`/`timerfd_settime`, `signalfd`, `inotify_init1` (used in child 4), `read`/`pread`/`write`, `close`, and error conversion to `AppError`.
* R2.2 `src/runtime/reactor.rs` owns one epoll set and one earliest-deadline timer. Sources register readable fds plus an opaque token; the reactor dispatches `Signal`s and never blocks on a single source.
* R2.3 Signals (`SIGHUP`, `SIGINT`, `SIGTERM`) are delivered through `signalfd`; all threads block them with `pthread_sigmask`. The current raw `signal()` handlers and 200 ms sleep loop are removed.
* R2.4 `src/adapters/psi.rs` watches `/proc/pressure/{cpu,memory,io}`. It registers `some` thresholds for each metric, opens one fd per trigger, handles `POLLPRI`/`POLLERR`, and reopens fds when a threshold changes. Enter events emit `Signal::Pressure`. Exit is detected by the quiet-window probe, not by assuming a matching notification.
* R2.5 A `MemInfoSampler` and a load sampler provide the fallback `Signal::Sample`. Their cadence is `base` in `Running`, `2× base` in `Cooling`, `min(4× base, 60 s)` in `Suspended`, and they reset to `base` on any event.
* R2.6 Capability detection reports `psi`, `trigger_write`, `cgroup_v2`, `inotify` (the last becomes useful in child 4). Degraded sources emit the child-1 `SourceDegraded { source, message }` signal and are logged once; a typed `SourceHealth` is deferred to child 3 when per-cgroup scopes need it.
* R2.7 `/run/useless/journal.jsonl` receives transition-only JSON Lines, bounded by keeping the last half over ~256 KiB. `/run/useless/state` receives mode, reasons, budget, available memory, capabilities, and the last transition as line-based `key=value`, written on change and at most once per 30 s. The state file is deliberately not JSON so `status` needs no parser.
* R2.8 `useless status` prints the capabilities, current mode/reason, budget/headroom, and last transition by reading the state file. Existing status lines stay intact.
* R2.9 No behavior regression when PSI is unavailable: the sampler path reproduces child 1's behavior with `check_interval_secs`.

## Acceptance criteria

* [ ] AC2.1 `cargo fmt --check`, `cargo check`, `cargo test` pass.
* [ ] AC2.2 On a Linux host, `strace -f -e epoll_wait,poll` during steady state shows long blocking waits, not a spin loop; no `thread::sleep` remains in the supervisor loop.
* [ ] AC2.3 Writing a synthetic PSI trigger (or running a memory hog) suspends the daemon in under 100 ms, measured from the pressure onset.
* [ ] AC2.4 SIGTERM and SIGHUP are handled through `signalfd`; shutdown joins all workers within two seconds.
* [ ] AC2.5 With PSI disabled (simulated by a `PsiOps` fake returning `ENOENT`), the daemon runs on the sampler and `status` reports `psi=unavailable`.
* [ ] AC2.6 Journal entries are valid JSON per line, bounded, and never block or crash the daemon when the path is unwritable.
* [ ] AC2.7 Unit tests use pipes for reactor dispatch and a fake `PsiOps` for trigger registration, reopen-on-`EBUSY`, and `POLLERR` handling.

## Out of scope

* Per-cgroup pressure scopes (child 3).
* Memory ledger and systemd unit changes (child 3).
* inotify session sources (child 4).
* Network gating (child 5).

## Open questions

None.
