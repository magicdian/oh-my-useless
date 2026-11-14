# Design: pure policy domain and ports

Shared architecture lives in `../11-14-event-driven-sensor-architecture/design.md`. This file records only what child 1 introduces.

## 1. Module layout

```text
src/
  domain/
    mod.rs          re-exports the public contract
    signal.rs       Signal, SampleSnapshot, SessionSet
    state.rs        Mode, ReasonCode, PolicyState, PolicyOutput
    reducer.rs      reduce(state, signal, config, now) -> PolicyOutput
    budget.rs       compute(&BudgetInput) -> Budget
    schedule.rs     BurstScheduler with injected seed
  ports.rs          EventSource, Sampler, Clock, StatePublisher, JournalSink
  runtime/
    mod.rs
    store.rs        Store { Mutex<StoreState>, Condvar }
    journal.rs      ring buffer + Transition struct
  adapters/
    mod.rs
    procfs.rs       current sensor.rs, moved verbatim, implements Sampler
  daemon.rs         composition root
```

`src/policy.rs`, `src/runtime.rs`, and `src/sensor.rs` are removed after their contents move. No compatibility shim is kept; the compiler enforces the new boundaries.

## 2. Types

```rust
pub enum Signal {
    Sample(SampleSnapshot),
    Sessions(SessionSet),
    SourceDegraded { source: String, message: String },
}

pub struct SampleSnapshot {
    pub load_percent: u8,
    pub mem_available_percent: u8,
    pub mem_available_bytes: u64,
    pub mem_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
}

pub enum Mode { Running, Cooling, Suspended }

pub enum ReasonCode {
    CollectorDegraded,
    LoadPressure,
    MemoryPressure,
    InteractiveSession,
}
```

`ReasonCode` ordering is explicit (`Ord` by severity), so `status` can print one primary reason while the journal keeps all. Child 2 adds pressure and traffic reasons when their adapters land.

## 3. Reducer parity rules

Child 1 reproduces current semantics exactly:

1. If a session is active, or `now - last_interactive < interactive_grace_secs`, mode is `Suspended` with `InteractiveSession`.
2. Else if `load_percent >= pause_load_percent` or `mem_available_percent <= pause_mem_available_percent`, `Suspended` with the corresponding reason.
3. Else if `load_percent >= caution_load_percent` or `mem_available_percent <= caution_mem_available_percent`, `Cooling`.
4. Duty is `100` in `Running`, `caution_duty_percent` in `Cooling`, `0` in `Suspended`.
5. Each module is allowed only when enabled, duty > 0, and its burst schedule says `Burst`.
6. Burst schedules advance on `is_active(now, window)` with the same phase-flip logic as today.

The only structural change is that `is_active` is evaluated even when duty is zero, so phase no longer freezes during suspension. This is intentional and must be documented in the parity test as the single accepted divergence, because freezing caused an immediate full-size burst on resume.

## 4. Store contract

```rust
pub struct StoreState {
    pub config: Arc<Config>,
    pub directives: PolicyOutput,
    pub generation: u64,
    pub shutdown: bool,
}

pub struct Store { inner: Mutex<StoreState>, changed: Condvar }

impl Store {
    pub fn publish(&self, output: PolicyOutput) -> u64;   // increments generation, notifies
    pub fn wait_for_change(&self, seen: u64, timeout: Duration) -> StoreState;
    pub fn snapshot(&self) -> StoreState;
}
```

`Store` implements `StatePublisher` and `Journal` implements `JournalSink`, so the ports have real consumers. Traits with no consumer until child 2 (`EventSource`) carry `#[allow(dead_code)]` with a comment naming the consuming child.

Workers keep their own `seen` generation. The 1 s timeout is a safety net during this child; child 2 makes wake-up the normal path.

## 5. Journal

`Journal` holds `VecDeque<Transition>` capped at 512 entries. A `Transition` carries time, signal kind, from/to mode, primary reason, all reasons, budget bytes, available bytes, pressure summary. In child 1 the sink is memory-only; `status` does not read it yet.

## 6. Behavior-parity test

Keep the old decision logic in `src/domain/reducer.rs` under `#[cfg(test)]` as `legacy_decision(sample, config, interactive)`. The table contains at least 20 sample rows spanning both thresholds of every module, each row asserting mode plus all three `allowed`/`duty` directive pairs. Separate tests cover grace expiry and a rest-phase boundary. The one documented exception is the burst-phase divergence from rule 6 above; the parity table asserts the new behavior only for the resume path and the old behavior everywhere else.

## 7. Trade-offs

* Moving files instead of re-exporting creates one large diff, but removes the dual-path risk that would otherwise persist for the whole project.
* Keeping `SampleSnapshot` close to today's shape avoids a policy rewrite; pressure and traffic fields arrive with the adapters that produce them.
* No new config keys means the config parser is untouched and cannot regress here.

## 8. Accepted divergences

Two intentional differences from the pre-refactor daemon:

1. **Schedules advance while suspended.** The old code short-circuited `is_active` when duty was zero, freezing the phase and occasionally starting a full burst immediately on resume. The new reducer advances every schedule on every evaluation.
2. **Sensor failure logs once.** The degraded summary (`sensor backend failed: ...`) now flows through the normal "log on summary change" path. The old code paused silently. This matches the parent design's log-once health rule.

The budget is computed in this child but not applied: the memory worker keeps the legacy duty-based sizing so behavior stays identical. Child 3 consumes `PolicyOutput.budget` with the chunked ledger.

## 9. Rollback

Revert the child commit. Because behavior is preserved except for the documented burst-phase fix, no config or deployment change is required.
