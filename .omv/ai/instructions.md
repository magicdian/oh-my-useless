# OMV Versioning Instructions

- Version truth lives in `.omv/state.toml`.
- Integration desired state and last detection snapshot live in `.omv/integrations.toml`.
- Read the current managed version with `omv current --json`.
- Preview target drift and proposed writes with `omv plan --json`.
- Check target drift without mutation with `omv sync --check --json`.
- Inspect host integration provider/capability status with `omv integrate status --json` when that command is available.
- Apply selected or pending host integration capabilities with `omv integrate apply --json` when that command is available.
- Treat `.trellis` and `.phronia` as mutually exclusive OMV projection targets. When both host trees exist, resolve the conflict (remove or rename one framework) before running `omv integrate apply` or `omv adapter refresh`; OMV does not choose, rewrite records, or delete framework files automatically.
- Do not run `omv bump` or `omv sync` locally; the release pipeline owns version writes.
- Do not call the OMV finalize-boundary helper or `omv bump`; the release pipeline assigns versions. Local version writes are rejected, and finalize calls would only record a pending entry that never bumps.
- The release job must run the project build command after bumping and commit the OMV-generated files together.
- `.omv/targets.toml` kind-based targets can manage text scalars, regex replacements, Markdown managed blocks, YAML scalars, C header macros, and Cargo workspaces; update OMV if a configured kind is reported as unsupported.
- Do not edit `Cargo.toml`, `CMakeLists.txt`, `pyproject.toml`, `go.mod`, or other native manifest versions directly.
- Before release-sensitive edits, run `omv plan --json`; before committing or publishing, run `omv sync --check --json`.
- Treat runtime export files such as `src/generated/version.rs` and `include/omv_version.h` as generated read-only views.
- Treat host files such as `AGENTS.md`, `CLAUDE.md`, `.codex/skills/*`, and Trellis/Phronia/OpenSpec guides as derived projections, not OMV authority.

When integrating OMV with agents or spec frameworks, keep the detailed rules in `.omv/ai/*` and project only thin host adapters into external files. Legacy `omv adapter ...` commands remain temporary compatibility commands during the MVP transition; new automation should prefer `omv integrate status/apply` where available.
