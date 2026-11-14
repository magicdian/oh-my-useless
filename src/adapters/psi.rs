use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;

use crate::domain::signal::PressureReading;
use crate::sys;

#[cfg(target_os = "linux")]
pub(crate) const PSI_TOKEN_BASE: u64 = 100;

#[allow(dead_code)]
pub trait PsiOps {
    fn exists(&self, path: &Path) -> bool;
    fn open(&self, path: &Path) -> io::Result<RawFd>;
    fn write(&self, fd: RawFd, bytes: &[u8]) -> io::Result<()>;
    fn read(&self, fd: RawFd, buffer: &mut [u8]) -> io::Result<usize>;
    fn close(&self, fd: RawFd);
}

pub struct RealPsiOps;

impl PsiOps for RealPsiOps {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn open(&self, path: &Path) -> io::Result<RawFd> {
        let fd = sys::open_rw(path)?;
        let raw = fd.raw();
        // The watch owns the descriptor and closes it through `PsiOps::close`.
        std::mem::forget(fd);
        Ok(raw)
    }

    fn write(&self, fd: RawFd, bytes: &[u8]) -> io::Result<()> {
        sys::write_all(fd, bytes)
    }

    fn read(&self, fd: RawFd, buffer: &mut [u8]) -> io::Result<usize> {
        sys::pread(fd, buffer)
    }

    fn close(&self, fd: RawFd) {
        unsafe {
            libc::close(fd);
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsiCapability {
    pub psi: bool,
    pub trigger_write: bool,
}

impl PsiCapability {
    pub fn label(self) -> String {
        format!(
            "psi={} trigger_write={}",
            status(self.psi),
            status(self.trigger_write)
        )
    }
}

fn status(value: bool) -> &'static str {
    if value { "ok" } else { "unavailable" }
}

#[allow(dead_code)]
pub fn parse_pressure(text: &str) -> Option<PressureReading> {
    let mut reading = PressureReading::default();
    let mut found = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("some ") {
            reading.some_avg10_x100 = field_x100(rest, "avg10=").unwrap_or(0);
            reading.some_total_us = field_u64(rest, "total=").unwrap_or(0);
            found = true;
        } else if let Some(rest) = line.strip_prefix("full ") {
            reading.full_avg10_x100 = field_x100(rest, "avg10=").unwrap_or(0);
            reading.full_total_us = field_u64(rest, "total=").unwrap_or(0);
            found = true;
        }
    }

    found.then_some(reading)
}

#[allow(dead_code)]
fn field_u64(text: &str, key: &str) -> Option<u64> {
    text.split_whitespace()
        .find_map(|part| part.strip_prefix(key)?.parse().ok())
}

#[allow(dead_code)]
fn field_x100(text: &str, key: &str) -> Option<u16> {
    let raw = text
        .split_whitespace()
        .find_map(|part| part.strip_prefix(key))?;
    let value: f64 = raw.parse().ok()?;
    Some((value * 100.0).round().clamp(0.0, u16::MAX as f64) as u16)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::unix::io::RawFd;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use crate::domain::signal::{PressureClass, PressureScope, Signal};
    use crate::error::AppResult;
    use crate::ports::EventSource;
    use crate::sys;

    use super::{PSI_TOKEN_BASE, PsiCapability, PsiOps, parse_pressure};

    const WATCH_FLAGS: u32 = (libc::EPOLLIN | libc::EPOLLPRI | libc::EPOLLERR) as u32;
    const ERROR_FLAGS: u32 = (libc::EPOLLERR | libc::EPOLLHUP) as u32;

    #[derive(Debug, Clone, Copy)]
    struct WatchSpec {
        class: PressureClass,
        kind: &'static str,
        path: &'static str,
    }

    fn default_specs() -> Vec<WatchSpec> {
        vec![
            WatchSpec {
                class: PressureClass::Cpu,
                kind: "some",
                path: "/proc/pressure/cpu",
            },
            WatchSpec {
                class: PressureClass::Memory,
                kind: "some",
                path: "/proc/pressure/memory",
            },
            WatchSpec {
                class: PressureClass::Memory,
                kind: "full",
                path: "/proc/pressure/memory",
            },
            WatchSpec {
                class: PressureClass::Io,
                kind: "some",
                path: "/proc/pressure/io",
            },
            WatchSpec {
                class: PressureClass::Io,
                kind: "full",
                path: "/proc/pressure/io",
            },
        ]
    }

    pub struct PsiSource {
        ops: Arc<dyn PsiOps>,
        specs: Vec<WatchSpec>,
        watches: Vec<Option<RawFd>>,
        tokens: Vec<u64>,
        enter_us: u64,
        window_us: u64,
        capability: Arc<Mutex<PsiCapability>>,
        epoll_fd: Option<RawFd>,
    }

    impl PsiSource {
        pub fn new(
            ops: Arc<dyn PsiOps>,
            enter_ms: u64,
            window_ms: u64,
            capability: Arc<Mutex<PsiCapability>>,
        ) -> Self {
            let specs = default_specs();
            let tokens = (0..specs.len())
                .map(|index| PSI_TOKEN_BASE + index as u64)
                .collect();

            Self {
                ops,
                watches: vec![None; specs.len()],
                specs,
                tokens,
                enter_us: enter_ms.saturating_mul(1000),
                window_us: window_ms.saturating_mul(1000),
                capability,
                epoll_fd: None,
            }
        }

        fn trigger(&self, index: usize) -> String {
            format!(
                "{} {} {}",
                self.specs[index].kind, self.enter_us, self.window_us
            )
        }

        fn open_watch(&self, index: usize) -> io::Result<RawFd> {
            let spec = &self.specs[index];
            let trigger = self.trigger(index);
            let fd = self.ops.open(Path::new(spec.path))?;

            match self.ops.write(fd, trigger.as_bytes()) {
                Ok(()) => Ok(fd),
                Err(error) if error.raw_os_error() == Some(libc::EBUSY) => {
                    self.ops.close(fd);
                    let fd = self.ops.open(Path::new(spec.path))?;
                    match self.ops.write(fd, trigger.as_bytes()) {
                        Ok(()) => Ok(fd),
                        Err(error) => {
                            self.ops.close(fd);
                            Err(error)
                        }
                    }
                }
                Err(error) => {
                    self.ops.close(fd);
                    Err(error)
                }
            }
        }

        fn close_watch(&mut self, index: usize) {
            if let Some(fd) = self.watches[index].take() {
                self.ops.close(fd);
            }
        }

        fn rebuild(&mut self, index: usize) -> AppResult<Vec<Signal>> {
            self.close_watch(index);
            let Some(epoll_fd) = self.epoll_fd else {
                return Ok(Vec::new());
            };

            match self.open_watch(index) {
                Ok(fd) => match sys::epoll_add(epoll_fd, fd, self.tokens[index], WATCH_FLAGS) {
                    Ok(()) => {
                        self.watches[index] = Some(fd);
                        Ok(Vec::new())
                    }
                    Err(error) => {
                        self.ops.close(fd);
                        Ok(vec![degraded(&self.specs[index].path, &error)])
                    }
                },
                Err(error) => Ok(vec![degraded(&self.specs[index].path, &error)]),
            }
        }
    }

    fn degraded(path: &str, error: &io::Error) -> Signal {
        Signal::SourceDegraded {
            source: format!("psi:{path}"),
            message: error.to_string(),
        }
    }

    impl EventSource for PsiSource {
        fn register(&mut self, epoll_fd: RawFd) -> AppResult<Vec<u64>> {
            self.epoll_fd = Some(epoll_fd);
            let mut registered = Vec::new();
            let mut psi = false;
            let mut trigger_write = false;

            for index in 0..self.specs.len() {
                let exists = self.ops.exists(Path::new(self.specs[index].path));
                match self.open_watch(index) {
                    Ok(fd) => match sys::epoll_add(epoll_fd, fd, self.tokens[index], WATCH_FLAGS) {
                        Ok(()) => {
                            self.watches[index] = Some(fd);
                            registered.push(self.tokens[index]);
                            psi = true;
                            trigger_write = true;
                        }
                        Err(_) => {
                            self.ops.close(fd);
                            psi |= exists;
                        }
                    },
                    Err(_) => {
                        psi |= exists;
                    }
                }
            }

            let mut capability = self.capability.lock().expect("psi capability");
            capability.psi = psi;
            capability.trigger_write = trigger_write;

            Ok(registered)
        }

        fn handle(&mut self, token: u64, events: u32) -> AppResult<Vec<Signal>> {
            let Some(index) = token
                .checked_sub(PSI_TOKEN_BASE)
                .map(|value| value as usize)
            else {
                return Ok(Vec::new());
            };
            if index >= self.specs.len() {
                return Ok(Vec::new());
            }
            if events & ERROR_FLAGS != 0 {
                return self.rebuild(index);
            }

            let Some(fd) = self.watches[index] else {
                return Ok(Vec::new());
            };

            let mut buffer = [0u8; 512];
            let count = match self.ops.read(fd, &mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Vec::new()),
                Err(_) => return self.rebuild(index),
            };
            let text = String::from_utf8_lossy(&buffer[..count]);
            let reading = parse_pressure(&text).unwrap_or_default();

            Ok(vec![Signal::Pressure {
                class: self.specs[index].class,
                scope: PressureScope::Host,
                reading,
            }])
        }
    }

    impl Drop for PsiSource {
        fn drop(&mut self) {
            for index in 0..self.watches.len() {
                self.close_watch(index);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::os::unix::io::RawFd;
    use std::sync::{Arc, Mutex};

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;

    use super::{PsiCapability, PsiOps};

    /// Non-Linux fallback: PSI is a Linux interface.
    pub struct PsiSource;

    impl PsiSource {
        pub fn new(
            _ops: Arc<dyn PsiOps>,
            _enter_ms: u64,
            _window_ms: u64,
            _capability: Arc<Mutex<PsiCapability>>,
        ) -> Self {
            Self
        }
    }

    impl EventSource for PsiSource {
        fn register(&mut self, _epoll_fd: RawFd) -> AppResult<Vec<u64>> {
            Ok(Vec::new())
        }

        fn handle(&mut self, _token: u64, _events: u32) -> AppResult<Vec<Signal>> {
            Ok(Vec::new())
        }
    }
}

pub use imp::PsiSource;

#[cfg(test)]
mod tests {
    use super::{PsiCapability, parse_pressure};

    #[test]
    fn parses_both_psi_lines() {
        let reading = parse_pressure(
            "some avg10=2.10 avg60=1.00 avg300=0.50 total=123456\n\
             full avg10=0.40 avg60=0.20 avg300=0.10 total=12345\n",
        )
        .expect("reading");

        assert_eq!(reading.some_avg10_x100, 210);
        assert_eq!(reading.full_avg10_x100, 40);
        assert_eq!(reading.some_total_us, 123456);
        assert_eq!(reading.full_total_us, 12345);
    }

    #[test]
    fn parses_a_some_only_file() {
        let reading =
            parse_pressure("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").expect("reading");
        assert_eq!(reading.some_avg10_x100, 0);
        assert_eq!(reading.full_avg10_x100, 0);
    }

    #[test]
    fn rejects_unrelated_text() {
        assert!(parse_pressure("nothing useful\n").is_none());
        assert!(parse_pressure("").is_none());
    }

    #[test]
    fn capability_label_is_operator_readable() {
        let capability = PsiCapability {
            psi: true,
            trigger_write: false,
        };
        assert_eq!(capability.label(), "psi=ok trigger_write=unavailable");
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use std::os::unix::io::RawFd;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::domain::signal::{PressureClass, PressureScope, Signal};
    use crate::ports::EventSource;
    use crate::sys;

    use super::{PSI_TOKEN_BASE, PsiCapability, PsiOps, PsiSource};

    struct FakePsiOps {
        fixture: Vec<u8>,
        writes: Mutex<Vec<String>>,
        opens: AtomicUsize,
        busy_once: AtomicBool,
        fail_open: AtomicBool,
    }

    impl FakePsiOps {
        fn new(fixture: &[u8]) -> Self {
            Self {
                fixture: fixture.to_vec(),
                writes: Mutex::new(Vec::new()),
                opens: AtomicUsize::new(0),
                busy_once: AtomicBool::new(false),
                fail_open: AtomicBool::new(false),
            }
        }
    }

    impl PsiOps for FakePsiOps {
        fn exists(&self, _path: &Path) -> bool {
            !self.fail_open.load(Ordering::Relaxed)
        }

        fn open(&self, _path: &Path) -> std::io::Result<RawFd> {
            if self.fail_open.load(Ordering::Relaxed) {
                return Err(std::io::Error::from_raw_os_error(libc::ENOENT));
            }
            self.opens.fetch_add(1, Ordering::Relaxed);
            let fd = sys::open_rw(Path::new("/dev/null"))?;
            let raw = fd.raw();
            std::mem::forget(fd);
            Ok(raw)
        }

        fn write(&self, _fd: RawFd, bytes: &[u8]) -> std::io::Result<()> {
            if self.busy_once.swap(false, Ordering::Relaxed) {
                return Err(std::io::Error::from_raw_os_error(libc::EBUSY));
            }
            self.writes
                .lock()
                .expect("writes")
                .push(String::from_utf8_lossy(bytes).to_string());
            Ok(())
        }

        fn read(&self, _fd: RawFd, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.fixture.len().min(buffer.len());
            buffer[..count].copy_from_slice(&self.fixture[..count]);
            Ok(count)
        }

        fn close(&self, fd: RawFd) {
            unsafe {
                libc::close(fd);
            }
        }
    }

    fn source(ops: Arc<FakePsiOps>) -> (PsiSource, Arc<Mutex<PsiCapability>>, sys::Fd) {
        let capability = Arc::new(Mutex::new(PsiCapability::default()));
        let source = PsiSource::new(ops, 50, 1000, Arc::clone(&capability));
        let epoll = sys::epoll_create().expect("epoll");
        (source, capability, epoll)
    }

    #[test]
    fn registers_one_trigger_per_watch() {
        let ops = Arc::new(FakePsiOps::new(
            b"some avg10=2.10 avg60=0.00 avg300=0.00 total=1\n",
        ));
        let (mut psi, capability, epoll) = source(Arc::clone(&ops));
        let tokens = psi.register(epoll.raw()).expect("register");

        assert_eq!(tokens.len(), 5);
        let writes = ops.writes.lock().expect("writes");
        assert!(writes.iter().any(|value| value == "some 50000 1000000"));
        assert!(writes.iter().any(|value| value == "full 50000 1000000"));
        assert_eq!(
            *capability.lock().expect("capability"),
            PsiCapability {
                psi: true,
                trigger_write: true
            }
        );
    }

    #[test]
    fn reopens_after_ebusy() {
        let ops = Arc::new(FakePsiOps::new(b"some avg10=1.00 total=1\n"));
        ops.busy_once.store(true, Ordering::Relaxed);
        let (mut psi, _, epoll) = source(Arc::clone(&ops));
        psi.register(epoll.raw()).expect("register");

        assert_eq!(ops.opens.load(Ordering::Relaxed), 6);
        assert_eq!(ops.writes.lock().expect("writes").len(), 5);
    }

    #[test]
    fn missing_files_degrade_capability() {
        let ops = Arc::new(FakePsiOps::new(b""));
        ops.fail_open.store(true, Ordering::Relaxed);
        let (mut psi, capability, epoll) = source(Arc::clone(&ops));
        let tokens = psi.register(epoll.raw()).expect("register");

        assert!(tokens.is_empty());
        assert_eq!(
            *capability.lock().expect("capability"),
            PsiCapability {
                psi: false,
                trigger_write: false
            }
        );
    }

    #[test]
    fn poll_error_rebuilds_the_watch() {
        let ops = Arc::new(FakePsiOps::new(b"some avg10=1.00 total=1\n"));
        let (mut psi, _, epoll) = source(Arc::clone(&ops));
        psi.register(epoll.raw()).expect("register");
        let before = ops.opens.load(Ordering::Relaxed);

        psi.handle(PSI_TOKEN_BASE, libc::EPOLLERR as u32)
            .expect("handle error");

        assert_eq!(ops.opens.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn emits_a_pressure_signal_on_readiness() {
        let ops = Arc::new(FakePsiOps::new(
            b"some avg10=2.10 avg60=0.00 avg300=0.00 total=1\n",
        ));
        let (mut psi, _, epoll) = source(Arc::clone(&ops));
        psi.register(epoll.raw()).expect("register");

        let signals = psi
            .handle(PSI_TOKEN_BASE, libc::EPOLLPRI as u32)
            .expect("handle");

        match signals.as_slice() {
            [
                Signal::Pressure {
                    class,
                    scope,
                    reading,
                },
            ] => {
                assert_eq!(*class, PressureClass::Cpu);
                assert_eq!(*scope, PressureScope::Host);
                assert_eq!(reading.some_avg10_x100, 210);
            }
            other => panic!("unexpected signals: {other:?}"),
        }
    }
}
