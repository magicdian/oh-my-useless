# Implementation plan: pure policy domain and ports

## Checklist

1. [ ] Create `src/domain/` with `signal.rs`, `state.rs`, `budget.rs`, `schedule.rs`, and `mod.rs`; move types from `src/policy.rs` and `src/sensor.rs` without changing semantics.
2. [ ] Implement `reducer.rs::reduce` from the five parity rules in `design.md`; move the phase-flip fix and mark it with a test.
3. [ ] Create `src/ports.rs` with the five traits; keep them minimal and object-safe where the reactor needs `Box<dyn>`.
4. [ ] Create `src/runtime/store.rs` and `src/runtime/journal.rs`; delete `src/runtime.rs` after migration.
5. [ ] Move `src/sensor.rs` to `src/adapters/procfs.rs`; implement `Sampler`; keep every parser and its tests verbatim.
6. [ ] Rewrite `src/daemon.rs` as the composition root: pump at `check_interval_secs`, emit signals, call `reduce`, publish directives, journal transitions.
7. [ ] Update `src/workers/*.rs` to wait on `Store` generation; keep the 1 s safety timeout.
8. [ ] Add the behavior-parity fixture and the table-driven tests.
9. [ ] Delete `src/policy.rs` and `src/sensor.rs`; run `grep` checks from AC1.4/AC1.5.
10. [ ] Record the spec changes in `spec-delta.md` (layout, contract paths, test locations). Do not edit `.phronia/spec/**` from this code task; the main session applies the delta after integration.

## Validation

```bash
cargo fmt --check
cargo check
cargo test
./bin/useless print-default-config   # must be unchanged
```

Manual smoke test on the development host (Linux required for `/proc`):

```bash
sudo ./bin/useless daemon --config /tmp/useless-test.toml
# in another shell, create an active tty or raise load; observe mode transitions in stderr
```

## Risky files

* `src/daemon.rs` — the composition root is rewritten; keep a single `reduce` call site.
* `src/adapters/procfs.rs` — a verbatim move is required; do not "improve" parsing in this child.
* `src/workers/*.rs` — the waiting mechanism changes but targets must not.

## Rollback point

Single commit. Revert restores the pre-refactor files; no config or deployment migration.
