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

### Daemon state and transition journal

Two bounded operational files complement stderr:

* `/run/useless/state` is a line-based `key=value` snapshot written on every mode transition and at most once per 30 s. Keys: `mode`, `reasons`, `scope`, `budget_mb`, `reserve_mb`, `available_mb`, `capabilities`, `last_transition`, `summary`. It is deliberately not JSON so `status` needs no parser.
* `/run/useless/journal.jsonl` is transition-only JSON Lines, capped at ~256 KiB by keeping the most recent `capacity/2` entries. Fields: `ts_ms`, `signal`, `from`, `to`, `reasons`, `budget_mb`, `available_mb`.

A journal write or trim failure logs once, disables the file sink for the process lifetime, and never crashes the daemon. `/run` is tmpfs, so both files are intentionally non-durable; journald already captures stderr.

The network gate logs each skipped burst with its ingress/egress estimate and logs a counter-read failure once; the probe runs in the worker thread, not the reactor.

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
