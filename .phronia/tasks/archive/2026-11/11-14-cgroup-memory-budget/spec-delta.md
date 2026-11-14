# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`
- `.phronia/spec/backend/quality-guidelines.md`

## Intended changes

### `daemon-install-contracts.md`

- Add global fields `reserve_mb` (default 512) and `reserve_percent` (default 10).
- Add `[memory].max_absolute_mb` (default 0, disables the absolute cap) and `[memory].chunk_mb` (default 16).
- Add a `[sensor]` section with `watch_slices` (default `["kubepods.slice", "user.slice"]`).
- Add validation rows: `reserve_percent >= 100` fails; `memory.chunk_mb = 0` fails.
- Under **Runtime behavior**, add:
  - Memory budget is `min(MemTotal × max_percent × duty, MemAvailable − reserve, max_absolute_mb)`, where `reserve = max(reserve_mb, MemTotal × reserve_percent / 100)`.
  - The memory worker is a chunked `mmap` ledger: it grows one chunk at a time only when a fresh `MemAvailable − reserve >= current + chunk` check passes, shrinks with `MADV_DONTNEED` + `munmap`, and releases everything on suspension.
  - The daemon watches each configured cgroup slice's `{cpu,memory,io}.pressure`, excludes its own cgroup and ancestors, and refreshes the slice set every 60 s and on `SIGHUP`.
  - `/run/useless/state` gains `reserve_mb` and `scope` (the cgroup slice that triggered pressure; empty for host pressure).
- Under **Install behavior**, add the systemd self-containment contract:
  - `OOMScoreAdjust=1000`, `IOSchedulingClass=idle`, `CPUWeight=10`, `Nice=19`.
  - `MemoryHigh = ceiling + 64M`, `MemoryMax = ceiling + 128M`, where `ceiling = min(MemTotal × memory.max_percent / 100, memory.max_absolute_mb?)`, clamped to at least 64 MB; a disabled memory module yields `ceiling = 64`.
  - `Restart=on-failure`, `RestartSec=10`, `StartLimitIntervalSec=300`, `StartLimitBurst=5`.
  - `install` regenerates the unit from the current config; `reload` does not.
- Unit-test expectations: budget table, ledger headroom/shrink/release tests, cgroup discovery, cgroup-scope PSI emission, unit-generation snapshot, and the new config validation.

### `directory-structure.md`

- Add `src/adapters/cgroups.rs` (scope discovery and own-cgroup exclusion).
- Add `src/workers/memory.rs`'s `mmap` ledger as the expected memory execution shape; note that no large `Vec::resize` may remain.

### `quality-guidelines.md`

- Add the ledger invariant to the review checklist: a memory growth step must be rejected when `MemAvailable − reserve < current + chunk`, and the JIT check must read fresh `/proc/meminfo`, not a cached sample.
- Note that `install.rs` unit generation must stay a pure function so it can be snapshot-tested.

## Rationale

Child 3 fixes the 2026-11-13 OOM incident: the old worker sized memory from `MemTotal` and committed it in one `Vec::resize`, so a single burst exceeded available memory. The new budget is headroom-derived, the ledger grows in checked chunks, and the unit makes the daemon the first OOM casualty instead of the business pods. The specs must describe these guarantees, not the old ratio-only model.

## Verification

- After applying: `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo check --tests --target x86_64-unknown-linux-gnu` pass.
- `grep -rn "resize(" src/workers/` shows no large-buffer resize.
- `systemctl show useless -p OOMScoreAdjust -p MemoryHigh -p MemoryMax -p IOSchedulingClass` matches the derived values on a test VM.
- `useless status` shows `budget_mb`, `reserve_mb`, `available_mb`, and `scope` from the state file.
