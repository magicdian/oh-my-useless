# Directory Structure

> Current frontend structure and the rule for introducing one later.

---

## Current State

There is no frontend directory structure today.

Negative examples from the repository:

* no `web/`, `app/`, or `ui/` package,
* no `src/components/`,
* no `src/pages/`,
* no frontend build config.

The current `src/` directory is Rust backend code and must not be reused for browser code.

---

## If A Frontend Is Added Later

Create a dedicated top-level package instead of mixing frontend files into the Rust crate.

Recommended shape:

```text
web/
├── src/
├── public/
├── package.json
└── tsconfig.json
```

or another clearly named top-level package such as `ui/`.

---

## Naming Conventions

Until a frontend exists:

* do not create placeholder component folders,
* do not add frontend assets under backend source paths,
* do not place TypeScript files under the Rust crate `src/`.

When a frontend is added, update this file with the actual chosen directory layout in the same task.
