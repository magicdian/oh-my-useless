# Implementation plan: cgroup scopes, headroom budget, and memory ledger

## Checklist

1. [ ] Add `reserve_mb`, `reserve_percent`, `max_absolute_mb`, `chunk_mb`, and `[sensor].watch_slices` to `config.rs`; update defaults, `default_config_text`, and `validate_config`.
2. [ ] Implement `domain::budget` with the table tests from AC3.2.
3. [ ] Implement `adapters/cgroups.rs::discover` and the 60 s/SIGHUP refresh; exclude the daemon's own cgroup.
4. [ ] Wire per-slice `*.pressure` watches through the child 2 PSI adapter via `PsiSpec`; report the cgroup scope in the summary and the state file.
5. [ ] Replace the memory worker with the `mmap` ledger; implement the fresh `can_grow` closure, chunked growth, `madvise` shrink, and zero-on-suspend.
6. [ ] Implement the ledger invariant and release tests with scripted headroom.
7. [ ] Derive `MemoryHigh`/`MemoryMax` in `install.rs`; add `OOMScoreAdjust=1000`, `IOSchedulingClass=idle`, `CPUWeight=10`, `Restart=on-failure`, and start limits.
8. [ ] Extend `status` with budget, headroom, reserve, primary scope, and cgroup health.
9. [ ] Add the unit-generation snapshot test and the scope-discovery fake-directory test.
10. [ ] Record the config keys, unit guarantees, state fields, and tests in `spec-delta.md`; do not edit `.phronia/spec/**`. README updates belong to child 5.

## Validation

```bash
cargo fmt --check
cargo check
cargo test
cargo check --tests --target x86_64-unknown-linux-gnu

# unit contents
systemctl show useless -p OOMScoreAdjust -p MemoryHigh -p MemoryMax -p IOSchedulingClass

# live ledger vs headroom
useless status
grep -E 'MemAvailable|MemTotal' /proc/meminfo

# no large resizes remain
grep -rn "resize(" src/workers/
```

VM soak (required before completion):

1. 2 vCPU / 2 GiB, no swap, K3s-like baseline.
2. Hold ~500 MB with a hoarder; run a reconcile loop that allocates and frees 100-200 MB periodically.
3. Verify `useless` stays below `MemAvailable − reserve`, releases on pressure, and never appears in an OOM line.
4. `dmesg -T | grep -E 'invoked oom-killer|Out of memory'` must be empty for `useless`.
5. Check `journal.jsonl` for the scope that triggered suspension.

## Risky files

* `src/workers/memory.rs` — the allocator rewrite; every path must release on `Drop` and on zero directives.
* `src/install.rs` — unit text is an operator contract; keep a snapshot test.
* `src/config.rs` — backward compatibility; parse the old config in a test.
* `src/adapters/cgroups.rs` — path assumptions vary by distro; keep discovery tolerant.

## Rollback point

One commit. Reverting requires rerunning `useless install` to restore the old unit text. If the fix is withdrawn, set `[memory] enabled=false` on multi-workload hosts and document it.
