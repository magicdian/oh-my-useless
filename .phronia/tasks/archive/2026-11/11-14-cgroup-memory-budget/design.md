# Design: cgroup scopes, headroom budget, and memory ledger

Shared architecture lives in `../11-14-event-driven-sensor-architecture/design.md`. This file records the budget, ledger, scope discovery, and unit derivation.

## 1. Scope discovery (`src/adapters/cgroups.rs`)

```rust
pub struct ScopeSet { pub watched: Vec<PathBuf>, pub missing: Vec<String> }

pub fn discover(spec: &[String], cgroup_root: &Path, own: &Path) -> ScopeSet;
```

* `own` comes from the `0::<path>` line of `/proc/self/cgroup`, resolved against `/sys/fs/cgroup`.
* Enumerate direct children of `/sys/fs/cgroup` whose names are in `spec`. Do not recurse; per-container pressure is aggregated by the slice controller.
* Exclude a path when it equals `own` or is an ancestor of `own`. Because PSI is hierarchical, watching `system.slice` would include the daemon's own pressure; that slice is therefore opt-in and the README says so.
* Refresh on a 60 s timer and on `SIGHUP`: add fds for new paths, close fds for removed paths, emit `SourceHealth` for a path that disappeared. Never fail the reactor because a slice vanished.

## 2. Budget (`src/domain/budget.rs`)

```rust
pub struct BudgetInput {
    pub mem_total: u64,
    pub mem_available: u64,
    pub max_percent: u8,
    pub duty_percent: u8,
    pub max_absolute_mb: u64,
    pub reserve_mb: u64,
    pub reserve_percent: u8,
}

pub fn budget(i: &BudgetInput) -> Budget {
    let reserve = (i.mem_total * i.reserve_percent as u64 / 100).max(i.reserve_mb << 20);
    let ratio = i.mem_total * i.max_percent as u64 / 100 * i.duty_percent as u64 / 100;
    let headroom = i.mem_available.saturating_sub(reserve);
    let absolute = if i.max_absolute_mb == 0 { u64::MAX } else { i.max_absolute_mb << 20 };
    let target = ratio.min(headroom).min(absolute);
    Budget { target, reserve, headroom }
}
```

All arithmetic uses `u64` with saturating operations; no floats. `duty_percent` is applied to the ratio cap only, so `Caution` shrinks the synthetic target without changing the reserve.

## 3. Ledger (`src/workers/memory.rs`)

```rust
struct Chunk { ptr: NonNull<u8>, len: usize }
pub struct Ledger { chunks: Vec<Chunk>, chunk_bytes: usize }

impl Ledger {
    pub fn reconcile<F>(&mut self, target: u64, mut can_grow: F)
    where F: FnMut(u64, u64) -> bool;
}
```

Growth protocol per chunk:

1. Ask `can_grow(current, chunk_bytes)`; the closure performs a **fresh** read of `MemAvailable`. It never uses the reducer's cached sample. PSI pressure is enforced by the directive gate: `Suspended` sets `allowed=false`, and the worker releases before the next growth pass.
2. If it returns false, stop; the ledger stays below the requested target.
3. `mmap(NULL, chunk, PROT_READ|WRITE, MAP_PRIVATE|ANONYMOUS|MAP_NORESERVE)`; write one byte per 4096-byte page to make the pages resident.
4. On `mmap` failure, log once, keep the current chunks, and stop growth; the next directive generation retries at the configured chunk size; never panic.

Shrink: newest chunk first; `madvise(MADV_DONTNEED)`, then `munmap`. `Suspended` reconciles to zero in one pass. `Drop` releases every chunk.

The closure is the safety invariant: no code path may grow the ledger without it. The unit tests inject a scripted closure to prove the invariant independent of real memory.

## 4. Reducer integration

* A `Pressure` signal with `scope = Cgroup("kubepods.slice")` enters `Suspended` with reason `MemoryPressure` or `IoPressure`; the state file writes `scope=kubepods.slice` and the summary appends `scope=kubepods.slice`.
* `Sample` carries `mem_available_bytes` and `mem_total_bytes`; the reducer computes the budget and publishes it in `Directives::memory`.
* `Cooling` uses `caution_duty_percent` on the ratio cap; `Suspended` publishes zero and the worker releases immediately.
* Slow start scales the effective `duty_percent` through 25/50/100 across `resume_ramp_secs`; the budget formula itself is unchanged.
* The reason set keeps host and cgroup scopes separate so `status` can print `memory_pressure (kubepods.slice)`.

## 5. Configuration additions

| Key | Default | Validation |
|---|---|---|
| `reserve_mb` | `512` | any `u64` |
| `reserve_percent` | `10` | `< 100` |
| `[memory].max_absolute_mb` | `0` | `0` disables |
| `[memory].chunk_mb` | `16` | `> 0` |
| `[sensor].watch_slices` | `["kubepods.slice", "user.slice"]` | may be empty |

`config.rs` gains the keys in `GlobalConfig`, `MemoryConfig`, and a new `SensorConfig`; `default_config_text` and `validate_config` are updated together. Unknown-key errors keep their current behavior.

## 6. Unit derivation (`src/install.rs`)

```text
ceiling_mb = min(MemTotal * memory.max_percent / 100,
                 memory.max_absolute_mb if > 0 else infinity)
MemoryHigh = ceiling_mb + 64
MemoryMax  = ceiling_mb + 128
```

`ceiling_mb` is clamped to at least 64 MB so a disabled or tiny memory module still yields a valid unit. `OOMScoreAdjust=1000` makes the kernel prefer the daemon. `MemoryHigh` throttles before `MemoryMax` kills, and both are inside the unit's own cgroup, so the business pods are never the first sacrifice.

`install` regenerates the unit from the current config; `reload` does not. `status` prints the live `budget_mb`, `reserve_mb`, `available_mb`, and cgroup `scope` from the state file, so the operator can compare them with the unit limits and spot a stale unit.

## 7. Tests

* Budget table with the six cases from the PRD.
* Ledger invariant with a scripted `can_grow(current, chunk)` sequence, including a false result mid-growth.
* Release-on-zero test.
* Unit generation snapshot for `MemTotal = 2048 MB`, `max_percent = 15`, `max_absolute_mb = 0`; and a second case with `max_absolute_mb = 128`.
* Config parse/round-trip and validation failures.
* Scope discovery with a fake directory tree and a fake `/proc/self/cgroup` reader.

## 8. Rollback

Revert the child commit. If the incident fix must be withdrawn, also set `[memory] enabled=false` on multi-workload hosts until a replacement lands; record this in the deployment notes.
