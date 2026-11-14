# Spec Delta

## Target files
- `.phronia/spec/backend/directory-structure.md`
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/index.md`
- `.phronia/spec/backend/quality-guidelines.md`
- `.phronia/spec/backend/error-handling.md`
- `.phronia/spec/frontend/state-management.md`
- `.phronia/spec/frontend/type-safety.md`

## Intended changes

### `directory-structure.md`

- Replace the `src/` tree with the new layout: `daemon.rs` as the composition root; `ports.rs`; `domain/` (`signal`, `state`, `reducer`, `budget`, `schedule`); `runtime/` (`store`, `journal`); `adapters/` (`procfs`); unchanged `workers/`.
- Add **Rule: Respect the layer direction** after the one-file rule: daemon may call all layers; adapters emit `Signal` but never call `reduce`; workers apply directives but never decide policy; domain may depend on config types, never on IO.
- Update the shared-types examples from `runtime.rs` / `policy.rs` to `runtime/store.rs` and `domain/state.rs`.
- Add forbidden patterns: adapters or workers deciding policy; `domain/` reading the clock or filesystem.

### `daemon-install-contracts.md`

- Add `Policy transitions in src/domain/ or observation sources in src/adapters/` to the Scope/Trigger list.
- Under **Runtime behavior**, add:
  - Policy evaluation is pure: `domain::reduce(state, signal, config, now)` performs no IO and reads no clock.
  - Adapters emit `Signal` values and never call the reducer; workers consume directives from `runtime::Store` and never compute policy.
  - `src/adapters/procfs.rs` is the current observation source and the permanent fallback.
- Update the unit-test expectations: sensor tests live in `src/adapters/procfs.rs`; policy tests (including the legacy parity table) live in `src/domain/reducer.rs`.

### `index.md`

- `Daemon runtime` row → `src/daemon.rs`, `src/runtime/`.
- `Scheduling` row → `src/domain/`, `src/adapters/`.

### `quality-guidelines.md`

- Replace the `src/policy.rs` test example with the behavior-parity tests in `src/domain/reducer.rs`.

### `error-handling.md`

- Replace the `src/sensor.rs::sample` example with `src/adapters/procfs.rs::sample`.

### `frontend/state-management.md`

- Runtime state path → `src/runtime/store.rs`.
- Scheduling and pressure state → `src/domain/state.rs`.

### `frontend/type-safety.md`

- Policy outputs → `src/domain/state.rs`.
- Sensor samples → `src/domain/signal.rs`.

## Rationale

Child 1 moved policy into a pure domain layer, observation into adapters, and shared directives into a generation-based store. The specs still describe `src/policy.rs`, `src/sensor.rs`, and `src/runtime.rs`, which would mislead child 2 and future maintenance. No config key, CLI, or systemd contract changed, so the delta is limited to paths and layer rules.

## Verification

- After applying: `grep -rn "src/policy.rs\|src/sensor.rs\|src/runtime.rs" .phronia/spec/` returns nothing.
- The layer rules match the code: `grep -rn "reduce(" src/adapters src/workers src/runtime` returns nothing; `grep -rn "Instant::now\|read_to_string\|Command::new" src/domain/` only hits `#[cfg(test)]` modules.
- `cargo fmt --check`, `cargo check`, `cargo test` are unaffected because the delta is documentation only.
