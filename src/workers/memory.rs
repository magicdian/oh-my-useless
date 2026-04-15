use std::thread;
use std::time::{Duration, Instant};

use crate::runtime::SharedRuntime;

pub fn run(shared: SharedRuntime) {
    let mut buffer = Vec::<u8>::new();
    let mut last_touch = Instant::now();

    while !should_stop(&shared) {
        let (allowed, max_percent, duty_percent) = {
            let state = shared.read().expect("memory worker read lock");
            (
                state.policy.memory.allowed,
                state.config.memory.max_percent,
                state.policy.memory.duty_percent,
            )
        };

        let total_memory = total_memory_bytes().unwrap_or(0);
        let target_size = if allowed && total_memory > 0 {
            total_memory
                .saturating_mul(max_percent as u64)
                .saturating_mul(duty_percent as u64)
                / 10_000
        } else {
            0
        } as usize;

        resize_buffer(&mut buffer, target_size);

        if !buffer.is_empty() && last_touch.elapsed() >= Duration::from_secs(10) {
            touch_buffer(&mut buffer);
            last_touch = Instant::now();
        }

        thread::sleep(Duration::from_secs(1));
    }
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

fn should_stop(shared: &SharedRuntime) -> bool {
    shared.read().expect("memory worker read lock").shutdown
}
