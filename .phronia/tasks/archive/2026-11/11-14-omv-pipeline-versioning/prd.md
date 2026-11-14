# feat: adopt OMV pipeline versioning

## Goal

Make `.omv/state.toml` the only version truth for `useless`, with `release_mode = "pipeline"`: contributors and AI agents never bump versions, and a GitHub Actions release job on `main` assigns the next date-based version, commits the version surfaces, tags the commit, and hands the tag to the existing multi-target build/upload workflow.

The user-visible outcomes:

- `omv current` is the authoritative version; `Cargo.toml`, `Cargo.lock`, and the generated runtime export are derived output.
- Merging to `main` (or manually running the Release workflow) produces one new `YYMM.dd.N` release with four Linux archives.
- Pull requests that edit version truth or drift from it fail CI.
- `useless version` and `useless update` keep working with the bare OMV tags.

## Background

The repository currently releases by hand: bump `Cargo.toml`, tag `vX.Y.Z`, push the tag, and let `.github/workflows/release.yml` build. There is no check that the tag matches the manifest beyond a version guard inside the release workflow, and two branches can pick the same version.

OMV (oh-my-versioning, `omv 2611.6.1` here) models version truth as `.omv/state.toml` and synchronizes native manifests from it. Its `release_mode = "pipeline"` makes the release job the only writer: `omv bump` and `omv sync` are denied without `--as-release`, while `omv current`, `omv plan`, and `omv sync --check` stay read-only. Versions are date-based triplets `YYMM.dd.BuildNumber`, which are valid semver values (`2611.14.1`), so `self_update`'s comparison still works.

Verified locally before planning:

- `omv sync --check` exits `1` when a required target drifts.
- `omv bump --expect <ver> --now <date> --as-release` is a compare-and-swap and exits `1` on a mismatch.
- A v1 `language = "rust"` target writes `Cargo.toml` and `src/generated/version.rs`, preserving unrelated manifest fields.
- The release commit does not update `Cargo.lock`; `cargo metadata --format-version 1 > /dev/null` refreshes it.
- `omv sync`/`bump` also generate `.omv/ai/**` and `.omv/skills/**`, which are version surfaces and belong in the release commit.
- Bootstrapping `build_number = 0` makes the first pipeline release `2611.14.1`.

## Requirements

**R1 — Single version truth.** `.omv/state.toml` is the only place a version is assigned. `.omv/config.toml` sets `release_mode = "pipeline"`, `requires_omv = "2611.2.1"`, `version_output = "date-triplet"`, `build_policy = "daily-reset"`, and a fixed `timezone = "UTC"`.

**R2 — Rust target contract.** `.omv/targets.toml` declares one v1 Rust target for the crate: `manifest_path = "Cargo.toml"`, `runtime_export_path = "src/generated/version.rs"`, `strategy = "intent-only"`, `enabled = true`. OMV writes both files from state.

**R3 — Bootstrap.** Initialize state at `2611.14.0` (`build_number = 0`, logical date 2026-11-14) so the first pipeline release is `2611.14.1`. Commit the generated `.omv/ai/**`, `.omv/skills/**`, `src/generated/version.rs`, the synced `Cargo.toml`, and the refreshed `Cargo.lock`.

**R4 — PR/CI drift guard.** CI installs a pinned OMV, runs `omv current --json` and `omv sync --check --json`, and fails on drift. On pull requests it additionally rejects changes to `.omv/state.toml` and `.omv/finalizations.toml`, and verifies `.omv/config.toml` still declares `release_mode = "pipeline"`.

**R5 — Pipeline release job.** `Release` runs on push to `main` and on `workflow_dispatch`, with a non-cancelling `omv-release` concurrency group. Jobs: `test` → `version` → `create-release` → `upload` (four-target matrix). The version job is the only writer.

**R6 — Release transaction.** The version job:
1. Reads `omv current --json`.
2. Runs `omv bump --expect <current> --now <UTC date> --as-release`.
3. Runs `omv sync --check`.
4. Runs `cargo metadata --format-version 1 > /dev/null` to refresh `Cargo.lock`.
5. Verifies every changed path is under `.omv/`, or is `Cargo.toml`, `Cargo.lock`, or `src/generated/version.rs`.
6. Commits exactly those paths as `chore(release): <version>`.
7. Pushes to `main`, creates the annotated tag `<version>` with a receipt, and pushes the tag.
8. Exposes the version as a job output.

The job is resumable: if `HEAD` is already a `chore(release): <version>` commit, it reuses that version and only ensures the tag exists.

**R7 — Tag format and self-update.** Tags are bare `YYMM.dd.N` (no `v`). `useless update --version` accepts both `2611.14.1` and `v2611.14.1` and normalizes to the bare tag. `Release` is no longer triggered by a tag push; the pipeline owns tag creation.

**R8 — Build and upload.** `create-release` uses `taiki-e/create-gh-release-action@v1` with `ref: refs/tags/<version>`. The upload matrix checks out that tag and uses `taiki-e/upload-rust-binary-action@v1` with the same `ref`, `archive: useless-$target`, `checksum: sha256`, and `include: README.md,LICENSE`.

**R9 — Documentation.** README documents: OMV as version truth, local read-only commands, `npm install -g @magicdian/omv@2611.6.1`, the main-push release flow, why manual tags are forbidden, the first version, and the bare-tag `--version` form. Backend specs receive the same contract through the task's `spec-delta.md`.

**R10 — Scope.** No OMV AI/spec adapters are installed (user decision). No local `omv bump` is allowed. No business code is part of the release commit. Existing self-update behavior, service restart, and artifact naming stay unchanged.

## Acceptance Criteria

- [ ] AC1 `.omv/config.toml`, `.omv/state.toml`, and `.omv/targets.toml` exist and `omv current --json` reports `2611.14.0` with `release_mode = "pipeline"`.
- [ ] AC2 `omv sync --check --json` exits 0 after bootstrap; editing `Cargo.toml` version by hand makes it exit 1.
- [ ] AC3 `omv bump --dry-run --as-release --now 2026-11-14 --json` plans `2611.14.1`.
- [ ] AC4 `omv bump` and `omv sync` without `--as-release` exit 1 with `release_mode_denied`.
- [ ] AC5 `Cargo.toml`, `Cargo.lock`, and `src/generated/version.rs` all carry `2611.14.0`; `useless version` prints it.
- [ ] AC6 `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` pass.
- [ ] AC7 `.github/workflows/ci.yml` contains the OMV drift job and PR version-surface guard; `.github/workflows/release.yml` contains `test`, `version`, `create-release`, and `upload` jobs and triggers on `main` push + `workflow_dispatch`.
- [ ] AC8 The release transaction simulated in a throwaway clone produces exactly one `chore(release): 2611.14.1` commit (state, skills, Cargo.toml, Cargo.lock, generated version), a clean worktree, and tag `2611.14.1`.
- [ ] AC9 `normalize_tag` unit tests cover `2611.14.1`, `v2611.14.1`, `V2611.14.1`, and whitespace; `useless update --version 2611.14.1` targets the bare tag.
- [ ] AC10 README contains the OMV versioning/release instructions, and `./scripts/package.sh` still produces the expected archive and checksum.

## Out of scope

- Installing OMV AI/spec adapters into `AGENTS.md`, `.phronia/spec`, or finish-work.
- Releasing from `dev` (CI validates `dev` but only `main` publishes).
- GitHub App/PAT automation for protected branches; the first version uses `GITHUB_TOKEN` and requires `main` to permit the Actions token, or a documented token swap.
- Changing artifact target matrix or self-update asset naming.
- Changelog generation or release notes beyond the OMV receipt in the tag message.

## Notes

- OMV's `semver` output option formats the same date triplet; there is no `.omv` mode that preserves `0.1.0` under `bump`. Adopting OMV means adopting the date triplet.
- `main` and `dev` are currently separate; the first release only happens after `main` receives this work.
- If branch protection blocks the Actions token, the version job fails before tagging; switch `secrets.GITHUB_TOKEN` to an App/PAT with bypass permission and keep the same transaction.
