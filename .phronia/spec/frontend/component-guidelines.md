# Component Guidelines

> Component rules for a project that currently has no UI components.

---

## Current State

There are no frontend components in this repository today.

Evidence:

* no `.tsx` files,
* no frontend framework dependency,
* no component directory,
* the runtime surface is a CLI daemon, not a browser app.

---

## Current Convention

Do not invent components as part of backend work.

If a task truly adds a UI, the task must also define:

* the chosen framework,
* where components live,
* how styling works,
* what accessibility expectations apply.

Until then, there is no valid "existing component pattern" to imitate.

---

## When Components Are Introduced

Use these minimum rules:

* colocate component code within the frontend package, not the Rust backend crate,
* type props explicitly,
* prefer accessible native elements before custom replacements,
* document the chosen styling system in this file during the same task.

If these rules become real code, replace this placeholder guidance with repository-backed examples.
