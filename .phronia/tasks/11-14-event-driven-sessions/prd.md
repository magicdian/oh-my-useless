# feat: event-driven session detection

## Goal

Detect login and logout through inotify instead of polling `who`, and probe tty idle time only for sessions that actually exist. Keep the current `who`/`ps` parser as a fallback adapter, and make resume wait for a quiet window instead of jumping back to full load after a fixed grace period.

## Background

* `src/sensor.rs:127-145` shells out to `who -u --ips` and `ps -eo args` every sample.
* `src/sensor.rs:233-253` classifies a session as active only when its idle time is within `active_tty_idle_secs`.
* `src/policy.rs:73-82` treats "no active session for `interactive_grace_secs`" as permission to run at full duty.
* On the incident host, `active_tty_idle_secs = 60` and `interactive_grace_secs = 30`: a human reading logs for one minute stopped counting as present.

Child 2 provides the reactor and the `EventSource` contract.

## Requirements

* R4.1 `src/adapters/sessions.rs` implements `SessionSource` with two backends: inotify on `/run/systemd/sessions` (preferred on systemd hosts), and inotify on `/var/run/utmp` (fallback). Both emit `Signal::Sessions(SessionSet)` on change.
* R4.2 The `utmp` watch handles `IN_MODIFY`, `IN_CLOSE_WRITE`, `IN_ATTRIB`, `IN_DELETE_SELF`, and `IN_MOVE_SELF`; on self-delete or self-move it re-adds the watch instead of silently degrading.
* R4.3 Session identity is the tty name (`pts/N` or `ttyN`), matching the existing deduplication semantics. A single `sudo -s` must not inflate the count.
* R4.4 Idle time comes from `stat("/dev/pts/N").st_atime`; a `TtyIdleProbe` runs only while at least one session exists, at an adaptive cadence (5 s while any session is within the idle threshold, 30 s otherwise). No `who` process is spawned in the normal path.
* R4.5 If inotify is unavailable or both watches fail, the existing `who -u --ips` + `ps` sampler becomes the active source at `check_interval_secs`; behavior must match today's parser and its tests.
* R4.6 Resume after the last logout requires `interactive_grace_secs` plus `resume_quiet_secs` of clean signals and a clean probe, matching the parent state machine. The 25/50/100% slow start applies.
* R4.7 Non-TTY work is not inferred from sessions. It is detected by PSI and headroom; this child does not add process scanning.
* R4.8 The reason reported on suspension distinguishes `interactive_session` from `ambient_pressure`, and `status` shows the session count and the idle source in use.

## Acceptance criteria

* [ ] AC4.1 `cargo fmt --check`, `cargo check`, `cargo test` pass; the six existing sensor parsing tests move with the fallback adapter and still pass.
* [ ] AC4.2 An inotify event on the session directory produces a `SessionSet` signal without spawning `who`; `strace -f -e execve` shows no `who`/`ps` in the normal path.
* [ ] AC4.3 SSH login suspends within one reactor wake-up; logout resumes only after grace plus quiet window; the transition is journaled with `interactive_session`.
* [ ] AC4.4 A `sudo -s` nested shell does not increase the active count; a second distinct `pts/*` does.
* [ ] AC4.5 A stale `pts/*` record whose atime is older than `active_tty_idle_secs` is excluded; when a session exists, the idle probe runs at the adaptive cadence and stops when the set becomes empty.
* [ ] AC4.6 With inotify disabled (`INotifyOps` fake returning `ENOSYS`), the fallback sampler produces the same session counts as the pre-change code for the fixture table.
* [ ] AC4.7 `IN_DELETE_SELF`/`IN_MOVE_SELF` on the watched path re-adds the watch; a test asserts the re-add call.

## Out of scope

* PAM integration, eBPF session tracing, non-TTY SSH command detection.
* Changing `active_tty_idle_secs` or `interactive_grace_secs` defaults.
* Accounting for every distro's utmp format.

## Open questions

None. The `who` parser already covers distro variance; it remains the fallback.
