use std::time::Instant;

use crate::domain::signal::{SessionSet, Signal};
use crate::ports::EventSource;

pub struct LegacySessionSampler {
    active_tty_idle_secs: u64,
}

impl LegacySessionSampler {
    pub fn new(active_tty_idle_secs: u64) -> Self {
        Self {
            active_tty_idle_secs,
        }
    }

    pub fn sample(&mut self) -> Signal {
        Signal::Sessions(crate::adapters::procfs::read_sessions(
            self.active_tty_idle_secs,
        ))
    }
}

/// One session source is active: inotify when available, otherwise the
/// `who`/`ps` sampler that already existed.
#[allow(dead_code)]
pub enum SessionRuntime {
    Event {
        source: Box<dyn EventSource>,
        monitor: SessionMonitor,
    },
    Legacy(LegacySessionSampler),
}

pub fn start(_token: u64, active_tty_idle_secs: u64) -> SessionRuntime {
    #[cfg(target_os = "linux")]
    {
        if let Some((source, monitor)) = linux::start(_token, active_tty_idle_secs) {
            return SessionRuntime::Event {
                source: Box::new(source),
                monitor,
            };
        }
    }

    SessionRuntime::Legacy(LegacySessionSampler::new(active_tty_idle_secs))
}

/// Shared handle for the tty idle probe. It is empty on non-Linux hosts,
/// where the legacy sampler is the only session source.
pub struct SessionMonitor {
    #[cfg(target_os = "linux")]
    state: std::sync::Arc<std::sync::Mutex<linux::SessionState>>,
}

impl SessionMonitor {
    pub fn initial(&self) -> Option<Signal> {
        #[cfg(target_os = "linux")]
        {
            self.state.lock().ok()?.take_initial()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    pub fn probe(&self, now: Instant) -> Option<Signal> {
        #[cfg(target_os = "linux")]
        {
            self.state.lock().ok()?.probe(now).map(Signal::Sessions)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = now;
            None
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        #[cfg(target_os = "linux")]
        {
            self.state.lock().ok()?.next_deadline()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> SessionSet {
        #[cfg(target_os = "linux")]
        {
            self.state
                .lock()
                .map(|state| state.active().clone())
                .unwrap_or_default()
        }
        #[cfg(not(target_os = "linux"))]
        {
            SessionSet::default()
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::io::RawFd;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime};

    use crate::domain::signal::{Session, SessionSet, Signal};
    use crate::error::AppResult;
    use crate::ports::EventSource;
    use crate::sys;

    const LOGIND_DIR: &str = "/run/systemd/sessions";
    const UTMP_PATH: &str = "/var/run/utmp";
    const WATCH_DIR: u32 = (libc::IN_CREATE
        | libc::IN_DELETE
        | libc::IN_MOVED_TO
        | libc::IN_MOVED_FROM
        | libc::IN_ATTRIB) as u32;
    const WATCH_FILE: u32 = (libc::IN_MODIFY
        | libc::IN_CLOSE_WRITE
        | libc::IN_ATTRIB
        | libc::IN_DELETE_SELF
        | libc::IN_MOVE_SELF) as u32;
    const REBUILD_FLAGS: u32 =
        (libc::IN_IGNORED | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) as u32;

    pub(super) struct SessionState {
        tracked: SessionSet,
        active: SessionSet,
        active_tty_idle_secs: u64,
        next_probe: Option<Instant>,
        initial_emitted: bool,
    }

    impl SessionState {
        fn new(set: SessionSet, active_tty_idle_secs: u64) -> Self {
            Self {
                tracked: set.clone(),
                active: set,
                active_tty_idle_secs,
                next_probe: Some(Instant::now()),
                initial_emitted: false,
            }
        }

        fn update(&mut self, set: SessionSet) {
            self.tracked = set;
            self.next_probe = Some(Instant::now());
        }

        pub(super) fn take_initial(&mut self) -> Option<Signal> {
            if self.initial_emitted {
                return None;
            }
            self.initial_emitted = true;
            if self.active.is_empty() {
                None
            } else {
                Some(Signal::Sessions(self.active.clone()))
            }
        }

        pub(super) fn probe(&mut self, now: Instant) -> Option<SessionSet> {
            self.probe_with(now, tty_idle_secs)
        }

        fn probe_with<F>(&mut self, now: Instant, idle: F) -> Option<SessionSet>
        where
            F: Fn(&str) -> u64,
        {
            let due = self.next_probe?;
            if now < due {
                return None;
            }

            let filtered: Vec<Session> = self
                .tracked
                .sessions()
                .iter()
                .filter(|session| idle(&session.tty) <= self.active_tty_idle_secs)
                .cloned()
                .collect();
            let filtered = SessionSet::new(filtered);
            let changed = filtered != self.active;
            self.active = filtered;
            self.next_probe =
                Some(now + Duration::from_secs(if self.active.is_empty() { 30 } else { 5 }));

            // Re-emit a non-empty active set every tick so the reducer keeps
            // refreshing `last_interactive`; a logout (empty set) only emits on
            // change. The tracked set is never replaced by the filter, so a tty
            // that becomes active again re-enters without a new login event.
            if changed || !self.active.is_empty() {
                Some(self.active.clone())
            } else {
                None
            }
        }

        pub(super) fn next_deadline(&self) -> Option<Instant> {
            self.next_probe
        }

        pub(super) fn active(&self) -> &SessionSet {
            &self.active
        }
    }

    enum Backend {
        Logind { dir: PathBuf, wd: i32 },
        Utmp { path: PathBuf, wd: i32 },
    }

    trait WatchOps {
        fn add_watch(&self, fd: RawFd, path: &Path, mask: u32) -> std::io::Result<i32>;
    }

    struct RealWatchOps;

    impl WatchOps for RealWatchOps {
        fn add_watch(&self, fd: RawFd, path: &Path, mask: u32) -> std::io::Result<i32> {
            sys::inotify_add_watch(fd, path, mask)
        }
    }

    pub(super) struct SessionSource {
        fd: sys::Fd,
        token: u64,
        backend: Backend,
        state: Arc<Mutex<SessionState>>,
        ops: Arc<dyn WatchOps>,
    }

    pub(super) fn start(
        token: u64,
        active_tty_idle_secs: u64,
    ) -> Option<(SessionSource, super::SessionMonitor)> {
        let fd = sys::inotify_create().ok()?;
        let logind = Path::new(LOGIND_DIR);
        let backend;

        if logind.is_dir() {
            let wd = sys::inotify_add_watch(fd.raw(), logind, WATCH_DIR).ok()?;
            backend = Backend::Logind {
                dir: logind.to_path_buf(),
                wd,
            };
        } else if Path::new(UTMP_PATH).exists() {
            let wd = sys::inotify_add_watch(fd.raw(), Path::new(UTMP_PATH), WATCH_FILE).ok()?;
            backend = Backend::Utmp {
                path: PathBuf::from(UTMP_PATH),
                wd,
            };
        } else {
            return None;
        }

        let set = scan(&backend, active_tty_idle_secs);
        let state = Arc::new(Mutex::new(SessionState::new(set, active_tty_idle_secs)));
        let source = SessionSource {
            fd,
            token,
            backend,
            state: Arc::clone(&state),
            ops: Arc::new(RealWatchOps),
        };
        let monitor = super::SessionMonitor { state };

        Some((source, monitor))
    }

    fn scan(backend: &Backend, idle_secs: u64) -> SessionSet {
        match backend {
            Backend::Logind { dir, .. } => scan_logind(dir),
            Backend::Utmp { .. } => crate::adapters::procfs::read_sessions(idle_secs),
        }
    }

    fn scan_logind(dir: &Path) -> SessionSet {
        let mut by_tty: BTreeMap<String, bool> = BTreeMap::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return SessionSet::default();
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".ref") {
                continue;
            }
            let Ok(content) = fs::read_to_string(entry.path()) else {
                continue;
            };

            let mut tty = None;
            let mut active = true;
            let mut remote = false;
            for line in content.lines() {
                if let Some(value) = line.strip_prefix("TTY=") {
                    tty = Some(value.trim().to_string());
                } else if let Some(value) = line.strip_prefix("ACTIVE=") {
                    active = value.trim() == "1";
                } else if let Some(value) = line.strip_prefix("STATE=") {
                    if value.trim() == "closing" {
                        active = false;
                    }
                } else if let Some(value) = line.strip_prefix("REMOTE=") {
                    remote = value.trim() == "1";
                }
            }

            if let Some(tty) = tty {
                if active && (tty.starts_with("pts/") || tty.starts_with("tty")) {
                    by_tty
                        .entry(tty)
                        .and_modify(|value| *value |= remote)
                        .or_insert(remote);
                }
            }
        }

        SessionSet::new(
            by_tty
                .into_iter()
                .map(|(tty, remote)| Session { tty, remote })
                .collect(),
        )
    }

    fn tty_idle_secs(tty: &str) -> u64 {
        path_idle_secs(&Path::new("/dev").join(tty))
    }

    fn path_idle_secs(path: &Path) -> u64 {
        let Ok(metadata) = fs::metadata(path) else {
            // Fail safe for the human: unknown idleness counts as active.
            return 0;
        };
        let Ok(accessed) = metadata.accessed() else {
            return 0;
        };
        SystemTime::now()
            .duration_since(accessed)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    struct InotifyEvent {
        mask: u32,
    }

    fn parse_events(buffer: &[u8]) -> Vec<InotifyEvent> {
        let header = std::mem::size_of::<libc::inotify_event>();
        let mut events = Vec::new();
        let mut offset = 0;

        while offset + header <= buffer.len() {
            let event = unsafe {
                std::ptr::read_unaligned(buffer[offset..].as_ptr() as *const libc::inotify_event)
            };
            let length = header + event.len as usize;
            if length == 0 || offset + length > buffer.len() {
                break;
            }
            events.push(InotifyEvent { mask: event.mask });
            offset += length;
        }

        events
    }

    impl SessionSource {
        fn re_add(&mut self) {
            let fd = self.fd.raw();
            let (path, mask, watch) = match &mut self.backend {
                Backend::Logind { dir, wd } => (dir.clone(), WATCH_DIR, wd),
                Backend::Utmp { path, wd } => (path.clone(), WATCH_FILE, wd),
            };

            match self.ops.add_watch(fd, &path, mask) {
                Ok(new_watch) => *watch = new_watch,
                Err(error) => eprintln!(
                    "[useless] session watch re-add failed for {}: {error}",
                    path.display()
                ),
            }
        }
    }

    impl EventSource for SessionSource {
        fn register(&mut self, epoll_fd: RawFd) -> AppResult<Vec<u64>> {
            sys::epoll_add(epoll_fd, self.fd.raw(), self.token, libc::EPOLLIN as u32)?;
            Ok(vec![self.token])
        }

        fn handle(&mut self, _token: u64, _events: u32) -> AppResult<Vec<Signal>> {
            let mut rebuild = false;
            let mut buffer = [0u8; 4096];

            loop {
                match sys::read_inotify(self.fd.raw(), &mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        for event in parse_events(&buffer[..count]) {
                            if event.mask & REBUILD_FLAGS != 0 {
                                rebuild = true;
                            }
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }

            if rebuild {
                self.re_add();
            }

            let idle_secs = self
                .state
                .lock()
                .expect("session state")
                .active_tty_idle_secs;
            let set = scan(&self.backend, idle_secs);
            // A logind create event can race the session-file write. The
            // `.ref` creation usually supplies a second event; this retry covers
            // the remaining gap without blocking on normal events.
            let set = if set.is_empty() {
                std::thread::sleep(Duration::from_millis(50));
                scan(&self.backend, idle_secs)
            } else {
                set
            };

            let mut state = self.state.lock().expect("session state");
            state.update(set);
            let signal = state.probe(Instant::now()).map(Signal::Sessions);

            Ok(signal.into_iter().collect())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::fs;
        use std::os::unix::io::RawFd;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        use crate::domain::signal::{Session, SessionSet};
        use crate::sys;

        use super::{
            Backend, SessionSource, SessionState, WatchOps, parse_events, path_idle_secs,
            scan_logind,
        };

        fn temp_dir(name: &str) -> PathBuf {
            std::env::temp_dir().join(format!("useless-sessions-{}-{name}", std::process::id()))
        }

        #[test]
        fn scans_logind_session_files() {
            let dir = temp_dir("logind");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("dir");
            fs::write(dir.join("1"), "TTY=pts/0\nACTIVE=1\nREMOTE=1\n").expect("session");
            fs::write(dir.join("2"), "TTY=pts/1\nACTIVE=0\n").expect("inactive");
            fs::write(dir.join("3"), "TTY=pts/2\nACTIVE=1\nSTATE=closing\n").expect("closing");
            fs::write(dir.join("1.ref"), "TTY=pts/9\nACTIVE=1\n").expect("ref");

            let set = scan_logind(&dir);

            assert_eq!(set.tty_count(), 1);
            assert_eq!(set.ssh_count(), 1);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn deduplicates_same_tty_sessions() {
            let dir = temp_dir("dedup");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("dir");
            fs::write(dir.join("1"), "TTY=pts/0\nACTIVE=1\n").expect("one");
            fs::write(dir.join("2"), "TTY=pts/0\nACTIVE=1\nREMOTE=1\n").expect("two");

            let set = scan_logind(&dir);

            assert_eq!(set.tty_count(), 1);
            assert_eq!(set.ssh_count(), 1);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn parses_variable_length_inotify_events() {
            let name = b"session-file\0";
            let mut buffer = vec![0u8; std::mem::size_of::<libc::inotify_event>() + name.len()];
            let event = libc::inotify_event {
                wd: 3,
                mask: libc::IN_CREATE,
                cookie: 0,
                len: name.len() as u32,
            };
            unsafe {
                std::ptr::write_unaligned(buffer.as_mut_ptr() as *mut libc::inotify_event, event);
            }
            buffer[std::mem::size_of::<libc::inotify_event>()..].copy_from_slice(name);

            let events = parse_events(&buffer);

            assert_eq!(events.len(), 1);
            assert_eq!(events[0].mask, libc::IN_CREATE);
        }

        #[test]
        fn probe_filters_idle_sessions() {
            let mut state = SessionState::new(
                SessionSet::new(vec![
                    Session {
                        tty: "pts/0".to_string(),
                        remote: false,
                    },
                    Session {
                        tty: "pts/1".to_string(),
                        remote: false,
                    },
                ]),
                60,
            );
            state.next_probe = Some(Instant::now() - Duration::from_secs(1));

            let filtered =
                state.probe_with(Instant::now(), |tty| if tty == "pts/0" { 999 } else { 1 });

            let filtered = filtered.expect("changed set");
            assert_eq!(filtered.tty_count(), 1);
            assert_eq!(filtered.sessions()[0].tty, "pts/1");
        }

        #[test]
        fn probe_reemits_active_sessions_even_when_unchanged() {
            let mut state = SessionState::new(
                SessionSet::new(vec![Session {
                    tty: "pts/0".to_string(),
                    remote: false,
                }]),
                60,
            );
            let now = Instant::now();

            state.next_probe = Some(now - Duration::from_secs(1));
            assert!(state.probe_with(now, |_| 1).is_some());
            state.next_probe = Some(now - Duration::from_secs(1));
            assert!(state.probe_with(now, |_| 1).is_some());
        }

        #[test]
        fn idle_tty_reactivates_from_the_tracked_set() {
            let mut state = SessionState::new(
                SessionSet::new(vec![Session {
                    tty: "pts/0".to_string(),
                    remote: false,
                }]),
                60,
            );
            let now = Instant::now();

            state.next_probe = Some(now - Duration::from_secs(1));
            let idle = state.probe_with(now, |_| 999).expect("idle emit");
            assert_eq!(idle.tty_count(), 0);

            state.next_probe = Some(now - Duration::from_secs(1));
            let active = state.probe_with(now, |_| 1).expect("reactivation");
            assert_eq!(active.tty_count(), 1);
        }

        struct FakeWatchOps {
            calls: AtomicUsize,
            fail: bool,
        }

        impl WatchOps for FakeWatchOps {
            fn add_watch(
                &self,
                _fd: RawFd,
                _path: &std::path::Path,
                _mask: u32,
            ) -> std::io::Result<i32> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                if self.fail {
                    Err(std::io::Error::other("fake failure"))
                } else {
                    Ok(42)
                }
            }
        }

        fn source_with(ops: Arc<dyn WatchOps>) -> SessionSource {
            let fd = sys::inotify_create().expect("inotify");
            let state = Arc::new(Mutex::new(SessionState::new(SessionSet::default(), 60)));
            SessionSource {
                fd,
                token: 3,
                backend: Backend::Utmp {
                    path: PathBuf::from("/tmp/useless-utmp-test"),
                    wd: 1,
                },
                state,
                ops,
            }
        }

        #[test]
        fn re_add_invokes_watch_ops() {
            let ops = Arc::new(FakeWatchOps {
                calls: AtomicUsize::new(0),
                fail: false,
            });
            let mut source = source_with(ops.clone());
            source.re_add();

            assert_eq!(ops.calls.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn failed_re_add_does_not_panic() {
            let ops = Arc::new(FakeWatchOps {
                calls: AtomicUsize::new(0),
                fail: true,
            });
            let mut source = source_with(ops.clone());
            source.re_add();

            assert_eq!(ops.calls.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn path_idle_reflects_access_time() {
            let path = temp_dir("atime");
            fs::write(&path, b"x").expect("file");
            let old = std::time::SystemTime::now() - Duration::from_secs(300);
            let file = fs::File::options().read(true).open(&path).expect("open");
            file.set_times(fs::FileTimes::new().set_accessed(old))
                .expect("set atime");

            assert!(path_idle_secs(&path) >= 250);
            let _ = fs::remove_file(&path);
        }
    }
}
