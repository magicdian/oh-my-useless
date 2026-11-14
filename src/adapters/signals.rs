#[cfg(target_os = "linux")]
mod imp {
    use std::os::unix::io::RawFd;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;
    use crate::sys;

    pub struct SignalSource {
        fd: sys::Fd,
        token: u64,
        terminate: &'static AtomicBool,
        reload: &'static AtomicBool,
    }

    impl SignalSource {
        pub fn new(
            token: u64,
            terminate: &'static AtomicBool,
            reload: &'static AtomicBool,
        ) -> AppResult<Self> {
            let fd = sys::signalfd_create(&[libc::SIGHUP, libc::SIGINT, libc::SIGTERM])?;
            Ok(Self {
                fd,
                token,
                terminate,
                reload,
            })
        }
    }

    impl EventSource for SignalSource {
        fn register(&mut self, epoll_fd: RawFd) -> AppResult<Vec<u64>> {
            sys::epoll_add(epoll_fd, self.fd.raw(), self.token, libc::EPOLLIN as u32)?;
            Ok(vec![self.token])
        }

        fn handle(&mut self, _token: u64, _events: u32) -> AppResult<Vec<Signal>> {
            loop {
                match sys::read_signal(self.fd.raw()) {
                    Ok(signal) => match signal {
                        libc::SIGHUP => self.reload.store(true, Ordering::Relaxed),
                        libc::SIGINT | libc::SIGTERM => {
                            self.terminate.store(true, Ordering::Relaxed)
                        }
                        _ => {}
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(Vec::new())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::os::unix::io::RawFd;
    use std::sync::atomic::AtomicBool;

    use crate::domain::signal::Signal;
    use crate::error::AppResult;
    use crate::ports::EventSource;

    /// Non-Linux fallback: the daemon installs raw signal handlers instead.
    pub struct SignalSource;

    impl SignalSource {
        pub fn new(
            _token: u64,
            _terminate: &'static AtomicBool,
            _reload: &'static AtomicBool,
        ) -> AppResult<Self> {
            Ok(Self)
        }
    }

    impl EventSource for SignalSource {
        fn register(&mut self, _epoll_fd: RawFd) -> AppResult<Vec<u64>> {
            Ok(Vec::new())
        }

        fn handle(&mut self, _token: u64, _events: u32) -> AppResult<Vec<Signal>> {
            Ok(Vec::new())
        }
    }
}

pub use imp::SignalSource;
