# Implementation plan: network JIT gate and documentation

## Checklist

1. [ ] Add `adapters/netstat.rs` with `NetOps`, interface counter reading, `TrafficLedger`, and the estimator.
2. [ ] Add `TrafficLedger`; update the network worker to record bytes read and written.
3. [ ] Implement `ExternalTrafficEstimator` with saturating math and the kbps conversion.
4. [ ] Add `domain::should_start_burst(ingress_kbps, egress_kbps, threshold)` and wire the gate into the worker.
5. [ ] Add `external_traffic_kbps` and `probe_window_ms` to config, defaults, validation, and `default_config_text`.
6. [ ] Add estimator, gate, counter-reader, and config round-trip tests.
7. [ ] Rewrite README per the documentation contract; record the spec changes in `spec-delta.md`; do not edit `.phronia/spec/**`.
8. [ ] Verify `print-default-config` parses with the new keys.
9. [ ] Run the manual saturation test (Linux VM; not available on the development host).

## Validation

```bash
cargo fmt --check
cargo check
cargo test

./bin/useless print-default-config > /tmp/useless-default.toml
./bin/useless status --config /tmp/useless-default.toml

grep -rn "external_traffic_kbps\|probe_window_ms" README.md .phronia/spec/backend/
```

Manual saturation test on the VM:

```bash
# background download that exceeds the configured threshold
curl -o /dev/null http://mirror.example/10gb.bin &
# observe the journal; useless must skip network bursts while the download runs
tail -f /run/useless/journal.jsonl
# stop the download; bursts resume
```

## Risky files

* `src/workers/network.rs` — the probe sleep must stay in the worker and never move onto the reactor.
* `src/config.rs` — unit conversion and validation; keep the old keys untouched.
* `README.md` and the contract spec — stale docs are a correctness defect for operators.

## Rollback point

One commit. Revert removes the gate and the doc changes; no config migration is required because the new keys are optional.
