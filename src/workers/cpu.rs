use std::hint::spin_loop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::runtime::SharedRuntime;

pub fn run(shared: SharedRuntime) {
    let cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .max(1);

    let thread_duties = Arc::new(
        (0..cores)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<AtomicU32>>(),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let mut burners = Vec::new();

    for index in 0..cores {
        let duties = Arc::clone(&thread_duties);
        let stop_flag = Arc::clone(&stop);
        burners.push(
            thread::Builder::new()
                .name(format!("useless-cpu-burn-{index}"))
                .spawn(move || burn_loop(index, duties, stop_flag))
                .expect("spawn cpu burner"),
        );
    }

    while !should_stop(&shared) {
        let (allowed, max_percent, duty_percent) = {
            let state = shared.read().expect("cpu worker read lock");
            (
                state.policy.cpu.allowed,
                state.config.cpu.max_percent,
                state.policy.cpu.duty_percent,
            )
        };

        let target_units = if allowed {
            ((cores as u64) * (max_percent as u64) * (duty_percent as u64) * 10 / 100)
                .min((cores as u64) * 1000) as u32
        } else {
            0
        };
        apply_target_units(&thread_duties, target_units);

        thread::sleep(Duration::from_secs(1));
    }

    stop.store(true, Ordering::Relaxed);
    apply_target_units(&thread_duties, 0);
    for burner in burners {
        let _ = burner.join();
    }
}

fn burn_loop(index: usize, duties: Arc<Vec<AtomicU32>>, stop: Arc<AtomicBool>) {
    let cycle = Duration::from_millis(100);
    while !stop.load(Ordering::Relaxed) {
        let duty = duties[index].load(Ordering::Relaxed).min(1000);
        if duty == 0 {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        let busy_millis = (cycle.as_millis() as u32 * duty / 1000) as u64;
        let busy_for = Duration::from_millis(busy_millis.min(100));
        let busy_until = Instant::now() + busy_for;
        while Instant::now() < busy_until {
            spin_loop();
        }

        let rest = cycle.saturating_sub(busy_for);
        if !rest.is_zero() {
            thread::sleep(rest);
        }
    }
}

fn apply_target_units(duties: &[AtomicU32], mut target_units: u32) {
    for duty in duties {
        let assigned = target_units.min(1000);
        duty.store(assigned, Ordering::Relaxed);
        target_units = target_units.saturating_sub(assigned);
    }
}

fn should_stop(shared: &SharedRuntime) -> bool {
    shared.read().expect("cpu worker read lock").shutdown
}
