# Implementation plan: OMV pipeline versioning

Ordered checklist. Each step ends with a validation command.

## Step 1 — Bootstrap OMV truth

- [ ] Write `.omv/config.toml`, `.omv/state.toml`, `.omv/targets.toml` exactly as in `design.md` §1.
- [ ] `omv current --json` → version `2611.14.0`, `release_mode: "pipeline"`.
- [ ] `omv sync --as-release --json` → `synced: 1`.
- [ ] `cargo metadata --format-version 1 > /dev/null` to refresh `Cargo.lock`.
- [ ] `omv sync --check --json` → `ok: true`.
- [ ] Inspect `Cargo.toml` (version `2611.14.0`, other fields preserved), `Cargo.lock` (`useless` 2611.14.0), `src/generated/version.rs`, `.omv/ai/`, `.omv/skills/`.
- [ ] `omv bump --dry-run --as-release --now 2026-11-14 --json` → `2611.14.1`.
- [ ] Confirm `omv bump --json` and `omv sync --json` exit 1 with `release_mode_denied`.

Review gate: no business code changed in this step; only version surfaces and generated guidance.

## Step 2 — Self-update tag compatibility

- [ ] `src/update.rs::normalize_tag`: strip one optional `v`/`V`, trim whitespace, return the bare tag; keep the empty / `-` / `/` rejection.
- [ ] Update tests: `2611.14.1` → `2611.14.1`; `v2611.14.1` → `2611.14.1`; `V2611.14.1` → `2611.14.1`; whitespace trimmed; invalid inputs still rejected.
- [ ] Update the `configured_updater` test expectation from `Some("v0.2.0")` to `Some("0.2.0")` (or use an OMV-shaped version).
- [ ] `cargo test update::cli::` (and the CLI tests) pass.

Review gate: `--version` accepts both forms but always targets the bare OMV tag.

## Step 3 — CI drift guard

- [ ] `.github/workflows/ci.yml`: add workflow-level `OMV_VERSION: "2611.6.1"`.
- [ ] Add the `version` job from `design.md` §3:
  - checkout with `fetch-depth: 0`
  - `npm install -g "@magicdian/omv@${OMV_VERSION}"`
  - `omv current --json`
  - `omv sync --check --json`
  - PR-only: reject `.omv/state.toml` / `.omv/finalizations.toml` changes against `origin/$BASE_REF`, and require `release_mode = "pipeline"`.
- [ ] Validation: YAML parses; `grep` confirms the job and commands.

## Step 4 — Release workflow

- [ ] Rewrite `.github/workflows/release.yml` per `design.md` §4:
  - `on: push: branches: [main]` + `workflow_dispatch`
  - `concurrency: omv-release`, `cancel-in-progress: false`
  - jobs `test` → `version` → `create-release` → `upload`
  - `version` outputs `version`, runs the transaction script, uses `permissions: contents: write`
  - `create-release` uses `taiki-e/create-gh-release-action@v1` with `ref: refs/tags/<version>`
  - `upload` checks out the tag and passes `ref` to `taiki-e/upload-rust-binary-action@v1`, plus `archive`, `checksum`, `include`, `locked`, `token`
- [ ] Validation: YAML parses; the four targets and both `ref` lines are present.

Review gate: no tag-push trigger remains; the build consumes the tag created in the same run.

## Step 5 — README

- [ ] Add a **Versioning** section:
  - `.omv/state.toml` is truth; `Cargo.toml`/`Cargo.lock`/`src/generated/version.rs` are derived.
  - Local read-only commands: `omv current`, `omv plan`, `omv sync --check`.
  - Install OMV locally: `npm install -g @magicdian/omv@2611.6.1`.
  - Pipeline mode forbids local `omv bump`/`omv sync`.
- [ ] Update **Releases and CI**:
  - Releases run on push to `main` or manual `workflow_dispatch`.
  - The job bumps, commits `chore(release): <version>`, tags `<version>`, then builds four targets.
  - Do not create tags manually.
  - First managed version is `2611.14.0`; first pipeline release is `2611.14.1`.
  - Branch protection caveat: default token needs push permission to `main`, or use an App/PAT.
- [ ] Update `useless update --version` examples to bare `2611.14.1` (mention `v` is accepted).
- [ ] Confirm no stale `git tag v0.2.0` instructions remain.

## Step 6 — Spec delta

- [ ] Write `.phronia/tasks/11-14-omv-pipeline-versioning/spec-delta.md` targeting:
  - `.phronia/spec/backend/daemon-install-contracts.md`: version truth, pipeline commands, release job, tag format, self-update normalization, PR guard.
  - `.phronia/spec/backend/directory-structure.md`: `.omv/`, `src/generated/version.rs`, workflow jobs.
- [ ] Do not edit `.phronia/spec/**` in the code commits; apply after integration via `phronia task spec plan|commit`.

## Step 7 — Verification

- [ ] `omv current --json`, `omv sync --check --json`, `omv sync --check` exit code.
- [ ] `omv bump --dry-run --as-release --now 2026-11-14 --json` → `2611.14.1`.
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] `./scripts/package.sh` → archive layout + checksum; `useless version` prints `2611.14.0`.
- [ ] Parse both workflow YAML files.
- [ ] Simulate the release transaction in a throwaway clone:
  ```bash
  rm -rf /tmp/omv-release-sim
  rsync -a --exclude .git --exclude target --exclude dist . /tmp/omv-release-sim/
  cd /tmp/omv-release-sim
  git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -qm bootstrap
  # run the version-job script with --now 2026-11-14
  ```
  Assert: one `chore(release): 2611.14.1` commit; changed paths are exactly `.omv/**`, `Cargo.toml`, `Cargo.lock`, `src/generated/version.rs`; `git status --porcelain` is empty; tag `2611.14.1` exists; `omv sync --check` passes.
- [ ] `git status --short` review before commit.

## Rollback points

- After Step 1: remove `.omv/`, `src/generated/`, revert `Cargo.toml`/`Cargo.lock`; no runtime behavior changes.
- After Step 2: revert `src/update.rs` and its tests.
- Workflows and README are additive; restore the previous `release.yml` from git.
- No daemon behavior, config keys, or on-disk install paths change.
