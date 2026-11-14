# Design: OMV pipeline versioning

## 1. Version truth model

```
.omv/state.toml  (only mutable version truth)
      │
      ├── omv bump/sync ──► Cargo.toml
      │                   ├─► Cargo.lock (via cargo metadata)
      │                   ├─► src/generated/version.rs
      │                   ├─► .omv/skills/**
      │                   └─► .omv/ai/**
      │
      └── omv current / plan / sync --check  (read-only for everyone else)
```

`.omv/config.toml`:

```toml
schema_version = 1
requires_omv = "2611.2.1"
locale = "en-US"
timezone = "UTC"
project_profile = "personal"
version_output = "date-triplet"
build_policy = "daily-reset"
ntp_enabled = true
release_mode = "pipeline"
```

- `requires_omv` makes an older OMV fail closed instead of silently treating `pipeline` as `local`.
- `timezone = "UTC"` matches the CI `--now "$(date -u +%F)"` logical date. Using `UTC+8` would make the version depend on the runner's clock relative to the developer's day.
- `daily-reset` is OMV's default: build number resets to 1 when the logical date changes.

`.omv/state.toml` (bootstrap):

```toml
schema_version = 1
logical_date = "2026-11-14"
build_number = 0
last_issued_version = "2611.14.0"
last_time_source = "manual-confirmed"
```

`build_number = 0` is the bootstrap counter; the first pipeline bump produces `1`, so the first published release is `2611.14.1` without skipping a number.

`.omv/targets.toml`:

```toml
schema_version = 1

[[targets]]
id = "useless-rust"
language = "rust"
root = "."
manifest_path = "Cargo.toml"
runtime_export_path = "src/generated/version.rs"
strategy = "intent-only"
enabled = true
```

The v1 Rust adapter writes both the manifest and the runtime export. `strategy` only affects `omv init` scaffolding, not sync. `src/generated/version.rs` stays unlinked, matching OMV's own repository; it is the runtime view, not a module.

Why not `kind = "cargo-workspace"`: that adapter requires a `[workspace].members` array. This repository is a single package, and adding a synthetic workspace to satisfy it would change the build topology for no benefit.

## 2. Bootstrap transaction

Run once, in the task branch:

```bash
# files above are written first
omv current --json                 # -> 2611.14.0, release_mode=pipeline
omv sync --as-release --json       # writes Cargo.toml + src/generated/version.rs + .omv/{ai,skills}
cargo metadata --format-version 1 > /dev/null   # refresh Cargo.lock package version
omv sync --check --json            # -> ok
```

Then commit `.omv`, `Cargo.toml`, `Cargo.lock`, and `src/generated/`. This commit is the bootstrap; it is not a published release. No tag is created here.

`omv sync --as-release` is used because the project is already in pipeline mode; the bootstrap is the one authorized local write. The alternative (bootstrap in `local`, then flip to `pipeline`) produces two commits and a window where local bump is allowed, so it is rejected.

## 3. CI: read-only drift guard

Add a `version` job to `.github/workflows/ci.yml`:

```yaml
env:
  OMV_VERSION: "2611.6.1"

jobs:
  version:
    name: Version surfaces
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
      - name: Install OMV
        run: npm install -g "@magicdian/omv@${OMV_VERSION}"
      - name: Show version truth
        run: omv current --json
      - name: Check target drift
        run: omv sync --check --json
      - name: Pull requests must not write version truth
        if: github.event_name == 'pull_request'
        env:
          BASE_REF: ${{ github.base_ref }}
        run: |
          set -euo pipefail
          git fetch --no-tags origin "$BASE_REF"
          changed="$(git diff --name-only "origin/$BASE_REF...HEAD" -- \
            .omv/state.toml .omv/finalizations.toml)"
          if [ -n "$changed" ]; then
            printf 'PR must not change version truth:\n%s\n' "$changed" >&2
            exit 1
          fi
          grep -q 'release_mode = "pipeline"' .omv/config.toml
```

`omv sync --check` covers the Cargo.toml/generated-file drift. The path check covers the hand-edited state file, which `sync --check` cannot distinguish from a legitimate release write. `.omv/config.toml` is not blocked wholesale (policy changes are legitimate) but must keep `release_mode = "pipeline"`.

Why not the OMV doc's literal whitelist that rejects any `Cargo.toml` change: this repository edits dependencies, features, and metadata in `Cargo.toml` in normal PRs. Blocking the file would block all normal work; `omv sync --check` is the precise check.

## 4. Release workflow

`.github/workflows/release.yml` becomes a single workflow. A separate tag-triggered workflow is rejected because tags pushed by `GITHUB_TOKEN` do not start new workflow runs, so the build would silently not happen.

```yaml
name: Release

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: omv-release
  cancel-in-progress: false

env:
  OMV_VERSION: "2611.6.1"
  CARGO_TERM_COLOR: always

jobs:
  test:            # fmt, clippy, test (same as CI linux job)
  version:
    needs: test
    permissions: { contents: write }
    outputs: { version: "${{ steps.release.outputs.version }}" }
    steps: [checkout fetch-depth 0, install omv, release transaction]
  create-release:
    needs: [test, version]
    permissions: { contents: write }
    steps:
      - checkout
      - taiki-e/create-gh-release-action@v1
        with:
          ref: refs/tags/${{ needs.version.outputs.version }}
          token: ${{ secrets.GITHUB_TOKEN }}
  upload:
    needs: [test, version, create-release]
    permissions: { contents: write }
    strategy:
      fail-fast: false
      matrix:
        target: [x86_64-unknown-linux-gnu, x86_64-unknown-linux-musl,
                 aarch64-unknown-linux-gnu, aarch64-unknown-linux-musl]
    steps:
      - uses: actions/checkout@v7
        with:
          ref: refs/tags/${{ needs.version.outputs.version }}
      - uses: taiki-e/upload-rust-binary-action@v1
        with:
          bin: useless
          target: ${{ matrix.target }}
          ref: refs/tags/${{ needs.version.outputs.version }}
          archive: useless-$target
          checksum: sha256
          include: README.md,LICENSE
          locked: true
          token: ${{ secrets.GITHUB_TOKEN }}
```

### Version job transaction

```bash
set -euo pipefail

head_msg="$(git log -1 --pretty=%s)"
if [[ "$head_msg" == "chore(release): "* ]]; then
  version="${head_msg#chore(release): }"
  echo "resuming release $version from HEAD"
else
  current="$(omv current --json | jq -r '.data.version')"
  version="$(omv bump --expect "$current" --now "$(date -u +%F)" --as-release --json \
    | jq -r '.data.version')"
  omv sync --check --json
  cargo metadata --format-version 1 > /dev/null

  changed="$(git status --porcelain | cut -c4-)"
  unexpected="$(printf '%s\n' "$changed" \
    | grep -vE '^(\.omv/|Cargo\.toml$|Cargo\.lock$|src/generated/version\.rs$)' || true)"
  if [ -n "$unexpected" ]; then
    printf 'unexpected files in release diff:\n%s\n' "$unexpected" >&2
    exit 1
  fi

  git config user.name "omv-release-bot"
  git config user.email "omv-release-bot@users.noreply.github.com"
  git add .omv Cargo.toml Cargo.lock src/generated/version.rs
  git commit -m "chore(release): $version"
  git push origin HEAD:main
fi

if git rev-parse -q --verify "refs/tags/$version" >/dev/null; then
  echo "tag $version already exists"
else
  git tag -a "$version" -m "omv-release receipt: source=$GITHUB_SHA run=$GITHUB_RUN_ID"
  git push origin "$version"
fi
echo "version=$version" >> "$GITHUB_OUTPUT"
```

Details that matter:

- `--expect` is the compare-and-swap; `concurrency` is the first line of defense.
- `--now "$(date -u +%F)"` makes the logical date deterministic and matches the `UTC` timezone in config.
- `cargo metadata` is required: v1 Rust targets do not touch `Cargo.lock`, and the existing build uses `--locked`.
- `git status --porcelain | cut -c4-` handles both ` M` and `??` entries. The whitelist is a safety net; `git add` lists paths explicitly so an unexpected untracked file is never committed.
- The resume branch exists because a rerun after a successful push but failed tag/upload must not issue a second version. The OMV doc's failure table is the source of this behavior.
- If the push is rejected (another commit landed), the job fails; rerun from the new tip. The OMV doc explicitly forbids rebasing a version commit, and a personal repository rarely races.
- `GITHUB_TOKEN` pushes do not retrigger this workflow, so there is no release loop. If a PAT is later substituted and branch protection requires it, add a guard that skips `chore(release):` head commits for push events and use `workflow_dispatch` for recovery.

### Branch protection

`permissions: contents: write` is sufficient on an unprotected `main`. If `main` is protected, the default token cannot push; the job fails before tagging. The documented fix is a GitHub App or fine-grained PAT with bypass permission, wired as `secrets.RELEASE_TOKEN`. The transaction is unchanged.

## 5. self-update compatibility

- OMV tags are bare: `2611.14.1`.
- `self_update`'s default tag prefix logic trims an optional leading `v`; bare semver tags parse directly.
- `src/update.rs::normalize_tag` changes from "always prepend `v`" to "strip one optional `v`/`V` and return the bare tag". `2611.14.1`, `v2611.14.1`, and `V2611.14.1` all target the bare tag.
- No legacy releases exist, so no compatibility fallback is needed. If a `v`-prefixed release is ever published, reintroduce a two-tag probe.
- `Release` no longer triggers on tag pushes. Manual tags are not part of the workflow; a stray tag creates no release.

## 6. Files changed

| Path | Change |
|---|---|
| `.omv/config.toml`, `.omv/state.toml`, `.omv/targets.toml` | New OMV truth. |
| `.omv/ai/**`, `.omv/skills/**` | Generated by OMV; committed as version surfaces. |
| `src/generated/version.rs` | Generated runtime export; committed, not a Rust module. |
| `Cargo.toml` | Version becomes `2611.14.0` via `omv sync`. |
| `Cargo.lock` | Package version refreshed via `cargo metadata`. |
| `src/update.rs` | `normalize_tag` returns bare tags; tests updated. |
| `.github/workflows/ci.yml` | Add OMV drift job and PR guard. |
| `.github/workflows/release.yml` | Single main/manual workflow with test → version → release → upload. |
| `README.md` | Versioning and release docs; no manual tags. |
| `.phronia/tasks/.../spec-delta.md` | Spec contract delta. |

## 7. Failure and rollback

| Failure | Behavior |
|---|---|
| Drift on a PR | CI `version` job fails; contributor runs `omv sync --check --json` locally to see the target plan. |
| CAS mismatch (`state_moved`) | Version job fails; rerun from the new tip. Never rebase or merge the version commit. |
| Branch protection blocks push | Job fails before tagging; switch to an App/PAT with bypass. |
| Job dies after push, before tag | Rerun detects the `chore(release)` HEAD, reuses the version, and creates the missing tag. |
| Tag exists on a different commit | `git rev-parse` says the tag exists; the job skips creation and the upload builds the existing tag. Investigate rather than force. |
| Upload/build fails | The tag and release exist; rerun the workflow; the version job resumes and uploads again. |
| Bad release | Roll forward with the next version; do not rewrite `.omv/state.toml`. |

Rollback for the adoption itself: delete `.omv/`, revert `Cargo.toml`/`Cargo.lock` to the prior version, remove the generated directory and the workflow `version` jobs. The previous tag-driven workflow can be restored from git history.

## 8. Alternatives rejected

- **Manual `omv bump` in a local hook.** Violates pipeline single-writer; two developers can still race before push.
- **Tag-triggered build after a `GITHUB_TOKEN` tag push.** GitHub suppresses workflow runs for events created by the default token, so the release would not build.
- **`cargo-workspace` target.** Requires a workspace members array; the repository is a single package.
- **OMV AI/spec adapters.** User opted for core OMV + CI only; adapters would add managed blocks to `AGENTS.md` and `.phronia/spec` and are a separate review surface.
- **Keeping `v`-prefixed tags.** OMV emits bare tags; adding a `v` would require either a manual rename in the workflow or a fork of OMV's release behavior.
