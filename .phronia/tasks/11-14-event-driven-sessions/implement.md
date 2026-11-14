# Implementation plan: event-driven session detection

## Checklist

1. [ ] Add `SessionSet`/`Session`/`TtyId` to `domain`; move the interactive-grace rule and the existing policy test into the reducer.
2. [ ] Add `inotify_init1`, `inotify_add_watch`, and `inotify_rm_watch` to `src/sys.rs`; define an `InotifyOps` trait for fakes.
3. [ ] Implement `LogindSessionSource` with directory rescan and `.ref` filtering.
4. [ ] Implement `UtmpSessionSource` with modify/self-delete handling and watch re-add.
5. [ ] Implement `TtyIdleProbe` with the adaptive cadence and fail-safe rules.
6. [ ] Move the existing `who`/`ps` parser into `LegacySessionSampler`; keep all six tests.
7. [ ] Wire source selection (logind → utmp → timer) and publish `SensorCapabilities.inotify`.
8. [ ] Extend `status` with the active session source, session count, and last session transition.
9. [ ] Add tests: fake inotify events, temp-directory rescan, atime probe, re-add, fallback parity.
10. [ ] Update README's interaction-tuning section and `.phronia/spec/backend/daemon-install-contracts.md` with the new source order and quiet-window semantics.

## Validation

```bash
cargo fmt --check
cargo check
cargo test

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
