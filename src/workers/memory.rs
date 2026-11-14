use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime::Store;

pub fn run(store: Arc<Store>) {
    let mut buffer = Vec::<u8>::new();
    let mut last_touch = Instant::now();
    let mut seen = 0u64;

    loop {
        let state = store.wait_for_change(seen, Duration::from_secs(1));
        seen = state.generation;
        if state.shutdown {
            break;
        }

        let total_memory = total_memory_bytes().unwrap_or(0);
        // Child 1 keeps the legacy ratio-based sizing to preserve behavior.
        // Child 3 replaces this with the headroom-checked chunked ledger driven
        // by `state.directives.budget`.
        let target_size = if state.directives.memory.allowed && total_memory > 0 {
            total_memory
                .saturating_mul(state.config.memory.max_percent as u64)
                .saturating_mul(state.directives.memory.duty_percent as u64)
                / 10_000
        } else {
            0
        } as usize;

        resize_buffer(&mut buffer, target_size);

        if !buffer.is_empty() && last_touch.elapsed() >= Duration::from_secs(10) {
            touch_buffer(&mut buffer);
            last_touch = Instant::now();
        }
    }

    buffer.clear();
    buffer.shrink_to(0);
}

fn resize_buffer(buffer: &mut Vec<u8>, target_size: usize) {
    if target_size == 0 {
        buffer.clear();
        buffer.shrink_to(0);
        return;
    }

    if buffer.len() < target_size {
        buffer.resize(target_size, 0);
        touch_buffer(buffer);
    } else if buffer.len() > target_size {
        buffer.truncate(target_size);
        buffer.shrink_to(target_size);
    }
}

fn touch_buffer(buffer: &mut [u8]) {
    let page = 4096usize;
    for index in (0..buffer.len()).step_by(page) {
        buffer[index] = buffer[index].wrapping_add(1);
    }
}

fn total_memory_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = content.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}
