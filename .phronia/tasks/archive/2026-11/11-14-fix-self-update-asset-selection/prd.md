# fix: self-update must select the tar.gz asset

## Goal

Make `useless update` select only `useless-<target>.tar.gz` from a GitHub release. A pinned update on `oracle-sg-tx-forward` selected the `.sha256` sidecar instead, wrote its text over `/usr/local/bin/useless`, and left the service failing to exec.

## Background

On 2611.14.2, `sudo useless update --version 2611.14.2` reported success and restarted the service, but `/usr/local/bin/useless` became a one-line file whose first line was the SHA-256 digest. systemd logged `Failed to execute /usr/local/bin/useless: Exec format error` and entered restart failures. The binary was restored from the verified archive.

`self_update`'s GitHub backend matches release assets with `asset.name.contains(target)` and an optional `asset_identifier` substring. GitHub returns assets in an order that placed `useless-aarch64-unknown-linux-musl.sha256` before `useless-aarch64-unknown-linux-musl.tar.gz`. Both contain the target triple, so the first match won. The installer then treated the checksum file as a plain archive and copied it into the install path.

The release also contains `install.sh`, which does not contain the target triple, so it is not a candidate for the running target; the `.sha256` sidecar is the dangerous one.

## Requirements

**R1 — Archive-only selection.** `configured_updater` must set `asset_identifier(".tar.gz")`. This excludes `useless-<target>.sha256` and any other non-archive asset while still matching `useless-<target>.tar.gz`.

**R2 — Check path.** The identifier is set for both the update and `--check` builders, so a future change cannot reintroduce the mismatch on one path.

**R3 — Regression test.** Extend `configures_the_install_target_and_archive_path` to assert `UpdateConfig::asset_identifier() == Some(".tar.gz")`.

**R4 — Release.** Land on `dev`, merge to `main`, and publish 2611.14.3 through the OMV pipeline.

**R5 — End-to-end validation.** On `oracle-sg-tx-forward`, keep 2611.14.2 installed, then run `useless update --check` (expect 2611.14.3) and `sudo useless update` (unpinned). Verify the downloaded file is the tar.gz (the binary is a static aarch64 ELF after replacement), the version is 2611.14.3, and the service is active with no start-limit failure. Keep the 2611.14.2 archive/binary as rollback.

**R6 — No install-script regression.** The release workflow already uploads `install.sh`; it must remain a release asset and must not be selected by the updater.

## Acceptance Criteria

- [ ] AC1 `configured_updater(...).asset_identifier()` is `Some(".tar.gz")`.
- [ ] AC2 `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` pass; the Linux Rust 1.100 container suite passes.
- [ ] AC3 Release 2611.14.3 contains the four Linux archives, their `.sha256` sidecars, and `install.sh`.
- [ ] AC4 On the server, `useless update --check` reports `2611.14.2 -> 2611.14.3`.
- [ ] AC5 `sudo useless update` replaces the binary with an ELF, `useless version` reports 2611.14.3, and the service is active.
- [ ] AC6 Rollback to 2611.14.2 is available from `/tmp/useless-2611.14.2/useless` and the release archive.

## Out of scope

- Changing the asset naming contract.
- Signing release assets.
- Pinning a specific asset URL instead of using the GitHub backend.

## Notes

- `self_update`'s asset matcher is substring-based and does not know file types; the identifier is the supported way to disambiguate.
- `--version` accepts bare and `v`-prefixed tags; the asset identifier is independent of the tag.
