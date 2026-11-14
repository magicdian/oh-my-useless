# Frontend Development Guidelines

> Frontend guidance for a repository that currently has no frontend code.

---

## Overview

There is no frontend package, no JavaScript or TypeScript toolchain, and no browser UI in the current repository.

Repository evidence:

* the product code under `src/` is Rust only,
* there is no `package.json`,
* there is no `tsconfig.json`,
* there are no `.tsx` component files.

The purpose of this directory is therefore:

* to make the current absence of a frontend explicit,
* to prevent AI agents from inventing frontend conventions that do not exist yet,
* to define the minimum documentation updates required if a frontend is added later.

---

## Pre-Development Checklist

Before adding any frontend code:

1. confirm that the task really requires a browser or desktop UI,
2. decide whether the frontend should live in a new top-level package instead of the current Rust `src/`,
3. update these frontend docs in the same task that introduces the new UI layer,
4. read `.trellis/spec/guides/cross-layer-thinking-guide.md` because config, daemon state, and frontend contracts will become cross-layer.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | What to do if a frontend package is introduced | Filled |
| [Component Guidelines](./component-guidelines.md) | Current absence of components and future rules | Filled |
| [Hook Guidelines](./hook-guidelines.md) | Current absence of hooks and future rules | Filled |
| [State Management](./state-management.md) | Current absence of UI state and future rules | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Quality bar for any future frontend addition | Filled |
| [Type Safety](./type-safety.md) | Type-system expectations for future UI code | Filled |

---

## Current Convention

The real current convention is simple:

* do not add frontend code silently inside the Rust backend tree,
* do not assume React, Vue, Svelte, or any other framework is already chosen,
* do not invent package-manager or build conventions that are not in the repo.

If a frontend is introduced later, the same task must update these docs from placeholders into real conventions.
