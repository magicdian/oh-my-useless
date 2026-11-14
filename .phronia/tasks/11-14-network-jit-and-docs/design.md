# Design: network JIT gate and documentation

Shared architecture lives in `../11-14-event-driven-sensor-architecture/design.md`. This file records the traffic estimator and the documentation contract.

## 1. Counter reader (`src/adapters/netstat.rs`)

Move the existing `/sys/class/net` logic out of `sensor.rs`:

```rust
pub struct NetTotals { pub rx: u64, pub tx: u64 }
pub fn read_totals(root: &Path) -> io::Result<NetTotals>;
```

Include only interfaces whose `operstate` is `up`, skip `lo`. Sum rx/tx. A missing attribute on one interface is skipped, not fatal.

## 2. Own-traffic ledger

```rust
pub struct TrafficLedger { rx: AtomicU64, tx: AtomicU64 }

impl TrafficLedger {
    pub fn add_rx(&self, bytes: u64);
    pub fn add_tx(&self, bytes: u64);
    pub fn snapshot(&self) -> (u64, u64);
}
```

The network worker is the only writer; the estimator reads a snapshot before and after the probe. A burst updates the ledger after each read/write chunk so a probe that overlaps a draining socket still subtracts correctly.

## 3. Estimator

```rust
pub fn estimate(ops: &impl NetOps, ledger: &TrafficLedger, window: Duration)
    -> io::Result<ExternalTraffic>;

pub struct ExternalTraffic { pub ingress_kbps: u64, pub egress_kbps: u64 }
```

Algorithm:

1. `before = read_totals()`, `own_before = ledger.snapshot()`.
2. Sleep `window` (worker thread, `window ≤ 2 s`).
3. `after = read_totals()`, `own_after = ledger.snapshot()`.
4. `total_rx = after.rx - before.rx` (saturating), `own_rx = own_after.0 - own_before.0`, `external_rx = total_rx.saturating_sub(own_rx)`.
5. `ingress_kbps = external_rx × 8 / window_ms` (integer math; `window_ms = window.as_millis()`), the same for egress.

A counter that goes backwards (interface reset) clamps to zero. The function never returns a negative value.

## 4. Gate

In `workers/network.rs`, before selecting a target:

```text
if estimates.ingress_kbps > cfg.external_traffic_kbps
   || estimates.egress_kbps  > cfg.external_traffic_kbps {
       publish ExternalTraffic;
       sleep 1 s;            // bounded backoff, worker thread only
       continue;
}
```

Notes:

* The probe is just-in-time, not periodic. It runs at most once per burst attempt.
* The 1 s backoff is a worker-local sleep, not a reactor timer; it cannot block event handling.
* `max_mbps` still caps the burst. `duty_percent` still scales it.
* On counter failure, log once, set a health flag, and proceed under `max_mbps`.

## 5. Config additions

| Key | Default | Validation |
|---|---|---|
| `[network] external_traffic_kbps` | `1024` | any `u32`; `0` disables the gate |
| `[network] probe_window_ms` | `1000` | `1..=2000` |

The unit is **kilobits per second** and the default config text says so.

## 6. Documentation contract

README must cover, in this order:

1. What the daemon does and the safety model (it yields to real work; it is not a benchmark).
2. Architecture in one diagram: reactor → signals → reducer → store → workers.
3. Config reference, grouped by section, with defaults and units; call out `reserve_mb`/`reserve_percent`, `max_absolute_mb`, `chunk_mb`, `pressure_*`, `resume_*`, `watch_slices`, `external_traffic_kbps`.
4. Observability: `status` fields, `/run/useless/state.json`, `/run/useless/journal.jsonl`.
5. Degradation matrix: no PSI, no cgroup v2, no inotify.
6. Operational notes: `install` regenerates the unit; `reload` does not; `uninstall` preserves config.

`.phronia/spec/backend/daemon-install-contracts.md` must list every new key and status field, the state/journal paths, the source selection order, and the unit guarantees. `directory-structure.md` must show the new `domain/`, `ports`, `adapters/`, `runtime/` layout.

## 7. Tests

* `NetTotals` reader against a temporary directory tree with fake `operstate` and statistics files.
* Estimator table from AC5.2, including a synthetic clock to avoid real sleeps (inject a `sleep` closure or a `Clock`).
* Gate decision as a pure function in `domain` (`should_start_burst(estimate, threshold)`) so the worker stays thin.
* Config round-trip and validation.

## 8. Rollback

Revert the child commit; the network worker returns to the pre-change ungated behavior. Documentation changes revert with it. No migration needed.
