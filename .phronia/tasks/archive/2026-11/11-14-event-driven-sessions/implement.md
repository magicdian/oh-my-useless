# Implementation plan: event-driven session detection

## Checklist

1. [ ] Keep `SessionSet`/`Session` from child 1; the reducer's interactive-grace rule is already in place.
2. [ ] Add `inotify_create`, `inotify_add_watch`, and `read_inotify` to `src/sys.rs` with Linux stubs.
3. [ ] Implement `SessionSource` with the logind directory backend and `.ref` filtering.
4. [ ] Add the utmp backend to the same source, including self-delete/move re-add.
5. [ ] Implement `SessionState::probe` with the 5 s/30 s cadence and fail-safe stat handling; it re-emits a filtered `SessionSet`.
6. [ ] Add `LegacySessionSampler`; stop `ProcfsSampler` from emitting `Signal::Sessions`.
7. [ ] Wire `SessionRuntime` selection (logind → utmp → legacy sampler) and publish `sessions=inotify|who` in the capabilities line.
8. [ ] Extend the state file/capabilities with the active session source; the summary already carries the session count.
9. [ ] Add tests: logind directory scan, variable-length inotify event parsing, probe filtering, and tty atime; Linux-only tests are `cfg(target_os = "linux")`.
10. [ ] Record the source order, probe cadence, and capability fields in `spec-delta.md`; do not edit `.phronia/spec/**`. README updates belong to child 5.

## Validation

```bash
cargo fmt --check
cargo check
cargo test
cargo check --tests --target x86_64-unknown-linux-gnu

# normal path must not spawn who/ps
sudo strace -f -e trace=execve -p "$(cat /run/useless.pid)" -c

# session state while logged in
useless status
tail -n 10 /run/useless/journal.jsonl
```

Manual SSH test:

1. Log in over SSH; `useless status` and the journal show `interactive_session` within one second.
2. Leave the session idle for longer than `active_tty_idle_secs`; the daemon may resume, and the reason changes.
3. Run `sudo -s`; the session count does not double.
4. Log out; verify resume only after `interactive_grace_secs + resume_quiet_secs`.
5. Open two distinct SSH sessions; the count is two, and both must close before resume.

## Risky files

* `src/adapters/sessions.rs` — parses operator-visible state; keep parsing defensive and log once.
* `src/sys.rs` — inotify event buffer handling; `inotify_event` has a variable-length name.
* `src/domain/reducer.rs` — grace/quiet interaction; keep the parity fixture from child 1 green.

## Rollback point

One commit. Revert restores the timer sampler; no config or deployment change.
