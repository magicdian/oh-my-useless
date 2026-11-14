# Design: PSI trigger one-shot re-arm and unit start limits

## 1. Why close and re-arm

The kernel on `oracle-sg-tx-forward` keeps a fired PSI trigger fd readable after the pressure ends. `read()` and `pread()` do not clear it, rewriting the threshold returns `EBUSY`, and a freshly opened trigger is immediately readable while the kernel still considers the pressure state active. A level-triggered epoll watch therefore spins.

Closing the fd removes it from epoll. Re-arming later with a new fd is the only reliable way to get a fresh trigger state. The cooldown bounds the work and matches the kernel's own notification window.

## 2. Pipeline

```
epoll wakes on PSI token
  └─ PsiSource::handle
       ├─ pread the trigger (current readings)
       ├─ close the watch (removes it from epoll)
       ├─ rearm_at[index] = now + pressure_window_ms
       └─ emit Signal::Pressure only when some/full avg10 > 0

daemon loop, every iteration
  └─ reactor.tick()
       └─ PsiSource::tick: for closed watches whose rearm_at <= now
            open + write trigger + epoll_add
```

While pressure persists, the re-armed trigger fires again; the daemon emits a pressure signal and re-schedules. Work is bounded by the sampler loop (5 s while pressure is present, 20 s while suspended). When pressure is gone, the re-armed trigger stays quiet; the reducer's `resume_quiet_secs` (90 s) expires and the daemon resumes.

Zero-reading events are dropped. They still close and re-arm the watch, so a sticky readable fd cannot spin or reset the quiet window.

## 3. Code changes

### `src/ports.rs`

```rust
pub trait EventSource {
    fn register(&mut self, epoll_fd: RawFd) -> AppResult<Vec<u64>>;
    fn handle(&mut self, token: u64, events: u32) -> AppResult<Vec<Signal>>;

    /// Periodic hook for sources that need to re-arm a one-shot fd.
    fn tick(&mut self) -> AppResult<()> {
        Ok(())
    }
}
```

### `src/runtime/reactor.rs`

```rust
pub fn tick(&mut self) -> AppResult<()> {
    for source in &mut self.sources {
        source.tick()?;
    }
    Ok(())
}
```

### `src/daemon.rs`

Call `reactor.tick()?;` immediately after `reactor.wait(wait_until)?;`. The re-armed fd is added to epoll but is not part of the current ready batch; the next wait observes it.

### `src/adapters/psi.rs`

Add fields:

```rust
cooldown: Duration,
rearm_at: Vec<Option<Instant>>,
```

`new` sets `cooldown = Duration::from_millis(window_ms.max(1))` and `rearm_at = vec![None; specs.len()]`.

`register` leaves `rearm_at` as `None` for watches that never opened; `tick` only re-arms watches that were explicitly scheduled.

`handle` for a pressure token:

```rust
if events & ERROR_FLAGS != 0 {
    self.close_watch(index);
    self.schedule_rearm(index);
    return Ok(Vec::new());
}
let Some(fd) = self.watches[index] else {
    return Ok(Vec::new());
};
let mut buffer = [0u8; 512];
let count = match self.ops.read(fd, &mut buffer) {
    Ok(count) => count,
    Err(_) => {
        self.close_watch(index);
        self.schedule_rearm(index);
        return Ok(Vec::new());
    }
};
self.close_watch(index);
self.schedule_rearm(index);
let text = String::from_utf8_lossy(&buffer[..count]);
let reading = parse_pressure(&text).unwrap_or_default();
if reading.some_avg10_x100 == 0 && reading.full_avg10_x100 == 0 {
    return Ok(Vec::new());
}
Ok(vec![Signal::Pressure { class, scope, reading }])
```

`schedule_rearm` sets `rearm_at[index] = Some(Instant::now() + self.cooldown)`. `close_watch` already sets `watches[index] = None` and closes the fd.

`tick`:

```rust
fn tick(&mut self) -> AppResult<()> {
    let Some(epoll_fd) = self.epoll_fd else {
        return Ok(());
    };
    let now = Instant::now();
    for index in 0..self.specs.len() {
        if self.watches[index].is_some() {
            continue;
        }
        let Some(at) = self.rearm_at[index] else {
            continue;
        };
        if now < at {
            continue;
        }
        self.rearm_at[index] = None;
        match self.open_watch(index) {
            Ok(fd) => match sys::epoll_add(epoll_fd, fd, self.tokens[index], WATCH_FLAGS) {
                Ok(()) => self.watches[index] = Some(fd),
                Err(_) => {
                    self.ops.close(fd);
                    self.schedule_rearm(index);
                }
            },
            Err(_) => self.schedule_rearm(index),
        }
    }
    Ok(())
}
```

`rebuild` is no longer used by the error path; remove it or make it call the same close/schedule logic. Keeping a separate rebuild that reopens immediately would reintroduce the spin, so the error path uses close + schedule.

### `src/install.rs`

`systemd_unit` moves `StartLimitIntervalSec=300` and `StartLimitBurst=5` from `[Service]` to `[Unit]`. The `[Service]` block keeps `Restart=on-failure` and `RestartSec=10`.

## 4. Test plan

Deterministic tests in `src/adapters/psi.rs` using `FakePsiOps` (eventfd-backed fd):

- `pressure_event_closes_the_watch`: after `handle`, `handle` again returns empty and `ops.opens` is unchanged; `tick` before the cooldown does not reopen; sleep past the cooldown; `tick` reopens (`opens` +1).
- `zero_reading_is_ignored`: fixture `some avg10=0.00 ...`; `handle` returns empty and closes; `tick` after cooldown reopens.
- `poll_error_schedules_a_rearm`: `handle(PSI_TOKEN_BASE, EPOLLERR)` returns empty, closes, and `tick` after cooldown reopens.
- Existing signal tests still assert the first event returns `Signal::Pressure`.

Use a `source_with_window(ops, window_ms)` helper; re-arm tests pass `window_ms = 10` and sleep ~20 ms. The existing `source()` keeps `window_ms = 1000` so the trigger-string assertion (`some 50000 1000000`) is unchanged.

Unit test in `src/install.rs`: split the generated unit at `[Service]` and assert both start-limit keys are in the `[Unit]` part and absent from the `[Service]` part.

## 5. Release and server validation

1. Land the fix on `dev`; run `cargo fmt/clippy/test` plus the Linux Rust 1.100 container suite.
2. Merge to `main`; the OMV pipeline publishes `2611.14.2`.
3. On `oracle-sg-tx-forward`: back up the old binary/unit, install `2611.14.2`, and verify:
   - `systemctl show` reports `StartLimitIntervalUSec=5min`.
   - idle CPU is near zero; `strace -c` does not show a `pread`/`epoll_wait` loop.
   - `dd`-generated IO pressure drives `mode=suspended`; after it stops, the daemon returns to `running`/`cooling`.
4. Validate self-update:
   - `useless update --check` reports `2611.14.2` and the latest release.
   - `sudo useless update --version 2611.14.2` re-downloads the pinned release, verifies the digest, replaces the binary, and restarts; the service stays on `2611.14.2`.
5. Keep the pre-fix backup until the new version is accepted.

## 6. Rejected alternatives

- **Edge-triggered epoll alone.** The fd stays readable while pressure persists; no new edge arrives, so the quiet window could expire while pressure continues. Re-arm is still needed.
- **Reopen immediately on every event.** A new trigger is immediately readable while the pressure state is active, which reproduces the spin.
- **Polling `/proc/pressure` only.** Removes the triggers but loses the 1 s reaction the event path provides. The re-arm fix keeps the event path.
- **Ignoring zero readings without closing.** The fd remains readable, so epoll still spins.
