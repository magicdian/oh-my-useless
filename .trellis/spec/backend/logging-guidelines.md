# Logging Guidelines

> How the current backend emits operational information.

---

## Overview

There is no logging crate yet. The current project uses:

* `println!` for user-requested CLI output,
* `eprintln!` for daemon lifecycle and error messages.

This is intentional for the current std-only MVP. If the project later adds structured logging, update this spec in the same task.

---

## Output Channels

### Use stdout for normal command output

Examples:

* `src/install.rs::status` prints service and config details with `println!`
* `src/install.rs::install` prints the final success message with `println!`
* `src/main.rs` prints help text and the default config path with `println!`

### Use stderr for daemon lifecycle and failures

Examples:

* daemon startup banner in `src/daemon.rs`
* config reload failure in `src/daemon.rs`
* policy summary changes in `src/daemon.rs`
* fatal CLI error wrapper in `src/main.rs`

This separation keeps command output script-friendly while still surfacing service diagnostics.

---

## What To Log

Log events that help an operator understand process state transitions:

* daemon start and stop,
* config reload success or failure,
* sensor backend failures,
* policy summary changes such as interactive-session suppression,
* install/uninstall success from CLI commands.

Keep log messages short and centered on actionable state.

---

## What Not To Log

* Full config contents when not required
* Shell command strings that include secrets in the future
* Per-loop spam when the state has not changed
* Large raw buffers or downloaded data

The current daemon already follows the "log on summary change" pattern in `src/daemon.rs` by tracking `last_summary`.

---

## Current Limitations

Because there is no structured logger yet:

* messages are plain text,
* there are no log levels beyond the stdout/stderr split,
* field-based filtering is not available.

That is acceptable today, but once logs become more numerous or machine-consumed, adopt a real logger instead of building an ad hoc format around `eprintln!`.

---

## Forbidden Patterns

### Do not mix status output and diagnostics on stdout

`useless status` should stay readable by humans and scripts. Keep operational warnings and errors on stderr.

### Do not print every loop iteration

The daemon loop runs frequently. Log only on transition or failure, not every sample tick.

### Do not add a logging dependency casually

The crate is intentionally std-only right now. Adding a logging crate changes the operational surface and should be justified in the task.
