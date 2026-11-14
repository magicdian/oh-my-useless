# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`
- `.phronia/spec/backend/quality-guidelines.md`

## Intended changes

### `daemon-install-contracts.md`

- Under **Runtime behavior**, replace the polling-only session description with the source order:
  - Preferred: inotify on `/run/systemd/sessions` (systemd/logind hosts); each directory event rescans the session files, skips `.ref`, and ignores `ACTIVE=0`/`STATE=closing`.
  - Fallback: inotify on `/var/run/utmp`; on `IN_DELETE_SELF`/`IN_MOVE_SELF`/`IN_IGNORED` the watch is re-added.
  - Last resort: the `who -u --ips` + `ps -eo args` sampler at `check_interval_secs`.
- The normal inotify path must not spawn `who`/`ps`; the sampler is the fallback only.
- Session identity remains the tty name (`pts/N` or `ttyN`); duplicate rows for the same tty count once.
- A tty atime probe runs only while sessions exist: 5 s while any session is within `active_tty_idle_secs`, 30 s otherwise, and it re-emits a `SessionSet` filtered to sessions within the threshold. A failed `stat` is treated as active (0 idle seconds).
- The state file's `capabilities` line includes `sessions=inotify|who` so `status` shows which session source is active.
- Resume after the last session still requires `interactive_grace_secs` plus `resume_quiet_secs` and the slow-start ramp.

### `directory-structure.md`

- Add `src/adapters/sessions.rs` (inotify session source, tty idle probe, legacy sampler).
- Note that `ProcfsSampler` emits only `Signal::Sample`; session signals come from `adapters/sessions.rs`.

### `quality-guidelines.md`

- Add session tests to the deterministic-test examples: logind directory scan, variable-length inotify event parsing, probe filtering, and tty atime.
- Linux-only session tests are `cfg(target_os = "linux")`; run the Linux cross-target check when developing on another host.

## Rationale

Child 4 replaces the per-sample `who`/`ps` shell-out with event-driven session changes and bounds idle probing to the sessions that actually exist. The specs must describe the source order and fallback so future changes do not reintroduce polling as the primary path.

## Verification

- After applying: `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo check --tests --target x86_64-unknown-linux-gnu` pass.
- `sudo strace -f -e trace=execve -p "$(cat /run/useless.pid)" -c` shows no `who`/`ps` on a systemd host while sessions are tracked through inotify.
- `useless status` shows `sessions=inotify` on a logind host and `sessions=who` when the fallback is active.
