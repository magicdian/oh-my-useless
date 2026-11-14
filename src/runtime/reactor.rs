#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::time::Instant;

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;
    use crate::sys;

    const TIMER_TOKEN: u64 = 1;

    /// One epoll set plus one deadline timer. Sources own their fds; the
    /// reactor only routes readiness by token.
    pub struct Reactor {
        epoll: sys::Fd,
        timer: sys::Fd,
        sources: Vec<Box<dyn EventSource>>,
        tokens: HashMap<u64, usize>,
    }

    impl Reactor {
        pub fn new() -> AppResult<Self> {
            let epoll = sys::epoll_create()?;
            let timer = sys::timerfd_create()?;
            sys::epoll_add(epoll.raw(), timer.raw(), TIMER_TOKEN, libc::EPOLLIN as u32)?;
            Ok(Self {
                epoll,
                timer,
                sources: Vec::new(),
                tokens: HashMap::new(),
            })
        }

        pub fn add(&mut self, mut source: Box<dyn EventSource>) -> AppResult<()> {
            let tokens = source.register(self.epoll.raw())?;
            let index = self.sources.len();
            for token in tokens {
                self.tokens.insert(token, index);
            }
            self.sources.push(source);
            Ok(())
        }

        /// Blocks until a source fires or `deadline` elapses. The deadline is
        /// armed through a timerfd, so this never spins.
        pub fn wait(&mut self, deadline: Instant) -> AppResult<Vec<Signal>> {
            sys::timerfd_arm(self.timer.raw(), deadline)?;

            let mut ready = [sys::ReadyEvent::default(); 32];
            let count = sys::epoll_wait(self.epoll.raw(), &mut ready, -1)?;
            let mut signals = Vec::new();

            for event in ready.iter().take(count) {
                if event.token == TIMER_TOKEN {
                    sys::timerfd_drain(self.timer.raw())?;
                    continue;
                }

                if let Some(index) = self.tokens.get(&event.token).copied() {
                    if let Some(source) = self.sources.get_mut(index) {
                        signals.extend(source.handle(event.token, event.events)?);
                    }
                }
            }

            Ok(signals)
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::time::Instant;

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;

    /// Non-Linux fallback for development and tests. It preserves the sampler
    /// deadline and performs no event dispatch.
    pub struct Reactor;

    impl Reactor {
        pub fn new() -> AppResult<Self> {
            Ok(Self)
        }

        pub fn add(&mut self, _source: Box<dyn EventSource>) -> AppResult<()> {
            Ok(())
        }

        pub fn wait(&mut self, deadline: Instant) -> AppResult<Vec<Signal>> {
            let wait = deadline.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            Ok(Vec::new())
        }
    }
}

pub use imp::Reactor;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;
    use crate::sys;

    use super::Reactor;

    struct PipeSource {
        read: sys::Fd,
        token: u64,
        reads: Arc<AtomicUsize>,
    }

    impl EventSource for PipeSource {
        fn register(&mut self, epoll_fd: std::os::unix::io::RawFd) -> AppResult<Vec<u64>> {
            sys::epoll_add(epoll_fd, self.read.raw(), self.token, libc::EPOLLIN as u32)?;
            Ok(vec![self.token])
        }

        fn handle(&mut self, _token: u64, _events: u32) -> AppResult<Vec<Signal>> {
            let mut buffer = [0u8; 16];
            let _ = sys::read_fd(self.read.raw(), &mut buffer)?;
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(vec![Signal::SourceDegraded {
                source: "pipe".to_string(),
                message: "test".to_string(),
            }])
        }
    }

    #[test]
    fn dispatches_ready_pipe_sources() {
        let (read, write) = sys::pipe().expect("pipe");
        let reads = Arc::new(AtomicUsize::new(0));
        let mut reactor = Reactor::new().expect("reactor");
        reactor
            .add(Box::new(PipeSource {
                read,
                token: 10,
                reads: Arc::clone(&reads),
            }))
            .expect("add source");

        sys::write_fd(write.raw(), b'x').expect("write");
        let signals = reactor
            .wait(Instant::now() + Duration::from_secs(1))
            .expect("wait");

        assert_eq!(signals.len(), 1);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn timer_deadline_wakes_without_fd_events() {
        let mut reactor = Reactor::new().expect("reactor");
        let start = Instant::now();
        let signals = reactor
            .wait(start + Duration::from_millis(50))
            .expect("wait");

        assert!(signals.is_empty());
        assert!(start.elapsed() >= Duration::from_millis(30));
    }
}
