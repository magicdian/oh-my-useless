# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`

## Intended changes

### `daemon-install-contracts.md`

- Under **Runtime behavior**, add:
  - A PSI trigger is one-shot per event. After handling a trigger, the source closes the watch (which removes it from epoll) and re-arms a fresh trigger after `pressure_window_ms`.
  - `EventSource::tick` (called by the reactor once per daemon loop iteration) performs the re-arm. This is required because a fired trigger fd can stay readable after the pressure ends on some kernels; level-triggered polling would spin and reset the quiet window forever.
  - Trigger events whose `some_avg10_x100` and `full_avg10_x100` are both zero are ignored. They still close and schedule a re-arm; they must not emit `Signal::Pressure` or reset `resume_quiet_secs`.
  - `POLLERR`/`POLLHUP` and read failures close the watch and schedule a re-arm instead of reopening immediately.
- Under **Install behavior**, add:
  - The unit places `StartLimitIntervalSec=300` and `StartLimitBurst=5` in `[Unit]`, not `[Service]`; systemd ignores them in `[Service]`. `Restart=on-failure` and `RestartSec=10` stay in `[Service]`.
- Add validation rows:
  - A fired trigger that stays readable must not cause an `epoll_wait`/`pread` busy loop.
  - A zero-reading trigger event must not produce a pressure signal.
  - A trigger is re-armed only after the cooldown; before that, `handle` returns empty.
- Add unit-test expectations: pressure event closes the watch, `tick` before the cooldown does not reopen, `tick` after the cooldown reopens, zero readings are ignored, and the error path schedules a re-arm.
- Add wrong-case entries: level-triggered polling of a perpetually readable PSI trigger; reopening a trigger immediately on every event; placing start-limit keys in `[Service]`.
- Add correct-case entries: close + cooldown re-arm; filter zero readings; verify start-limit placement by splitting the generated unit at `[Service]`.

### `directory-structure.md`

- Note that `src/ports.rs` `EventSource` has an optional `tick` hook for sources that re-arm one-shot descriptors, and `src/runtime/reactor.rs` exposes `Reactor::tick` which the daemon calls once per loop iteration.
- Note that `src/adapters/psi.rs` owns the close/cooldown/re-arm state for PSI triggers.

## Rationale

Release 2611.14.1 spun at ~114% CPU and stayed permanently suspended on `oracle-sg-tx-forward`: the kernel kept fired PSI trigger fds readable, level-triggered epoll returned immediately, and every event reset the quiet window. The fix is a behavioral contract, not an implementation detail: fired triggers are consumed once and re-armed, zero readings are not pressure, and start-limit keys must live in `[Unit]` to take effect.

## Verification

- After applying: `grep -n "re-arm\|StartLimitIntervalSec" .phronia/spec/backend/daemon-install-contracts.md` returns the new contract lines.
- Linux container: `cargo clippy --all-targets --locked -- -D warnings` and `cargo test --all-targets --locked` pass (96 tests), including the new PSI close/re-arm/zero/error tests.
- On the release host after 2611.14.2: idle CPU near zero, no `pread`/`epoll_wait` spin, and suspended → running after IO pressure stops.
- `systemctl show useless -p StartLimitIntervalUSec` reports `5min`.
