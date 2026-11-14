# Design: reactor and push-based host pressure

Shared architecture lives in `../11-14-event-driven-sensor-architecture/design.md`. This file records the reactor, syscall, and PSI details.

## 1. Syscall layer (`src/sys.rs`)

Use `libc` types and constants directly; never re-declare kernel structs by hand.

```rust
pub struct Fd(RawFd);                       // close on drop
pub fn epoll_create() -> io::Result<Fd>;
pub fn epoll_ctl_add(ep: RawFd, fd: RawFd, token: u64, events: u32) -> io::Result<()>;
pub fn epoll_wait(ep: RawFd, out: &mut [libc::epoll_event], timeout_ms: i32) -> io::Result<usize>;
pub fn timerfd_create() -> io::Result<Fd>;
pub fn timerfd_arm(fd: RawFd, deadline: Instant) -> io::Result<()>;
pub fn signalfd_create(signals: &[i32]) -> io::Result<Fd>;
pub fn set_nonblocking(fd: RawFd) -> io::Result<()>;
pub fn pread_exact(fd: RawFd, buf: &mut [u8], offset: u64) -> io::Result<usize>;
```

`epoll_event` is registered with `EPOLLIN | EPOLLPRI | EPOLLERR`; `EPOLLPRI` is required for PSI triggers. All fds are non-blocking; each drain loops until `EAGAIN` with a hard iteration cap (8) so one source cannot starve the reactor.

## 2. Reactor (`src/runtime/reactor.rs`)

```rust
pub trait EventSource {
    fn register(&mut self, ep: RawFd) -> Result<()>;
    fn handle(&mut self, token: u64) -> Result<Vec<Signal>>;
    fn next_deadline(&self) -> Option<Instant> { None }
    fn on_deadline(&mut self, now: Instant) -> Result<Vec<Signal>> { Ok(Vec::new()) }
}
```

`Reactor` owns the epoll fd, the timerfd, a `Vec<Box<dyn EventSource>>`, and a token→source map. Loop:

1. Collect the earliest `next_deadline` across sources; arm the timerfd with it; compute the `epoll_wait` timeout.
2. `epoll_wait`.
3. For each ready event, dispatch to the owning source and append its signals.
4. If the timerfd fired, call `on_deadline` on every source whose deadline passed.
5. Return the batch to the composition root, which calls `reduce` once per batch.

A `SignalfdSource` maps `SIGHUP` to `Signal::ConfigReloaded` (the root reloads config and re-publishes) and `SIGINT`/`SIGTERM` to a shutdown flag. The old raw `signal()` handlers and `sleep_with_interrupts` are deleted.

## 3. PSI adapter (`src/adapters/psi.rs`)

```rust
pub trait PsiOps {
    fn open_rw(&self, path: &Path) -> io::Result<RawFd>;
    fn write_trigger(&self, fd: RawFd, spec: &str) -> io::Result<()>;
    fn read_pressure(&self, fd: RawFd) -> io::Result<PsiFile>;
    fn close(&self, fd: RawFd);
}
```

* One `PsiWatch` per `(class, scope, kind)` where `kind ∈ {some, full}`. Child 2 uses only the host scope and `some`; `full` is registered for memory and IO because the design uses it in the reducer.
* Register by writing `"<kind> <enter_us> <window_us>"`. `enter_us = pressure_enter_ms × 1000`.
* On `POLLPRI`: `pread(fd, 0, ..)` the file, parse both `some` and `full` lines, emit `Signal::Pressure`. The trigger only reports entry; do not wait for an exit event. Exit is decided by the quiet timer plus the next fallback sample (`MemAvailable` and load), which is the positive probe the state machine needs.
* On `POLLERR`: close the fd, reopen, re-register; if that fails twice, emit `SourceDegraded { source, message }` and stop watching this metric.
* Threshold changes require `close` + `open` + `write`; a second write on an armed fd returns `EBUSY` (identified by `errno`).
* Capability probe: open `/proc/pressure/memory` `O_RDWR`, write a trigger, then immediately close. Failure marks `trigger_write=false` and the daemon runs sampler-only.

`PsiFile` parses:
```text
some avg10=0.00 avg60=0.00 avg300=0.00 total=0
full avg10=0.00 avg60=0.00 avg300=0.00 total=0
```
Lines are independent; a missing `full` line is normal for `cpu` at host scope.

## 4. Adaptive sampler

`domain::cadence(mode, base_secs) -> Duration`:

| Mode | Cadence |
|---|---|
| `Running` | `base_secs` |
| `Cooling` | `2 × base_secs` |
| `Suspended` | `min(4 × base_secs, 60 s)` |

`SamplerSource` holds the next deadline and emits `Signal::Sample` from `MemInfoSampler` + load + swap + net counters (net counters are consumed in child 5; sampling them here is harmless and keeps one code path). Any `Signal` handled in the same reactor batch resets the next deadline to `now + cadence(Running)`, so events always restore fast observation.

## 5. Journal and state snapshot

`runtime::journal::FileSink` writes to `/run/useless/journal.jsonl`:

```json
{"ts_ms":1763128800123,"signal":"psi:memory","from":"running","to":"suspended","reasons":["memory_pressure"],"budget_mb":0,"available_mb":612}
```

* Append on transition only. When the file exceeds 256 KiB, keep the most recent `capacity/2` entries and rewrite atomically via a temp file + rename.
* A write failure disables the sink for the process lifetime, logs once, and leaves the in-memory ring intact.

`/run/useless/state` is written atomically on transition and at most once per 30 s. It is line-based `key=value` on purpose: `status` must not need a JSON parser in a std-only binary.

```text
mode=suspended
reasons=memory_pressure
budget_mb=0
available_mb=612
capabilities=psi=ok trigger_write=ok cgroup_v2=ok inotify=ok
last_transition=running -> suspended (psi:memory)
summary=pressure=Pause load=12 mem_avail=31 ...
```

Values are single-line. `summary` may contain `=`; parse on the first separator.

## 6. Status

`install.rs::status` keeps existing lines and appends:

```text
sensors: psi=ok trigger_write=ok cgroup_v2=ok inotify=ok
mode: suspended (memory_pressure)
budget: 0 MB / available 612 MB
last_transition: 2026-11-14T14:00:00.123Z
```

If `/run/useless/state` is missing (daemon not running, or older binary), print `daemon_state: unavailable` and do not fail.

## 7. Failure handling

* `epoll_wait` `EINTR` → continue.
* `epoll_wait` other error → rebuild the epoll set once; on a second failure, emit `CollectorDegraded` and run the sampler loop with a 1 s `poll` timeout (still event-first, but self-healing).
* A source that panics is not tolerated: adapters return `Result`, and the reactor converts an error into `SourceDegraded { source, message }` plus a dropped source. No `unwrap` on IO.
* `MemAvailable` unreadable → `Signal::Sample` is not emitted; the reducer keeps the last state and the daemon logs once. The existing fail-closed behavior for a failed sensor is preserved in the root.

## 8. Tests

* Reactor: two pipes, one timerfd; assert dispatch order, deadline firing, and that a silent pipe does not spin.
* PSI: fixture strings for both lines, malformed lines, `EBUSY` reopen path, `POLLERR` rebuild, capability probe failure.
* Journal: overflow halving, unwritable path, atomic state write (temp + rename).
* Cadence: table test for the three modes and base values including `0` clamping to 1 s.

## 9. Platform split

All Linux-only code is `cfg(target_os = "linux")`. On non-Linux:

* `Reactor::wait` sleeps until the sampler deadline and returns no events.
* `SignalSource` and `PsiSource` are no-ops; the daemon installs raw signal handlers.
* PSI parsing and capability types still compile so their tests run.

This exists so `cargo check` and `cargo test` work on the macOS development host. The production target is Linux, and `cargo check --tests --target x86_64-unknown-linux-gnu` type-checks the gated code.

## 10. Rollback

Revert the child commit; the sampler path from child 1 remains. Remove `libc` from `Cargo.toml` only if no other child depends on it (children 3-5 do, so keep it).
