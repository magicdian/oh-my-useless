# fix: PSI trigger busy loop and unit start limits

## Goal

Make the event-driven daemon safe on kernels where a fired PSI trigger file descriptor stays readable after it fires. Today that condition turns the reactor into a busy loop: `epoll_pwait` returns immediately, every iteration emits a pressure signal, the 90-second quiet window is reset forever, and the daemon stays suspended at ~100% CPU.

The fix closes and re-arms a trigger watch on a cooldown, ignores trigger events whose readings are all zero, and moves the unit's start-limit keys into `[Unit]` where systemd actually reads them. After a 2611.14.2 release, the change is installed on `oracle-sg-tx-forward` and the self-update path is validated there.

## Background

On `oracle-sg-tx-forward` (Ubuntu 22.04, aarch64, glibc 2.35, kernel 6.8.0-1067-oracle), release 2611.14.1 started correctly but sat in `suspended/io_pressure` forever and used ~114% CPU.

Measured evidence:

- `strace -c` over 3 seconds: ~15,000 `epoll_pwait`, ~15,000 `timerfd_settime`, ~150,000 `pread64`.
- Normal pressure files reported `avg10=0.00` for both host and `user.slice`.
- A controlled test armed `/proc/pressure/io` with `some 50000 1000000`, generated IO with `dd`, and confirmed the trigger fires (`poll` returns `POLLPRI`).
- After pressure stopped, both `read()` and `pread()` left the fd readable; rewriting the threshold returned `EBUSY`; closing and reopening produced a new trigger that was immediately readable while the pressure state was still active in the kernel.
- The daemon registers 10 trigger fds (5 host + 5 `user.slice`) and uses level-triggered epoll, so every readable fd re-enters the loop immediately.

A second defect surfaced in the same install: `StartLimitIntervalSec=300` is written under `[Service]`, and systemd logs `Unknown key name 'StartLimitIntervalSec' in section 'Service', ignoring`. `systemctl show` reports the default 10-second interval instead of 300 seconds.

The old manually built binary (2026-04-15, no `version` command) does not use PSI triggers and remains active as the rollback target.

## Requirements

**R1 — No busy loop.** A fired PSI trigger must not be polled again until a cooldown has elapsed. The source must close the watch after handling an event and re-arm it from the event loop.

**R2 — Sticky zero readings are ignored.** If a trigger event's parsed `PressureReading` has all `some`/`full` averages at zero, it must not produce a `Signal::Pressure`. The watch is still closed and re-armed; the daemon must be able to leave `suspended` when real pressure is gone.

**R3 — Re-arm mechanism.** Add a default no-op `tick` to the `EventSource` trait, a reactor-level `tick`, and call it once per daemon loop iteration. `PsiSource::tick` re-opens and re-adds watches whose cooldown has expired. No new thread and no new timerfd are introduced.

**R4 — Cooldown.** The cooldown equals `pressure_window_ms` (the configured tracking window, 1 s by default). Re-arming every sampler loop (5–20 s) bounds the work; a persisted pressure keeps emitting at most one event per cooldown.

**R5 — Error path.** A `POLLERR`/`POLLHUP` on a trigger also closes the watch and schedules a re-arm instead of reopening immediately.

**R6 — Unit placement.** `StartLimitIntervalSec` and `StartLimitBurst` move to `[Unit]`. `Restart=on-failure`, `RestartSec`, and the self-containment keys stay in `[Service]`.

**R7 — Tests.** Add deterministic tests for: a pressure event closes the watch; a second handle before the cooldown returns empty; tick before the cooldown does not reopen; tick after the cooldown reopens; an all-zero reading produces no signal; the generated unit places the start-limit keys in `[Unit]`.

**R8 — Release and validation.** Release 2611.14.2 through the existing OMV pipeline, install it on `oracle-sg-tx-forward` with backups, verify low CPU and correct suspend/resume around real IO pressure, then validate `useless update --check` and a pinned `useless update --version 2611.14.2`.

## Acceptance Criteria

- [ ] AC1 `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` pass; the Linux container suite (Rust 1.100) also passes.
- [ ] AC2 The new PSI tests cover close-on-event, cooldown gating, re-arm, zero-reading suppression, and the error path.
- [ ] AC3 The generated unit contains `StartLimitIntervalSec=300` and `StartLimitBurst=5` in `[Unit]`; `systemctl show` on the test host reports a 300 s interval.
- [ ] AC4 `oracle-sg-tx-forward` runs 2611.14.2 with CPU near idle while suspended, and does not spin under `dd`-generated IO pressure.
- [ ] AC5 With real IO pressure, the daemon enters `suspended`; after pressure stops and the quiet window elapses, it returns to `running`/`cooling` without manual intervention.
- [ ] AC6 `useless update --check` reports the installed version and latest release; `sudo useless update --version 2611.14.2` completes and the service restarts on the same version.
- [ ] AC7 The rollback path remains available and documented; the old binary/unit backup is preserved until the new release is accepted.

## Out of scope

- Redesigning the sensor layer or replacing PSI triggers with a different kernel interface.
- Changing the OMV version policy or release pipeline.
- Tuning this host's config beyond what the fix requires.
- macOS support for PSI (the code is Linux-only).

## Notes

- The sticky-readability behavior is kernel-specific; the fix must be safe on kernels where the trigger deasserts normally too. Closing and re-arming is valid on both.
- The daemon's reducer only consumes `class` and `scope` from `Signal::Pressure`; the reading values are used for the zero-reading filter.
- `useless update --version <same>` is an intentional pinned reinstall: `self_update` installs the pinned release without a version comparison, which exercises download, digest verification, replacement, and restart.
