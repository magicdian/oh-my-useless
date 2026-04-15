# Error Handling

> How errors are represented and propagated in the current Rust backend.

---

## Overview

The project currently uses a minimal std-only error model:

* `AppResult<T>` for fallible application functions,
* `AppError` as `Box<dyn Error + Send + Sync>`,
* `boxed_error(...)` for user-facing or synthetic error messages.

The design goal is simple propagation with `?`, one top-level CLI error print, and targeted daemon logging for long-running failures.

---

## Error Types

Current shared error definitions live in `src/error.rs`.

Use:

```rust
pub type AppError = Box<dyn Error + Send + Sync>;
pub type AppResult<T> = Result<T, AppError>;
```

Create custom messages with:

```rust
boxed_error("this command must be run as root")
```

Examples in the codebase:

* `src/cli.rs` returns `boxed_error(...)` for unknown flags and commands.
* `src/config.rs` returns line-aware parse errors such as invalid config syntax.
* `src/install.rs` wraps failed command execution into a detailed message with stdout and stderr.

---

## Propagation Patterns

### Rule: Propagate operational failures with `?`

Use `?` for filesystem, parsing, process, and networking failures when the caller should decide what to do next.

Examples:

* `src/config.rs::load_config`
* `src/install.rs::install`
* `src/daemon.rs::run`
* `src/sensor.rs::sample`

### Rule: Convert user-input mistakes into explicit messages

When validating CLI flags or config fields, return a readable error that includes the bad value or line number.

Examples:

* unknown CLI command in `src/cli.rs`
* missing `--config` value in `src/cli.rs`
* invalid config line and unknown config key in `src/config.rs`

### Rule: Best-effort cleanup may ignore non-critical failures

Cleanup during uninstall or shutdown may intentionally use `let _ = ...` when failure should not block the whole operation.

Examples:

* removing old systemd or init.d files in `src/install.rs`
* joining worker threads during daemon shutdown in `src/daemon.rs`

Use this only for cleanup or non-authoritative teardown, not for the main happy path.

---

## Entry-Point Behavior

### CLI contract

`src/main.rs` prints one final error line and exits non-zero:

```rust
if let Err(error) = run() {
    eprintln!("error: {error}");
    std::process::exit(1);
}
```

Do not print the same fatal CLI error in multiple layers.

### Daemon contract

The daemon keeps running on recoverable loop failures and logs those failures to stderr.

Examples:

* config reload failure is logged and the old config stays active in `src/daemon.rs`
* sensor sampling failure pauses policy and updates runtime state instead of crashing immediately

---

## Allowed `expect(...)` Usage

The current codebase does use `expect(...)`, but only for internal invariants, not for user input or OS-dependent operations.

Acceptable current examples:

* poisoned lock messages when reading shared runtime state in workers
* thread spawn failures in `src/workers/mod.rs`

Do not use `expect(...)` for:

* parsing user config,
* validating CLI input,
* filesystem paths that may not exist,
* process calls that can fail on a real machine.

---

## Wrong vs Correct

### Wrong

```rust
let value = args.next().unwrap();
```

Why it is wrong:

* this turns a user input mistake into a panic,
* it loses the CLI context that should be returned as a normal error.

### Correct

```rust
let Some(value) = args.next() else {
    return Err(boxed_error("missing value after --config"));
};
```

---

## Common Mistakes

* Printing the same failure both in a helper and again in `main`
* Using panic-style APIs for operator input
* Swallowing install or reload failures that the operator needs to act on
* Returning vague errors without the offending path, line number, or command
