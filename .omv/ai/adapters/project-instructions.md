<!-- OMV-MANAGED-FILE source=.omv/ai/adapters/project-instructions.md contract=1 -->
# OMV Agent Instructions

Read `./.omv/ai/instructions.md` before touching project versions.

- Use `omv current --json` to inspect the managed version.
- Use `omv plan --json` before editing version-sensitive surfaces.
- Use `omv sync --check --json` to verify target drift without writing.
- Use `omv integrate status --json` and `omv integrate apply --json` for host integration provider/capability status where available.
- Do not call the OMV finalize-boundary helper or `omv bump`; the release pipeline assigns versions. Local version writes are rejected, and a finalize call only records a pending entry that never bumps.
- The release job must run the project build command after bumping and commit the OMV-generated files together.
- Do not run `omv bump` locally; the release pipeline assigns versions.
- Do not edit native manifest versions directly.
- Treat this host file as a derived projection; `.omv/*` and `.omv/ai/*` remain authoritative.