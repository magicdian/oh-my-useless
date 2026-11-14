# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`

## Intended changes

### `daemon-install-contracts.md`

- Under **Update behavior**, add:
  - The updater sets `asset_identifier(".tar.gz")` and therefore selects only `useless-<target>.tar.gz`. The release also carries `useless-<target>.sha256` and `install.sh`; neither is an installable asset.
  - `self_update`'s GitHub asset matcher is substring-based on the target triple, and asset order is not guaranteed. Without the identifier, the `.sha256` sidecar can be selected and copied over `/usr/local/bin/useless`.
- Add a validation row:
  - A pinned or unpinned update must install an ELF from the tar.gz; selecting the `.sha256` sidecar is a failure.
- Add a unit-test expectation: `UpdateConfig::asset_identifier()` is `Some(".tar.gz")` in `configures_the_install_target_and_archive_path` (`src/update.rs`).
- Add wrong-case entry: rely on the target triple alone for asset selection.
- Add correct-case entry: set `asset_identifier(".tar.gz")` and keep the `bin_path_in_archive("useless")` contract.

## Rationale

A pinned update on 2611.14.2 selected `useless-aarch64-unknown-linux-musl.sha256` because it contains the target triple and appeared first in the release assets. The updater copied the checksum text to `/usr/local/bin/useless`; systemd then failed with `Exec format error`. The identifier narrows the match to the only installable artifact without changing the release asset contract.

## Verification

- After applying: `grep -n "asset_identifier" .phronia/spec/backend/daemon-install-contracts.md` returns the new contract lines.
- `cargo test update::` passes with the new assertion.
- Linux Rust 1.100 container: clippy and 96 tests pass.
- After 2611.14.3: `useless update` on the server leaves a static aarch64 ELF, `useless version` reports 2611.14.3, and the service is active.
