# Database Guidelines

> Persistence conventions for this project.

---

## Current State

This repository does **not** currently use a database, ORM, migration tool, or embedded store.

Evidence from the current codebase:

* `Cargo.toml` has no database dependencies.
* `src/` contains no persistence module.
* Operator state is configuration and service metadata, not application data:
  * config file at `/etc/useless/config.toml`,
  * PID file at `/run/useless.pid`,
  * service definitions under `/etc/systemd/system/` or `/etc/init.d/`.

That means the current convention is:

* keep operator settings in the config file,
* keep transient runtime state in memory or PID files,
* do not introduce hidden persistence casually.

---

## Query Patterns

There are no query patterns yet because there is no database layer.

If a task introduces persistence later:

1. create a dedicated module for persistence rather than scattering file or DB writes across workers,
2. document the storage contract in this file during the same task,
3. treat schema choice as a cross-layer decision because install, config, and runtime behavior will all change.

---

## Migrations

There is no migration system today.

If a database is added later, do not land ad hoc schema files without answering all of the following in the same task:

* Where the storage lives
* How schema versions are tracked
* How upgrades and rollbacks work
* What happens on partial failure

---

## Naming Conventions

Current naming conventions are for file-based operational state:

* `/etc/useless/config.toml` for persistent operator configuration
* `/run/useless.pid` for ephemeral runtime process state
* `/usr/local/bin/useless` for the installed binary

If real persistence is added, keep one naming style across keys, files, tables, and fields instead of mixing shell-style names and Rust-style names arbitrarily.

---

## Forbidden Patterns

### Do not smuggle persistence into random files

Do not start writing silent state files in `/tmp`, `/var/tmp`, or the current working directory just to avoid making a storage decision explicit.

### Do not overload the config file as telemetry storage

`/etc/useless/config.toml` is for operator-edited configuration, not counters, metrics, or daemon snapshots.

### Do not add a database dependency without updating this spec

The project currently has a very small surface area. A database is a meaningful architecture change and must be documented as such.

---

## Common Mistakes To Avoid

* Treating PID or service-manager files as a general persistence layer
* Hiding dynamic runtime state inside the editable config file
* Adding storage because it is convenient for one worker instead of because the product actually needs durable state
