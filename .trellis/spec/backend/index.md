# Backend Development Guidelines

> Backend conventions for the current `oh-my-useless` repository.

---

## Overview

This repository is currently a single Rust binary crate. The main product code lives under `src/` and implements:

* a CLI entrypoint,
* config parsing and defaults,
* daemon lifecycle management,
* host sensing and scheduling policy,
* resource workers for CPU, memory, and network load,
* installer logic for systemd and init.d.

There is also an imported shell reference under `Oracle-server-keep-alive-script-main/`, but new backend work should target the Rust implementation unless the task is explicitly about preserving or documenting the legacy shell scripts.

---

## Current Backend Surface

| Area | Current Source of Truth | Notes |
|------|--------------------------|-------|
| CLI | `src/main.rs`, `src/cli.rs` | Subcommands dispatch from `main.rs`; parsing lives in `cli.rs` |
| Config | `src/config.rs` | Defaults, parsing, validation, install paths |
| Daemon runtime | `src/daemon.rs`, `src/runtime.rs` | Signal handling, PID file, shared state |
| Scheduling | `src/policy.rs`, `src/sensor.rs` | Pressure detection, interactive-session protection |
| Resource execution | `src/workers/*.rs` | CPU, memory, network workers |
| Service install | `src/install.rs` | systemd first, init.d fallback |
| Packaging | `scripts/package.sh` | Produces `tar.gz` for release delivery |

---

## Pre-Development Checklist

Read these before changing backend code:

1. `directory-structure.md`
2. `daemon-install-contracts.md`
3. `quality-guidelines.md`
4. `error-handling.md`
5. `logging-guidelines.md`
6. `database-guidelines.md` only if the task introduces persistence or schema changes

Also read:

1. `.trellis/spec/guides/index.md`
2. `.trellis/spec/guides/cross-layer-thinking-guide.md` for config or service-manager changes
3. `.trellis/spec/guides/code-reuse-thinking-guide.md` before adding shared helpers or constants

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | Filled |
| [Daemon Install Contracts](./daemon-install-contracts.md) | CLI, config, service-manager, and runtime contracts | Filled |
| [Database Guidelines](./database-guidelines.md) | Persistence rules and current non-use of DB | Filled |
| [Error Handling](./error-handling.md) | Error types, propagation, and cleanup rules | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Build, testing, and review expectations | Filled |
| [Logging Guidelines](./logging-guidelines.md) | stdout vs stderr and daemon logging patterns | Filled |

---

## Repository Examples

* `src/main.rs` keeps the entrypoint thin and dispatch-only.
* `src/config.rs` is the single source of truth for config paths, defaults, and validation.
* `src/install.rs` owns service-manager detection and install/uninstall flows.
* `src/workers/mod.rs` groups resource-specific executors under a dedicated namespace.
* `scripts/package.sh` is kept outside `src/` because packaging is an operator-facing shell concern, not runtime logic.

---

## Notes

The docs in this directory are written from the current codebase, not from an ideal future architecture. If the repository later adds multiple crates, a database, or a frontend package, update these specs in the same task that introduces that change.
