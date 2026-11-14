# Implementation plan: archive-only self-update selection

## Step 1 — Code

- [ ] `src/update.rs::configured_updater`: add `builder.asset_identifier(".tar.gz");` after `bin_path_in_archive(BIN_NAME)`.
- [ ] Extend `configures_the_install_target_and_archive_path` with `assert_eq!(updater.asset_identifier(), Some(".tar.gz"));`.
- [ ] Validation: `cargo test update::`.

## Step 2 — Full gates

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] Linux Rust 1.100 container clippy + tests.
- [ ] `omv sync --check --json`; `./scripts/package.sh`.

## Step 3 — Release

- [ ] Commit, integrate to `dev`, spec sync, archive, push `dev`.
- [ ] PR `dev` → `main`; wait for Release to publish `2611.14.3`.
- [ ] Confirm the release has four archives, four `.sha256` sidecars, and `install.sh`.

## Step 4 — Server validation

- [ ] Confirm 2611.14.2 is active and `/usr/local/bin/useless` is an ELF.
- [ ] `useless update --check` → `2611.14.2 -> 2611.14.3`.
- [ ] `sudo useless update` → success; verify:
  - `/usr/local/bin/useless` is a static aarch64 ELF (not text).
  - `useless version` = 2611.14.3.
  - `systemctl is-active useless` = active; no `Exec format error`; no start-limit failure.
  - CPU is near zero while suspended (no PSI spin).
- [ ] `useless update --check` again → up to date.
- [ ] Keep `/tmp/useless-2611.14.2/useless` as rollback; if the update fails, restore it and `systemctl reset-failed`.

## Step 5 — Spec delta

- [ ] Record in `spec-delta.md`: the updater sets `asset_identifier(".tar.gz")`; `.sha256`/`install.sh` are not installable assets; add a validation row and a regression-test expectation.
