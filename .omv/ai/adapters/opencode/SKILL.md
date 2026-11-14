---
name: omv-versioning
description: "Use OMV as the version source of truth for this project."
---

<!-- OMV-MANAGED-FILE source=.omv/ai/adapters/opencode/SKILL.md contract=1 -->

1. Read `./.omv/ai/instructions.md`.
2. Use `omv current --json` to inspect current version truth.
3. Use `omv plan --json` or `omv sync --check --json` before changing version-sensitive files.
4. Use `omv integrate status --json` to inspect host integration state and `omv integrate apply --json` to apply selected capabilities where available.
- Do not call the OMV finalize-boundary helper or `omv bump`; the release pipeline assigns versions. Local version writes are rejected, and a finalize call only records a pending entry that never bumps.
6. Do not run `omv bump` locally; the release pipeline assigns versions.
7. Do not hand-edit manifest versions or treat host adapter files as authority.