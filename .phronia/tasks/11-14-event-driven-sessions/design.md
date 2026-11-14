# Design: event-driven session detection

Shared architecture lives in `../11-14-event-driven-sensor-architecture/design.md`. This file records the session model, inotify handling, and idle probing.

## 1. Session model

```rust
pub struct SessionSet {
    pub sessions: Vec<Session>,
}

pub struct Session {
    pub tty: TtyId,          // "pts/0", "tty1"
    pub remote: bool,        // remote pts, used for the ssh counter
    pub user: Option<String>,
    pub since: Option<SystemTime>,
}
```

Identity is the tty string. Counting is `dedup_by(tty)`, which preserves the current `sudo -s` behavior: a nested shell on the same `pts/N` does not add a session.

## 2. Sources

### 2.1 logind directory (preferred)

Watch `/run/systemd/sessions` with `IN_CREATE | IN_DELETE | IN_MOVED_TO | IN_MOVED_FROM | IN_ATTRIB`. On any event, rescan the directory and rebuild the `SessionSet`. The directory has one small file per session (plus `.ref` entries), so a rescan is cheaper and far less error-prone than per-file watches.

Session files are keyed lines:

```text
UID=1000
USER=ops
ACTIVE=1
STATE=active
REMOTE=1
TTY=pts/0
TYPE=tty
SCOPE=session-3.scope
```

Rules: skip `.ref`; skip entries without `TTY=` (the file may be mid-write; the next event or the idle probe refreshes it); `remote = REMOTE=1` or `TTY=pts/*` with a remote address. A parse failure is a health note, not a fatal error.

### 2.2 utmp (fallback)

Watch `/var/run/utmp` (symlink-resolved) with `IN_MODIFY | IN_CLOSE_WRITE | IN_ATTRIB | IN_DELETE_SELF | IN_MOVE_SELF`. On a change, run the existing `who`-based sampler once and publish the resulting set. On self-delete or self-move, re-add the watch; if that fails twice, emit `SourceHealth::Degraded` and switch to the timer sampler.

The utmp binary is not parsed directly. `who` already normalizes distro differences, and the event rate is a login/logout, not a heartbeat.

### 2.3 Timer sampler (last resort)

Reuse the child 1 `adapters/procfs.rs` session path (`who -u --ips` + `ps -eo args`) at `check_interval_secs`. This is the pre-change behavior and its six tests.

Selection order: logind directory → utmp watch → timer sampler. A `SensorCapabilities.inotify` flag and `status` report which one is active.

## 3. Idle probing

```rust
pub struct TtyIdleProbe { sessions: Vec<(TtyId, PathBuf)>, next_due: Instant }

pub fn probe(&mut self, now: Instant, active_tty_idle_secs: u64) -> Vec<Signal>;
```

* For each session, `stat("/dev/pts/N")` and use `st_atime`. This is the same source `who -u` uses.
* Cadence: 5 s while any session is within `active_tty_idle_secs`; 30 s once every session looks idle. The probe stops entirely when the set is empty.
* `stat` failure: treat the session as active for one cycle (fail safe for the human), then exclude it and log once. This avoids a permanently elevated state if a tty path disappears.
* The probe emits `SessionIdle { tty, idle }`; the reducer marks the session inactive when `idle > active_tty_idle_secs`.

This is the only intentional polling in the session layer, and it is bounded by the number of real sessions (normally 0-3).

## 4. Reducer integration

* `Signal::Sessions(set)` replaces the set; the reducer computes `tty_sessions`, `ssh_sessions`, and `interactive_active`.
* Any session event or an active idle reading updates `last_interactive`.
* `Suspended` on `interactive_session` is immediate.
* Resume requires `now - last_interactive > interactive_grace_secs` **and** `now - last_pressure > resume_quiet_secs` **and** a clean probe.
* The slow start from the parent design applies after resume.

The existing policy test (`pauses_on_interactive_sessions`) moves to the reducer and keeps its assertion.

## 5. Testability

* `InotifyOps` trait: `add_watch`, `read_events`, `rm_watch`, `close`. A fake produces scripted events, including `IN_DELETE_SELF`/`IN_MOVE_SELF`.
* Directory rescans run against a temporary directory with synthetic session files.
* Idle probing runs against a temporary file whose atime is set with `utimensat` or `File::set_times`.
* utmp path: feed the existing `who` fixture strings through the sampler; no binary utmp fixture is needed.

## 6. Failure and edge cases

| Case | Behavior |
|---|---|
| `/run/systemd/sessions` absent (non-systemd) | Skip to utmp; `status` reports the source. |
| Both inotify paths unwatchable | Timer sampler; one log line, then quiet. |
| Session file mid-write | Skip the entry; the next event or probe refreshes. |
| `pts/N` reused after logout | New utmp/session entry; the dedup key is rebuilt from the fresh set. |
| Detached tmux/screen | Not a login session; real activity is caught by PSI, not by this adapter. Documented. |

## 7. Rollback

Revert the child commit; the timer sampler from child 1 remains, and behavior returns to the pre-change session model. No config change is required.
