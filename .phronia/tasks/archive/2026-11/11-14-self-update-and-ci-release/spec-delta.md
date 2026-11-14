# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/directory-structure.md`

## Intended changes

### `daemon-install-contracts.md`

- Add `Self-update behavior in src/update.rs or the release workflow under .github/workflows/` to the Scope/Trigger list.
- Add to the CLI command list:
  - `useless update [--check] [--version <semver>] [--no-restart]`
  - `useless version` (also `--version`, `-V`)
- Add an **Update behavior** subsection under Contracts:
  - Updates are explicit; there is no background updater, timer, or automatic install.
  - The updater uses the `self_update` GitHub backend with `checksums`; the GitHub per-asset SHA-256 digest is verified before extraction.
  - Release assets are named `useless-<target>.tar.gz`, contain `useless` at the archive root, and must exist for every published target. Selection matches the running binary's compile-time target triple.
  - `update --check` performs no write and does not require root.
  - `update` requires root and writes `/usr/local/bin/useless` (`INSTALL_BIN_PATH`), not `current_exe()`.
  - `--version` accepts `X.Y.Z` and `vX.Y.Z`; it pins the release tag and may install an older version (rollback).
  - After a successful install, restart the service only when it is active; `--no-restart` skips it. A failed restart is reported after the binary has already been replaced.
  - `USELESS_GITHUB_TOKEN`, when set and non-empty, is passed to the GitHub API.
  - Tag-driven releases must reject a tag that does not match `CARGO_PKG_VERSION`.
- Add validation rows:
  - Non-root `useless update` fails with a root-required error before any network request.
  - Non-root `useless update --check` is allowed.
  - `--version` with no value or a flag value is a parse error.
  - A missing release asset for the running target fails; never install a foreign architecture.
  - A digest mismatch aborts before extraction and leaves the installed binary unchanged.
  - An update that succeeds but whose restart fails reports the restart failure separately.
  - A release tag that does not match `CARGO_PKG_VERSION` fails the release workflow before release creation.
- Update the required checks to `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo check`, and `cargo test --all-targets --locked`.
- Add update unit-test expectations: CLI parsing for `update`/`version`, tag normalisation (`X.Y.Z` and `vX.Y.Z`), and the configured install path, archive path, current version, and pinned tag (`src/cli.rs`, `src/update.rs`).
- Add `useless version` and `useless update --check` to the manual-test list.
- Add wrong-case entries: updating `current_exe()` instead of `INSTALL_BIN_PATH`; adding a background auto-update path without signing/rollout design; publishing an asset name the updater cannot match.
- Add a correct-case entry: keep the archive contract (`useless-<target>.tar.gz`, binary at root) in sync across `scripts/package.sh`, the release workflow, and `src/update.rs`.
- Change the Linux cross-check guidance from `cargo check --tests --target x86_64-unknown-linux-gnu` to `cross check --all-targets --target x86_64-unknown-linux-gnu`, because `ring` needs a Linux C toolchain.

### `directory-structure.md`

- Add `src/update.rs` (self-update policy: CLI options, root gate, GitHub Releases, restart decision) to the `src/` layout.
- Add `.github/workflows/ci.yml` and `.github/workflows/release.yml` to the layout, and note that `scripts/package.sh` uses the same archive layout as the release workflow.
- Add `src/update.rs` to the one-file-one-responsibility list as "self-update policy and the GitHub release protocol".

## Rationale

The task adds a root-executed update path and the release pipeline that feeds it. The existing install contract stops at `install`/`reload`/`status`, so it would mislead future changes about where the updater writes, how assets are named, and which checks gate a release. The release archive contract is now shared by three places (`scripts/package.sh`, `.github/workflows/release.yml`, `src/update.rs`) and must be documented as one contract.

## Verification

- After applying: `grep -n "useless update" .phronia/spec/backend/daemon-install-contracts.md` and `grep -n "src/update.rs" .phronia/spec/backend/directory-structure.md` return hits.
- `grep -n "current_exe" .phronia/spec/backend/daemon-install-contracts.md` appears only in the wrong-case entry.
- The documented commands match `src/cli.rs` (`update`, `version`, `--version`, `--no-restart`).
- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` still pass (documentation-only delta).
