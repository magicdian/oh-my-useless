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
}
```

`user`/`since` are not needed by the counts or the summary and are deferred until a consumer needs them.

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

Rules: skip `.ref`; skip entries without `TTY=`; `remote = REMOTE=1` or `TTY=pts/*` with a remote address. A parse failure is a health note, not a fatal error. A create event can race the session-file write, so an empty rescan is retried once after 50 ms; the `.ref` creation usually supplies a second event.

### 2.2 utmp (fallback)

Watch `/var/run/utmp` (symlink-resolved) with `IN_MODIFY | IN_CLOSE_WRITE | IN_ATTRIB | IN_DELETE_SELF | IN_MOVE_SELF`. On a change, run the existing `who`-based sampler once and publish the resulting set. On self-delete or self-move, re-add the watch; a failed re-add logs and the source stays active. A dynamic switch to the legacy sampler after repeated failures is deferred because the reactor owns the source.

The utmp binary is not parsed directly. `who` already normalizes distro differences, and the event rate is a login/logout, not a heartbeat.

### 2.3 Timer sampler (last resort)

`LegacySessionSampler` calls `procfs::read_sessions` (`who -u --ips` + `ps -eo args`) at `check_interval_secs`; `ProcfsSampler` no longer emits `Signal::Sessions` itself. This is the pre-change parser and its tests.

Selection order: logind directory → utmp watch → timer sampler. A `SensorCapabilities.inotify` flag and `status` report which one is active.

## 3. Idle probing

```rust
pub struct SessionState {
    tracked: SessionSet,
    active: SessionSet,
    active_tty_idle_secs: u64,
    next_probe: Option<Instant>,
}

impl SessionState {
    pub fn probe(&mut self, now: Instant) -> Option<SessionSet>;
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn active(&self) -> &SessionSet;
}
```

The inotify source and the daemon share `SessionState` through an `Arc<Mutex<_>>`; the daemon calls `probe` on the monitor handle. `SessionState` keeps two sets: `tracked` (the raw login/session set) and `active` (sessions within `active_tty_idle_secs`).

* For each tracked session, `stat("/dev/pts/N")` and use `st_atime`. This is the same source `who -u` uses.
* Cadence: 5 s while any session is active; 30 s once every session looks idle. When the tracked set is empty the probe sleeps for 30 s and performs no stats.
* `stat` failure is treated as active (return 0 idle seconds), which is fail safe for the human.
* The probe re-emits a non-empty `active` set on every tick so the reducer keeps refreshing `last_interactive`; an empty set is emitted only when it changes (logout). A tty that goes idle is removed from `active` but stays in `tracked`, so the next probe can re-activate it without a new login event.

This is the only intentional polling in the session layer, and it is bounded by the number of real sessions (normally 0-3).

## 4. Reducer integration

* `Signal::Sessions(set)` replaces the set; the reducer computes `tty_sessions`, `ssh_sessions`, and `interactive_active`.
* A non-empty session set keeps `interactive_active` true regardless of the grace timer; the grace timer only governs resume after logout.
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
| Session file mid-write | Skip the entry; the source rescans once after 50 ms and the `.ref` event supplies a second chance. |
| tty idle then active again | Stays in `tracked`; the next probe re-activates it without a new login event. |
| `pts/N` reused after logout | New utmp/session entry; the dedup key is rebuilt from the fresh set. |
| Detached tmux/screen | Not a login session; real activity is caught by PSI, not by this adapter. Documented. |

## 7. Rollback

Revert the child commit; the timer sampler from child 1 remains, and behavior returns to the pre-change session model. No config change is required.
