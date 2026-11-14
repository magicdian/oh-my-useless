# Directory Structure

> How backend code is organized in this project today.

---

## Overview

The backend is a single binary crate with responsibility-driven modules. Keep `main.rs` thin, keep top-level orchestration in dedicated modules, and place resource-specific execution code under `src/workers/`.

Do not treat the imported shell reference as the primary implementation path. New features belong in Rust unless the task explicitly says otherwise.

---

## Directory Layout

```text
src/
├── main.rs         # command dispatch only
├── cli.rs          # CLI parsing and usage text
├── config.rs       # config paths, defaults, parsing, validation
├── daemon.rs       # composition root: sampler -> reducer -> store -> workers
├── install.rs      # install, uninstall, reload, status, service manager glue
├── update.rs       # self-update policy: CLI options, root gate, GitHub Releases, restart decision
├── ports.rs        # Sampler, Clock, StatePublisher, JournalSink, EventSource traits
├── domain/
│   ├── mod.rs      # public domain contract
│   ├── signal.rs   # Signal, SampleSnapshot, SessionSet
│   ├── state.rs    # Mode, ReasonCode, PolicyState, PolicyOutput
│   ├── reducer.rs  # pure reduce(state, signal, config, now)
│   ├── budget.rs   # memory budget arithmetic
│   └── schedule.rs # burst/rest scheduler with an injected seed
├── runtime/
│   ├── mod.rs
│   ├── store.rs    # generation counter + condvar for worker directives
│   └── journal.rs  # bounded transition history
├── adapters/
│   ├── mod.rs
│   ├── cgroups.rs  # cgroup slice discovery and own-cgroup exclusion
│   ├── netstat.rs  # interface counters, traffic ledger, external-traffic estimator
│   ├── psi.rs      # PSI trigger watches for host and cgroup scopes
│   ├── sessions.rs # inotify session sources, tty idle probe, legacy sampler
│   ├── signals.rs  # signalfd source
│   └── procfs.rs   # host metrics; who/ps parser used by the legacy sampler
├── error.rs        # AppResult / boxed_error helpers
├── generated/
│   └── version.rs  # OMV runtime export (generated; not a Rust module)
└── workers/
    ├── mod.rs      # worker registration
    ├── cpu.rs      # CPU load worker
    ├── memory.rs   # chunked mmap ledger with fresh headroom checks
    └── network.rs  # network load worker

.omv/
├── config.toml     # release_mode=pipeline, version format/policy
├── state.toml      # single version truth (YYMM.dd.N)
├── targets.toml    # Rust target → Cargo.toml + src/generated/version.rs
├── ai/             # generated agent/spec contract (committed)
└── skills/         # generated helper guidance (committed)

scripts/
└── package.sh      # release tarball builder (same layout as the release workflow)

.github/workflows/
├── ci.yml          # fmt, clippy, tests; macOS; OMV drift + PR version-truth guard
└── release.yml     # main/manual: OMV bump, commit, tag, multi-target upload

Oracle-server-keep-alive-script-main/
└── ...             # legacy shell reference, not the main implementation target
```

---

## Module Organization

### Rule: One file, one top-level responsibility

Use a dedicated module for each boundary:

* command-line shape in `cli.rs`,
* operator-visible config model in `config.rs`,
* composition and lifecycle in `daemon.rs`,
* install/service-manager concerns in `install.rs`,
* self-update policy and the GitHub release protocol in `update.rs`,
* pure decisions in `domain/` (no filesystem, clock, or logging),
* observation in `adapters/` (emits `Signal`, never decides),
* shared directives and transition history in `runtime/`.

Do not collapse these concerns into `main.rs`.

Version truth lives under `.omv/`; `src/generated/version.rs` is generated output and must not be edited by hand.

### Rule: Respect the layer direction

`daemon.rs` may call `adapters`, `domain`, `runtime`, and `workers`. `adapters` may use `domain` types but never `reduce`. `workers` may read `runtime::Store` and the config values needed for actuation, but never decide policy. `domain` may depend on config data types, never on IO.

### Rule: Group executors by domain

If code directly consumes CPU, memory, or network, it belongs in `src/workers/`.

Examples:

* `src/workers/cpu.rs` owns the busy-loop worker and duty distribution.
* `src/workers/memory.rs` owns allocation and buffer-touch behavior.
* `src/workers/network.rs` owns rate-limited download behavior.

### Rule: Shared types live above the worker layer

Shared runtime state, policy outputs, and config structures belong in non-worker modules so all workers read the same contract.

Examples:

* `src/runtime/store.rs` defines shared directives and the generation counter across threads.
* `src/domain/state.rs` defines `PolicyOutput`, `ModuleDirective`, and `ReasonCode`.
* `src/config.rs` defines `Config`, per-module sections, and install paths.

---

## Naming Conventions

* File names use `snake_case`.
* Module names reflect responsibility, not vague categories like `helpers` or `misc`.
* Keep worker files singular by resource: `cpu.rs`, `memory.rs`, `network.rs`.
* Use plural directories only for real families of modules, such as `workers/`.

---

## Forbidden Structure Patterns

### Do not put business logic in `main.rs`

`main.rs` should only wire modules together and map commands to functions.

### Do not create generic dumping-ground modules

Avoid files like `utils.rs`, `common.rs`, or `misc.rs` unless the code is truly cross-cutting and already reused.

### Do not mix runtime logic with packaging or shell integration

Runtime logic stays in Rust modules under `src/`.
Release and operator shell glue stays under `scripts/`.

### Do not let adapters or workers decide policy

Only `daemon.rs` calls `domain::reduce`. Adapters emit facts; workers apply directives. A worker that re-implements a threshold, or an adapter that returns a decision, is a layering regression.

### Do not read the clock or filesystem from `domain/`

Pass `now` into `reduce` and keep every observation behind an adapter. This is what makes the policy table-testable without a Linux host.

---

## Examples

* `src/main.rs` is the correct shape for the crate root: parse arguments, dispatch, exit with an error code.
* `src/install.rs` is the correct place for generated systemd and init.d templates.
* `src/workers/mod.rs` is the correct place to spawn one thread per worker family.
* `scripts/package.sh` is the correct place for tarball creation.
