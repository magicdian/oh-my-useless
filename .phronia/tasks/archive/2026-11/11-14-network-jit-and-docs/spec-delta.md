# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`
- `.phronia/spec/backend/quality-guidelines.md`
- `.phronia/spec/backend/logging-guidelines.md`

## Intended changes

### `daemon-install-contracts.md`

- Add `[network].external_traffic_kbps` (default 1024; `0` disables the gate) and `[network].probe_window_ms` (default 1000; 1..=2000) to the supported config list.
- Add a validation row: `probe_window_ms` outside 1..=2000 fails.
- Under **Runtime behavior**, add:
  - Before each network burst, the worker samples interface rx/tx counters twice over `probe_window_ms`, subtracts its own bytes, and converts to kilobits per second.
  - If either direction exceeds `external_traffic_kbps`, the burst is skipped and the decision is logged; retry happens on the next schedule. No mode transition is involved.
  - If counters are unreadable, the gate is open, a single warning is logged, and `max_mbps` still caps the burst.
  - `max_mbps` remains the hard cap; the gate only decides whether to start.
- Unit-test expectations: counter reader with a fake `/sys/class/net` tree, estimator subtraction/own-equals-total/wrap, the pure `should_start_burst` table, and config parsing/round-trip for the new keys.

### `directory-structure.md`

- Add `src/adapters/netstat.rs` (interface counters, traffic ledger, estimator).
- Note that `src/workers/network.rs` owns the ledger/gate and never makes policy decisions beyond the pure gate.

### `quality-guidelines.md`

- Add network estimator/gate tests to the deterministic-test examples.
- Note that the gate decision must stay a pure function (`domain::should_start_burst`) so it can be table-tested.

### `logging-guidelines.md`

- Network gate: log each skipped burst with ingress/egress estimates, and log the counter-read failure once. The probe is bounded to `probe_window_ms` and runs in the worker thread, not the reactor.

## Rationale

Child 5 stops the synthetic download from competing with business traffic on small links. The gate is just-in-time and subtracts the daemon's own bytes, so it does not self-trigger. The specs must describe the new keys and the "skip, don't suspend" semantics; the old network description has neither.

## Verification

- After applying: `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo check --tests --target x86_64-unknown-linux-gnu` pass.
- `useless print-default-config` includes `external_traffic_kbps` and `probe_window_ms` and re-parses.
- On a VM, an external download above the threshold causes the daemon to log skips and stop bursts; stopping the download resumes them.
