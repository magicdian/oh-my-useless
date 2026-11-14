# Quality Guidelines

> Quality expectations for any future frontend work in this repository.

---

## Current State

There is no frontend to lint, typecheck, or test today.

The current repository quality checks are backend-only:

* `cargo fmt`
* `cargo check`
* `cargo test`

---

## Current Convention

Do not claim frontend quality conventions already exist when the toolchain does not exist.

If a frontend package is introduced, the same task must add:

* its format and lint commands,
* its typecheck command,
* its test strategy,
* any accessibility checks if it renders UI.

Until then, backend checks remain the only real enforced quality gate in this repository.
