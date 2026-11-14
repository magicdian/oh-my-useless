use std::io;
use std::sync::Arc;
use std::time::Duration;

use crate::runtime::Store;

const PAGE: usize = 4096;

struct Chunk {
    ptr: *mut u8,
    len: usize,
}

/// Anonymous-memory ledger. Growth is one chunk at a time and always gated by
/// a fresh headroom check; release is per chunk with `MADV_DONTNEED` + `munmap`.
pub struct Ledger {
    chunks: Vec<Chunk>,
    current: u64,
    chunk_bytes: usize,
    warned: bool,
}

impl Ledger {
    pub fn new(chunk_bytes: usize) -> Self {
        Self {
            chunks: Vec::new(),
            current: 0,
            chunk_bytes: chunk_bytes.max(PAGE),
            warned: false,
        }
    }

    #[allow(dead_code)]
    pub fn current(&self) -> u64 {
        self.current
    }

    pub fn set_chunk_bytes(&mut self, chunk_bytes: usize) {
        self.chunk_bytes = chunk_bytes.max(PAGE);
    }

    /// Grows toward `target` while `can_grow(current, chunk)` allows, then
    /// shrinks to `target`. Returns the resulting resident size.
    pub fn reconcile<F>(&mut self, target: u64, mut can_grow: F) -> u64
    where
        F: FnMut(u64, u64) -> bool,
    {
        while self.current < target {
            let chunk = (target - self.current).min(self.chunk_bytes as u64) as usize;
            if chunk == 0 || !can_grow(self.current, chunk as u64) {
                break;
            }
            match self.allocate(chunk) {
                Ok(()) => {}
                Err(error) => {
                    if !self.warned {
                        eprintln!("[useless] memory ledger mmap failed: {error}");
                        self.warned = true;
                    }
                    break;
                }
            }
        }

        while self.current > target {
            if !self.release_last() {
                break;
            }
        }

        self.current
    }

    pub fn release_all(&mut self) {
        while self.release_last() {}
    }

    fn allocate(&mut self, len: usize) -> io::Result<()> {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        let ptr = ptr as *mut u8;
        for offset in (0..len).step_by(PAGE) {
            unsafe {
                ptr.add(offset).write_volatile(0);
            }
        }

        self.chunks.push(Chunk { ptr, len });
        self.current += len as u64;
        Ok(())
    }

    fn release_last(&mut self) -> bool {
        let Some(chunk) = self.chunks.pop() else {
            return false;
        };
        unsafe {
            libc::madvise(
                chunk.ptr as *mut libc::c_void,
                chunk.len,
                libc::MADV_DONTNEED,
            );
            libc::munmap(chunk.ptr as *mut libc::c_void, chunk.len);
        }
        self.current = self.current.saturating_sub(chunk.len as u64);
        true
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        self.release_all();
    }
}

pub fn run(store: Arc<Store>) {
    let mut ledger = Ledger::new(16 * 1024 * 1024);
    let mut seen = 0u64;

    loop {
        let state = store.wait_for_change(seen, Duration::from_secs(1));
        seen = state.generation;
        if state.shutdown {
            break;
        }

        ledger.set_chunk_bytes((state.config.memory.chunk_mb * 1024 * 1024) as usize);

        if !state.directives.memory.allowed {
            ledger.release_all();
            continue;
        }

        let target = state.directives.budget.target_bytes;
        let reserve = state.directives.budget.reserve_bytes;
        ledger.reconcile(target, |current, pending| {
            let available = read_mem_available_bytes().unwrap_or(0);
            available.saturating_sub(reserve) >= current + pending
        });
    }

    ledger.release_all();
}

fn read_mem_available_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = content
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

#[cfg(test)]
mod tests {
    use super::Ledger;

    const CHUNK: usize = 4096 * 4;

    #[test]
    fn grows_only_while_headroom_allows() {
        let mut ledger = Ledger::new(CHUNK);
        let target = (CHUNK * 3) as u64;

        let current = ledger.reconcile(target, |current, pending| {
            current + pending <= (CHUNK * 2) as u64
        });

        assert_eq!(current, (CHUNK * 2) as u64);
        assert_eq!(ledger.current(), (CHUNK * 2) as u64);
    }

    #[test]
    fn shrinks_to_the_requested_target() {
        let mut ledger = Ledger::new(CHUNK);
        let target = (CHUNK * 3) as u64;
        ledger.reconcile(target, |_, _| true);
        assert_eq!(ledger.current(), target);

        let current = ledger.reconcile(CHUNK as u64, |_, _| true);
        assert_eq!(current, CHUNK as u64);
    }

    #[test]
    fn zero_target_releases_everything() {
        let mut ledger = Ledger::new(CHUNK);
        ledger.reconcile((CHUNK * 2) as u64, |_, _| true);
        assert!(ledger.current() > 0);

        ledger.reconcile(0, |_, _| true);
        assert_eq!(ledger.current(), 0);
    }
}
