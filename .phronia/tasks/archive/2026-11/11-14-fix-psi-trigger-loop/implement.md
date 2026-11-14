# Implementation plan: PSI trigger re-arm and unit start limits

## Step 1 — Re-arm plumbing

- [ ] `src/ports.rs`: add `fn tick(&mut self) -> AppResult<()> { Ok(()) }` to `EventSource`.
- [ ] `src/runtime/reactor.rs`: add `pub fn tick(&mut self) -> AppResult<()>` that calls every source's `tick`.
- [ ] `src/daemon.rs`: call `reactor.tick()?;` immediately after `reactor.wait(wait_until)?;`.
- [ ] Validation: `cargo check --all-targets --locked`.

Review gate: no behavior change for sources that do not implement `tick`.

## Step 2 — PsiSource one-shot behavior

- [ ] Add `cooldown: Duration` and `rearm_at: Vec<Option<Instant>>` to `PsiSource`.
- [ ] `new`: `cooldown = Duration::from_millis(window_ms.max(1))`; initialize `rearm_at`.
- [ ] Add `schedule_rearm(index)`.
- [ ] `handle`:
  - Error/HUP: close the watch, schedule re-arm, return empty.
  - Read error: close, schedule, return empty.
  - Successful read: close and schedule before parsing.
  - Parse; if `some_avg10_x100 == 0 && full_avg10_x100 == 0`, return empty.
  - Otherwise return `Signal::Pressure`.
- [ ] Remove or rewrite `rebuild` so no path reopens immediately.
- [ ] Implement `EventSource::tick` for `PsiSource` using `epoll_fd` and `open_watch` + `epoll_add`; on failure, reschedule.
- [ ] Validation: `cargo check --all-targets --locked`.

Review gate: no `open_watch` call from `handle`/`rebuild`.

## Step 3 — Unit start-limit placement

- [ ] `src/install.rs`: move `StartLimitIntervalSec=300` and `StartLimitBurst=5` into `[Unit]`; leave `Restart=on-failure` and `RestartSec=10` in `[Service]`.
- [ ] Update `derives_self_containment_limits` to assert the placement by splitting at `[Service]`.
- [ ] Validation: `cargo test install::`.

## Step 4 — Tests

- [ ] Add a `source_with_window(ops, window_ms)` helper; keep `source()` at 1000 ms so the `some 50000 1000000` assertion stays valid.
- [ ] Add:
  - `pressure_event_closes_and_rearms_after_cooldown`: first `handle` returns a pressure signal; second returns empty; `tick` before cooldown does not increase `opens`; sleep past cooldown; `tick` increases `opens`.
  - `zero_reading_is_ignored`: fixture `avg10=0.00`; `handle` returns empty, closes; `tick` after cooldown reopens.
  - `poll_error_schedules_a_rearm`: `handle(..., EPOLLERR)` returns empty; `tick` after cooldown reopens.
- [ ] Update `poll_error_rebuilds_the_watch` (rename/replace) to the cooldown behavior.
- [ ] Validation: `cargo test adapters::psi::linux_tests`.

## Step 5 — Full verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] Linux Rust 1.100 container:
  ```bash
  docker run --rm -v "$PWD":/work -w /work -e CARGO_TARGET_DIR=/tmp/target rust:1.100 \
    bash -c 'rustup component add clippy >/dev/null && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked'
  ```
- [ ] `omv sync --check --json`; `./scripts/package.sh`.

Review gate: 94+ Linux tests pass; no clippy warnings.

## Step 6 — Release and server validation

- [ ] Commit, integrate to `dev`, spec sync, archive, push `dev`.
- [ ] PR `dev` → `main`; wait for the Release workflow to publish `2611.14.2`.
- [ ] On `oracle-sg-tx-forward`:
  - Back up binary/unit/config to `/root/useless-backup-<timestamp>`.
  - Install `2611.14.2` via the release archive; restart.
  - Verify `StartLimitIntervalUSec=5min`, `OOMScoreAdjust=1000`, `MemoryHigh/Max`, `IOSchedulingClass=3`.
  - Measure CPU over 8 s while suspended (must be near zero; no thousands of `pread`/`epoll_wait` via `strace -c`).
  - Generate IO pressure with `dd`; confirm `mode=suspended`; stop; wait past `resume_quiet_secs`; confirm `running`/`cooling`.
  - `useless update --check` → installed 2611.14.2, latest 2611.14.2.
  - `sudo useless update --version 2611.14.2` → download, digest verify, replace, restart; `useless version` still 2611.14.2.
- [ ] If anything fails, restore the backup and report.

## Rollback

- Code: revert the task commits; the daemon behavior returns to the prior release.
- Server: restore `/root/useless-backup-*/useless`, `useless.service`; `systemctl daemon-reload`; `systemctl restart useless`.
- Release: OMV rolls forward; do not rewrite `.omv/state.toml`.
