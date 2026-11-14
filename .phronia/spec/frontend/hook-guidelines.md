# Hook Guidelines

> Hook guidance for a repository that currently has no frontend runtime.

---

## Current State

There are no custom hooks because there is no React or React-like frontend in the repository.

That means:

* no `use*` hooks exist,
* no client-side data-fetching convention exists,
* no shared UI stateful logic exists.

---

## Current Convention

Do not create hook abstractions speculatively while the project is still backend-only.

If a future task introduces React or another hook-based framework, document:

* where hooks live,
* how async data is fetched,
* what must stay local vs shared,
* how hook naming works.

Until then, this file exists mainly to stop AI from assuming hook patterns that are not present.
