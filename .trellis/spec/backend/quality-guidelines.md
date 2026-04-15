# Quality Guidelines

> Code quality standards for the current Rust backend.

---

## Overview

The current backend quality bar is pragmatic:

* keep the code std-only unless a dependency clearly buys real value,
* keep responsibilities separated by module,
* keep install/runtime/config contracts explicit,
* keep the CLI and daemon behavior testable with small unit tests and command checks.

Every backend task should preserve the ability to run:

```bash
cargo fmt
cargo check
cargo test
```

---

## Required Patterns

### Keep the crate entrypoint thin

`src/main.rs` should dispatch only. Subcommand parsing belongs in `src/cli.rs`; work happens in dedicated modules.

### Keep config as the single source of truth

Paths, defaults, presets, and validation rules should stay in `src/config.rs`.

Do not duplicate path constants in workers or installers.

### Keep service-manager integration centralized

systemd and init.d template generation belongs in `src/install.rs`, not in ad hoc string literals scattered across the codebase.

### Keep worker-specific behavior isolated

CPU, memory, and network execution logic stays under `src/workers/`, while shared policy and state live above that layer.

### Add tests when behavior is deterministic

Current examples:

* config parsing test in `src/config.rs`
* interactive-session policy test in `src/policy.rs`

If you add deterministic parsing, threshold, or scheduling logic, add or extend tests in the same module.

---

## Forbidden Patterns

### Do not add dependencies for convenience only

The crate currently has no external dependencies. Before adding one, check whether the need is real or whether std already covers it.

### Do not bypass root checks in install flows

Install and uninstall mutate system paths. The guard in `src/install.rs::ensure_root` is part of the contract.

### Do not duplicate config or policy constants

If CPU, memory, and network workers need shared values, read them from shared state or config instead of re-declaring them.

### Do not edit the legacy shell reference as a substitute for Rust work

`Oracle-server-keep-alive-script-main/` is a reference input. It is not where new product behavior should live by default.

---

## Testing Requirements

Minimum checks before considering backend work complete:

1. `cargo fmt`
2. `cargo check`
3. `cargo test`

Add higher-level manual checks when changing install or daemon behavior:

* `useless install`
* `useless status`
* `useless reload`
* `useless uninstall`

If a task changes service definitions, configuration shape, or worker behavior, note which manual checks were run.

---

## Review Checklist

* Is the change in the correct module, or did it leak into `main.rs`?
* Did config defaults, parser rules, and README stay in sync?
* Did install/systemd/init.d changes preserve root-only behavior?
* Did worker code stay resource-specific instead of growing a shared utility dump?
* Are failures propagated as `AppResult` instead of panics?
* Are new deterministic rules covered by unit tests?

---

## Repository Examples

* `src/cli.rs` is a good example of explicit input validation without dependencies.
* `src/config.rs` is a good example of centralizing defaults and validation.
* `src/install.rs` is a good example of keeping service-manager logic in one place.
* `src/workers/mod.rs` is a good example of a narrow orchestration module.
