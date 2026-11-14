# Design: archive-only self-update selection

## 1. Root cause

`self_update::Release::asset_for(target, identifier)` selects the first asset whose name contains the target triple (and the identifier when set). The release asset order is not guaranteed. Both these assets contain `aarch64-unknown-linux-musl`:

- `useless-aarch64-unknown-linux-musl.tar.gz`
- `useless-aarch64-unknown-linux-musl.sha256`

The updater picked the `.sha256` file, treated it as a plain archive, and copied its text into `bin_install_path`.

## 2. Fix

In `src/update.rs::configured_updater`, add:

```rust
builder.asset_identifier(".tar.gz");
```

`asset_identifier` is a substring filter applied alongside the target filter. Only the tar.gz asset contains `.tar.gz`; the `.sha256` sidecar and `install.sh` do not. The identifier is set for both the update path and the `--check` path because they share `configured_updater`.

The `bin_path_in_archive("useless")` remains correct: the archive root contains `useless`, and `self_update` extracts that member.

## 3. Test

Extend `configures_the_install_target_and_archive_path`:

```rust
assert_eq!(updater.asset_identifier(), Some(".tar.gz"));
```

The `UpdateConfig` trait already exposes `asset_identifier()`. No network is involved.

## 4. Release and validation

1. Land the one-line change plus test on `dev`; run the full local and Linux container gates.
2. Merge to `main`; the OMV pipeline publishes `2611.14.3`.
3. On `oracle-sg-tx-forward`, 2611.14.2 is restored and active:
   - `useless update --check` → `2611.14.2 -> 2611.14.3`.
   - `sudo useless update` → downloads the tar.gz, verifies the GitHub digest, replaces the binary, restarts.
   - `file /usr/local/bin/useless` → static aarch64 ELF; `useless version` → 2611.14.3; `systemctl is-active useless` → active.
   - Keep `/tmp/useless-2611.14.2/useless` and the 2611.14.2 archive for rollback.
4. If the update fails, restore the 2611.14.2 binary, `systemctl reset-failed`, and restart.

## 5. Alternatives rejected

- **Rename the checksum assets.** The current naming is the release contract and the install script depends on it; changing it is larger than the updater fix.
- **Custom `asset_matcher`.** `self_update`'s GitHub builder exposes `asset_identifier`; a custom matcher is unnecessary for this constraint.
- **Hard-code the asset URL.** Bypasses the backend's target/version/digest verification and breaks rollback pinning.
