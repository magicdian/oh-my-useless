# Design: self-update and release pipeline

## 1. Dependency choice

| Option | Verdict |
|---|---|
| `self_update` 1.3.3 | Chosen. GitHub Releases backend, `tar.gz` support, GitHub per-asset SHA-256 digest verification (`checksums` feature), and `self-replace` underneath for the atomic swap. |
| `self-replace` alone | Rejected as the primary API. It is the right primitive (and is what `self_update` uses), but we would reimplement release listing, asset selection, download, and digest checks. |
| Custom HTTP + `tar` + `sha2` | Rejected. Fewer transitive crates, but all of the protocol is ours to get wrong. |

Cargo entry:

```toml
self_update = { version = "1.3", default-features = false, features = [
    "ureq", "rustls", "github", "progress-bar",
    "archive-tar", "compression-tar-gz", "checksums",
] }
```

The `ureq` client is deliberate. `self_update`'s default `reqwest` + `rustls` path enables `aws-lc-rs`, whose `aws-lc-sys` build needs CMake and a cross C toolchain; that makes aarch64 and musl release builds fragile. `ureq`'s `rustls` feature uses `ring`, which cross-compiles far more reliably, and it drops the entire tokio/hyper tree (the release binary fell from 8.6 MB to 5.5 MB). `rustls` is still required so musl builds do not depend on OpenSSL. MSRV is 1.88, so `rust-version = "1.88"` is recorded in `Cargo.toml`.

## 2. Module and boundary placement

```
src/update.rs   → update policy: CLI arguments, root gate, reporter, restart decision
src/install.rs  → service detection/restart, ensure_root (shared, pub(crate))
src/cli.rs      → Update/Version commands and flag parsing
src/main.rs     → dispatch
```

`update.rs` is the only module that knows about `self_update`. It does not read the daemon config and does not touch domain/runtime code. Service management stays in `install.rs`, which already owns systemd/init.d detection.

## 3. Update flow

```
parse args
  ├─ --check        → build updater, query latest release, print, exit 0 (no root, no write)
  └─ update         → require euid 0
                      ensure parent of INSTALL_BIN_PATH exists
                      build updater:
                        repo_owner = "magicdian"
                        repo_name  = "oh-my-useless"
                        bin_name   = "useless"
                        bin_install_path = "/usr/local/bin/useless"
                        bin_path_in_archive = "useless"
                        current_version = env!("CARGO_PKG_VERSION")
                        check_install_path_writable(true)
                        no_confirm(true), show_output(false)
                        show_download_progress(!quiet)
                        optional auth_token from USELESS_GITHUB_TOKEN
                        optional release_tag from --version
                      update()
                        ├─ UpToDate          → print, exit 0, no restart
                        └─ Updated(v)        → print old → v
                                               if !no_restart → install::restart_service_if_active()
```

`self_update` verifies the GitHub asset digest before extraction and replaces the destination atomically via `self-replace`, so a failed download or digest mismatch leaves the installed binary untouched.

### Version pinning

`--version` is normalised to a tag: strip one leading `v`, then prepend `v`. `--version 0.2.0` and `--version v0.2.0` both produce `v0.2.0`. Tags are semver by contract; a rolling tag fails with the crate's error, which we surface verbatim.

### Why the asset name matches

`self_update` selects an asset by checking whether the asset name contains the compile-time target triple (`x86_64-unknown-linux-gnu`, etc.), falling back to arch + OS tokens. `taiki-e/upload-rust-binary-action` names archives `$bin-$target.tar.gz` by default, yielding exactly `useless-x86_64-unknown-linux-gnu.tar.gz`. `bin_path_in_archive = "useless"` matches an archive whose root holds the executable (the action's default, `leading-dir: false`).

If a release lacks the matching target, selection fails loudly. We do not add a fallback to a foreign architecture; running the wrong arch is worse than a clear error.

### Root gate

`install::ensure_root()` becomes `pub(crate)` and is reused. It is called before constructing the updater, so a non-root `update` fails before any network request. `--check` deliberately skips it.

### Restart semantics

Add `install::restart_service_if_active() -> AppResult<bool>`:

- systemd: `systemctl is-active --quiet useless`; if active, `systemctl restart useless`.
- init.d: script exists and `status` succeeds; if active, run `restart`.
- none: return `Ok(false)`.

A restart failure returns an error, but `update` has already succeeded; the command prints "updated to X but restart failed: …" and exits non-zero so scripts notice. `--no-restart` skips the call entirely. The running daemon keeps its old inode until the restart, which is the reason this step is not optional-by-accident.

## 4. CLI surface

```
useless update [--check] [--version <semver>] [--no-restart]
useless version
useless --version | -V
```

`--version` is only an option of `update`; at the top level it prints the version. Parsing stays hand-written in `cli.rs` to match the existing style (no clap).

Exit codes: `0` for success and up-to-date; `1` for errors, including a failed restart after a successful install.

## 5. GitHub Actions design

Two workflows, both least-privilege.

### `.github/workflows/ci.yml`

- Triggers: `pull_request`, `push` to `main` and `dev`.
- `concurrency` keyed by workflow + ref, `cancel-in-progress: true`.
- `permissions: contents: read`.
- `linux` job (`ubuntu-latest`): checkout → `dtolnay/rust-toolchain@stable` with `rustfmt, clippy` → `Swatinem/rust-cache@v2` → `cargo fmt --all --check` → `cargo clippy --all-targets --locked -- -D warnings` → `cargo test --all-targets --locked`.
- `macos` job (`macos-latest`): checkout → toolchain → cache → `cargo check --all-targets --locked` → `cargo test --all-targets --locked`. This preserves the repository's stated non-Linux compile support without pretending macOS is a production target.

The seven existing clippy findings are fixed first so `-D warnings` is a real gate.

### `.github/workflows/release.yml`

- Triggers: `push` tags matching `v[0-9]+.*`; `workflow_dispatch` is intentionally omitted so releases always map to a tag.
- `permissions: contents: read` at workflow level; `contents: write` only on the two jobs that need it.
- Jobs:
  1. `test` — identical commands to the CI Linux job. Release builds must not ship untested code.
  2. `create-release` — `needs: test`, `actions/checkout@v7`, `taiki-e/create-gh-release-action@v1`. Creates the draft/live release from the tag.
  3. `upload` — `needs: [test, create-release]`, matrix over the four targets, `ubuntu-latest`, `taiki-e/upload-rust-binary-action@v1` with:
     - `bin: useless`
     - `target: ${{ matrix.target }}`
     - `archive: useless-$target`
     - `checksum: sha256`
     - `include: README.md,LICENSE`
     - `token: ${{ secrets.GITHUB_TOKEN }}`
     The action invokes `cross` automatically for non-host targets, so aarch64 and musl need no extra setup.
- A version guard runs before `create-release`:

  ```bash
  version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
  test "v$version" = "$GITHUB_REF_NAME"
  ```

  This prevents the updater's semver comparison from looping on a tag that disagrees with `CARGO_PKG_VERSION`.

Matrix: `fail-fast: false` so one target failure does not hide the others.

Artifacts per target: `useless-<target>.tar.gz` (contains `useless`, `README.md`, `LICENSE` at the root) and `useless-<target>.tar.gz.sha256`. The updater relies on GitHub's per-asset digest; the sidecar is for humans and CI.

## 6. Trust and failure model

| Concern | Handling |
|---|---|
| Corrupted or truncated download | GitHub per-asset SHA-256 digest verified by `self_update` before extraction. |
| Wrong architecture asset | Exact target substring match; no arch fallback. |
| Non-root invocation | Refused before download. |
| Running daemon holds the old inode | Explicit restart after install; `--no-restart` documented. |
| Network failure mid-download | `self-replace` never runs; installed binary unchanged. |
| GitHub rate limit | Optional `USELESS_GITHUB_TOKEN`; errors are surfaced verbatim. |
| Compromised release | Out of scope for this iteration. Digest is integrity, not authenticity; zipsign/cosign is a follow-up. |
| Accidental dev-build update | `bin_install_path` is fixed to `/usr/local/bin/useless`. |
| Rollback | Reinstall a pinned release: `useless update --version <older>`. |

## 7. Testing strategy

- Pure parsing tests for every new CLI variant (AC3).
- A unit test for the version-tag normalisation (`0.2.0` / `v0.2.0`).
- No test performs network I/O. The update path is exercised by hand against a real release, or simulated at the argument/config layer.
- Local acceptance: `./scripts/package.sh` output inspected with `tar -tzf` to confirm root layout, then `cargo fmt/clippy/test`.
- Workflow files validated as YAML in the implementation step (Python YAML parse) because `actionlint` is not guaranteed on the developer machine.
- On first real tag, a maintainer runs `useless update --check` on a test host before `useless update`.

## 8. Alternatives and rejected scope

- **cargo-dist instead of taiki-e actions.** cargo-dist is mature and generates installers, but it owns project metadata and produces `.tar.xz` by default. The `self_update` + taiki-e pairing keeps the asset contract explicit and the repository free of generated installer logic.
- **Systemd timer for automatic updates.** Rejected for now: a root daemon that installs code unattended needs signing and a rollout story, neither of which exists yet.
- **`update-informer`.** Checks for updates but cannot install; it would add a second updater concept.
