# State Management

> State-management guidance for a project that currently exposes only CLI and daemon behavior.

---

## Current State

There is no frontend state layer today.

The active state in this repository is backend/runtime state:

* daemon runtime state in `src/runtime.rs`,
* scheduling and pressure state in `src/policy.rs`,
* operator configuration in `src/config.rs`.

That is backend state, not client-side UI state.

---

## Current Convention

Do not describe backend runtime state as if it were frontend global state.

If a frontend is added later, decide explicitly:

* what state remains daemon-only,
* what state is fetched as read-only status,
* what state is editable by operators,
* whether a frontend state library is even needed.

No such frontend convention exists yet.
