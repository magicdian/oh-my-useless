# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`

## Intended changes

### `daemon-install-contracts.md`

- Add to the Scope/Trigger list: version surfaces under `.omv/`, `src/generated/version.rs`, and the version/release jobs under `.github/workflows/`.
- Add a **Versioning and release** subsection under Contracts:
  - `.omv/state.toml` is the single version truth; `.omv/config.toml` declares `release_mode = "pipeline"` and `requires_omv = "2611.2.1"`.
  - `Cargo.toml`, `Cargo.lock`, and `src/generated/version.rs` are synchronized outputs; `Cargo.lock` is refreshed by `cargo metadata` because the v1 Rust target does not touch it.
  - Local and AI callers use only `omv current`, `omv plan`, and `omv sync --check`. `omv bump` and `omv sync` are denied without `--as-release` in pipeline mode.
  - Versions are date triplets `YYMM.dd.BuildNumber` (for example `2611.14.1`), which are valid semver for the updater. Bootstrap state is `2611.14.0`; the first pipeline release is `2611.14.1`.
  - Tags are bare `<version>` with no `v` prefix. The release workflow creates the tag; manual tags are not a release path.
  - The release job runs on push to `main` or `workflow_dispatch` and performs: test → `omv bump --expect <current> --now <UTC date> --as-release` → `omv sync --check` → `cargo metadata` → path whitelist → commit `chore(release): <version>` → push → annotated tag.
  - The release commit may contain only `.omv/**`, `Cargo.toml`, `Cargo.lock`, and `src/generated/version.rs`.
  - `ci.yml` runs `omv sync --check` on every push/PR; PRs must not change `.omv/state.toml` or `.omv/finalizations.toml`, and `.omv/config.toml` must keep `release_mode = "pipeline"`.
  - `useless update --version` accepts `X.Y.Z` and `vX.Y.Z` and normalizes to the bare tag `X.Y.Z`.
- Update the enforcement note for the default `GITHUB_TOKEN`: it must be allowed to push to `main`; protected branches require an App or PAT with bypass permission.
- Add validation rows:
  - `omv sync --check` drift exits non-zero; a PR editing `.omv/state.toml` fails CI.
  - `omv bump` / `omv sync` without `--as-release` in pipeline mode exit with `release_mode_denied`.
  - A release diff outside the whitelist aborts before commit.
  - A release tag is created only by the workflow; the build job consumes it in the same run.
- Update the required checks to include `omv current --json`, `omv sync --check --json`, and `omv bump --dry-run --as-release`.
- Update unit/test expectations: `normalize_tag` accepts bare and `v`-prefixed forms; the release transaction simulation produces one `chore(release): 2611.14.1` commit, a clean worktree, and the matching tag.
- Add wrong-case entries: hand-editing `Cargo.toml` version; running `omv bump` locally; pushing a manual release tag; making a tag-push-triggered workflow depend on a `GITHUB_TOKEN` tag push.
- Add correct-case entries: version surfaces are committed by the pipeline; PRs are read-only for version truth; tag format is bare and normalized in the updater.

### `directory-structure.md`

- Add `.omv/config.toml`, `.omv/state.toml`, `.omv/targets.toml`, and the generated `.omv/ai/**` / `.omv/skills/**` to the layout as the version-truth surface.
- Add `src/generated/version.rs` as the OMV runtime export (committed, not a Rust module).
- Add the `version` job to `.github/workflows/ci.yml` and note that `.github/workflows/release.yml` owns version assignment, tagging, and multi-target upload.

## Rationale

The repository now has a single version writer and a generated runtime export. The existing install contract describes `install`/`update` but not where versions come from, and the directory layout has no `.omv/` or generated export. Without this delta, a future change could reintroduce hand-edited versions, a tag-triggered build that never runs, or a release commit containing business code.

## Verification

- After applying: `grep -n "omv sync --check" .phronia/spec/backend/daemon-install-contracts.md` and `grep -n "src/generated/version.rs" .phronia/spec/backend/directory-structure.md` return hits.
- `omv current --json` reports `2611.14.0`, `release_mode: "pipeline"`.
- `omv sync --check --json` exits 0 on the landed tree.
- The release transaction simulation in a throwaway clone produces one `chore(release): 2611.14.1` commit, a clean worktree, and tag `2611.14.1`.
- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` pass (documentation-only delta).
