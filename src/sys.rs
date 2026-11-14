// Many wrappers are only exercised on the Linux target or in tests.
#![allow(dead_code)]

use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;
#[cfg(not(target_os = "linux"))]
use std::time::Instant;

/// A readiness notification, independent of the platform's event struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadyEvent {
    pub token: u64,
    pub events: u32,
}

/// Owned file descriptor. Closing is the only cleanup this layer performs.
#[derive(Debug)]
pub struct Fd(RawFd);

impl Fd {
    pub fn raw(&self) -> RawFd {
        self.0
    }

    pub fn from_raw(fd: RawFd) -> Self {
        Self(fd)
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

pub fn block_signals(signals: &[libc::c_int]) -> io::Result<()> {
    let mut set = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    unsafe {
        libc::sigemptyset(&mut set);
    }
    for signal in signals {
        unsafe {
            libc::sigaddset(&mut set, *signal);
        }
    }
    let rc = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    Ok(())
}

pub fn open_rw(path: &Path) -> io::Result<Fd> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::other("path contains a NUL byte"))?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Fd::from_raw(fd))
}

pub fn write_all(fd: RawFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let rc = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        if rc < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if rc == 0 {
            return Err(io::Error::other("zero-length write"));
        }
        bytes = &bytes[rc as usize..];
    }
    Ok(())
}

pub fn pread(fd: RawFd, buffer: &mut [u8]) -> io::Result<usize> {
    let rc = unsafe {
        libc::pread(
            fd,
            buffer.as_mut_ptr() as *mut libc::c_void,
            buffer.len(),
            0,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(rc as usize)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::unix::io::RawFd;
    use std::time::Instant;

    use super::{Fd, ReadyEvent};

    pub fn epoll_create() -> io::Result<Fd> {
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Fd::from_raw(fd))
    }

    pub fn epoll_add(epoll_fd: RawFd, fd: RawFd, token: u64, events: u32) -> io::Result<()> {
        let mut event = libc::epoll_event { events, u64: token };
        let rc = unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut event) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn epoll_wait(
        epoll_fd: RawFd,
        ready: &mut [ReadyEvent],
        timeout_ms: i32,
    ) -> io::Result<usize> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 32];
        let limit = events.len().min(ready.len());
        loop {
            let rc = unsafe {
                libc::epoll_wait(
                    epoll_fd,
                    events.as_mut_ptr(),
                    limit as libc::c_int,
                    timeout_ms,
                )
            };
            if rc >= 0 {
                let count = rc as usize;
                for (slot, event) in ready.iter_mut().zip(events.iter()).take(count) {
                    *slot = ReadyEvent {
                        token: event.u64,
                        events: event.events,
                    };
                }
                return Ok(count);
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
    }

    pub fn timerfd_create() -> io::Result<Fd> {
        let fd = unsafe {
            libc::timerfd_create(
                libc::CLOCK_MONOTONIC,
                libc::TFD_NONBLOCK | libc::TFD_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Fd::from_raw(fd))
    }

    pub fn timerfd_arm(fd: RawFd, deadline: Instant) -> io::Result<()> {
        let wait = deadline.saturating_duration_since(Instant::now());
        let mut tv_nsec = wait.subsec_nanos() as libc::c_long;
        if wait.is_zero() {
            tv_nsec = 1;
        }
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let value = libc::timespec {
            tv_sec: wait.as_secs() as libc::time_t,
            tv_nsec,
        };
        let spec = libc::itimerspec {
            it_interval: zero,
            it_value: value,
        };
        let rc = unsafe { libc::timerfd_settime(fd, 0, &spec, std::ptr::null_mut()) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn timerfd_drain(fd: RawFd) -> io::Result<()> {
        let mut value = [0u8; 8];
        let rc = unsafe { libc::read(fd, value.as_mut_ptr() as *mut libc::c_void, value.len()) };
        if rc < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn signalfd_create(signals: &[libc::c_int]) -> io::Result<Fd> {
        let mut set = unsafe { std::mem::zeroed::<libc::sigset_t>() };
        unsafe {
            libc::sigemptyset(&mut set);
        }
        for signal in signals {
            unsafe {
                libc::sigaddset(&mut set, *signal);
            }
        }
        let fd = unsafe { libc::signalfd(-1, &set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Fd::from_raw(fd))
    }

    pub fn read_signal(fd: RawFd) -> io::Result<i32> {
        let mut info = MaybeUninit::<libc::signalfd_siginfo>::uninit();
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        let rc = unsafe { libc::read(fd, info.as_mut_ptr() as *mut libc::c_void, size) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if rc as usize != size {
            return Err(io::Error::other("short signalfd read"));
        }
        Ok(unsafe { info.assume_init() }.ssi_signo as i32)
    }

    pub fn inotify_probe() -> bool {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return false;
        }
        unsafe {
            libc::close(fd);
        }
        true
    }

    #[cfg(test)]
    pub fn pipe() -> io::Result<(Fd, Fd)> {
        let mut fds = [0; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((Fd::from_raw(fds[0]), Fd::from_raw(fds[1])))
    }

    #[cfg(test)]
    pub fn write_fd(fd: RawFd, byte: u8) -> io::Result<()> {
        super::write_all(fd, &[byte])
    }

    #[cfg(test)]
    pub fn read_fd(fd: RawFd, buffer: &mut [u8]) -> io::Result<usize> {
        let rc = unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(rc as usize)
    }
}

#[cfg(target_os = "linux")]
pub use linux::{
    epoll_add, epoll_create, epoll_wait, inotify_probe, read_signal, signalfd_create, timerfd_arm,
    timerfd_create, timerfd_drain,
};

#[cfg(target_os = "linux")]
#[cfg(test)]
pub use linux::{pipe, read_fd, write_fd};

#[cfg(not(target_os = "linux"))]
fn unsupported<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this capability requires Linux",
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_create() -> io::Result<Fd> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_add(_epoll_fd: RawFd, _fd: RawFd, _token: u64, _events: u32) -> io::Result<()> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_wait(
    _epoll_fd: RawFd,
    _ready: &mut [ReadyEvent],
    _timeout_ms: i32,
) -> io::Result<usize> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn timerfd_create() -> io::Result<Fd> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn timerfd_arm(_fd: RawFd, _deadline: Instant) -> io::Result<()> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn timerfd_drain(_fd: RawFd) -> io::Result<()> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn signalfd_create(_signals: &[libc::c_int]) -> io::Result<Fd> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn read_signal(_fd: RawFd) -> io::Result<i32> {
    unsupported()
}

#[cfg(not(target_os = "linux"))]
pub fn inotify_probe() -> bool {
    false
}
