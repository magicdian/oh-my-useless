<!-- OMV-MANAGED-FILE source=.omv/ai/adapters/phronia/guide.md contract=1 -->
# OMV Versioning Guide

- `.omv/state.toml` is the version source of truth.
- Use `omv current --json` for reads.
- Use `omv plan --json` to preview target changes.
- Use `omv sync --check --json` to verify drift without mutation.
- Use `omv integrate status --json` and `omv integrate apply --json` for host integration provider/capability state where available.
- Do not call the OMV finalize-boundary helper or `omv bump`; the release pipeline assigns versions. Local version writes are rejected, and a finalize call only records a pending entry that never bumps.
- Do not run `omv bump` or `omv sync` locally; the release pipeline owns writes.
- Do not trust manifest versions as authority.
- Do not treat this guide or other host files as OMV authority.

Canonical reference: `./.omv/ai/instructions.md`