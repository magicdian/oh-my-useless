# feat: network JIT gate and architecture docs

## Goal

Stop competing with business traffic on small links. Before each network burst, estimate external throughput with a just-in-time counter probe that subtracts the daemon's own bytes, and skip the burst when real traffic is present. Finish by making the architecture, config surface, and operational behavior legible in README and specs.

## Background

* `src/workers/network.rs` downloads from public mirrors at `max_mbps` with no awareness of other traffic.
* `src/sensor.rs:147-169` already reads interface counters and computes rx/tx rates, but the policy never uses them.
* On the incident host, `max_mbps = 4` on a small cloud instance is a material share of the uplink; a burst can crowd out kubelet, image pulls, or controller traffic.
* Child 1 provides the ports/reducer; child 3 provides the headroom/JIT pattern this child mirrors for network.

## Requirements

* R5.1 `adapters/netstat.rs` exposes interface totals for up interfaces excluding `lo`, reusing the existing filtering logic.
* R5.2 `TrafficLedger` records the daemon's own rx/tx bytes atomically; the network worker updates it with the bytes it actually reads and writes.
* R5.3 `ExternalTrafficEstimator` reads counters twice around `probe_window_ms`, subtracts the ledger delta, and returns `(ingress_kbps, egress_kbps)`. The conversion is bytes/sec × 8 / 1000; the unit is kilobits per second and is documented.
* R5.4 Before a burst, the network worker runs the estimate. If either direction exceeds `external_traffic_kbps`, it skips the burst, emits `Signal::ExternalTraffic`, and retries on the next schedule. The probe runs in the worker thread and is bounded by `probe_window_ms ≤ 2000`.
* R5.5 The estimate must not self-trigger: own bytes during the probe are subtracted, and a test with `own == total` yields zero external traffic.
* R5.6 `max_mbps` remains the hard cap; the gate only decides whether to start.
* R5.7 If counters are unreadable, the gate is open (network load is not the OOM risk), a health note is logged once, and the burst proceeds under `max_mbps`.
* R5.8 README and specs are updated: architecture overview, new config keys, journal/state files, `status` fields, capability degradation, memory safety model, and the session-source order. `print-default-config` includes every new key.

## Acceptance criteria

* [ ] AC5.1 `cargo fmt --check`, `cargo check`, `cargo test` pass.
* [ ] AC5.2 Estimator tests cover: no external traffic, external ingress only, external egress only, own-equals-total, missing counters, and counter wrap/negative delta clamped to zero.
* [ ] AC5.3 Gate tests show a burst skipped when the estimate exceeds the threshold and started when it is below.
* [ ] AC5.4 During a normal burst, the gate does not self-trigger; `journal.jsonl` shows no `external_traffic` transition caused by the daemon's own download.
* [ ] AC5.5 `print-default-config` round-trips through `parse_config` and includes `external_traffic_kbps` and `probe_window_ms`.
* [ ] AC5.6 README and `.phronia/spec/backend/daemon-install-contracts.md` describe every config key, the state/journal paths, the status fields, and the degradation rules; a doc review finds no stale key names.
* [ ] AC5.7 Manual test: saturate the uplink with an external download; `useless` skips bursts and resumes after it ends.

## Out of scope

* Per-socket or per-cgroup network attribution.
* eBPF, `tc` integration, packet capture.
* Changing the default target list.

## Open questions

None.
