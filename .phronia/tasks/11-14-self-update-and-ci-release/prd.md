# feat: self-update and GitHub Actions release pipeline

## Goal

Give an installed `useless` a supported way to move itself to a newer published release, and make those releases reproducible from a git tag through GitHub Actions.

The user-visible outcomes:

- `useless update --check` reports the installed and latest versions without changing anything.
- `useless update` replaces `/usr/local/bin/useless` with the latest GitHub Release asset for the running target, verifies the artifact digest, and restarts the service if it is active.
- Pushing a `v*` tag runs fmt, clippy, and tests, then publishes Linux archives plus sha256 sums that the updater can consume.

## Background

The repository is a single Rust binary crate (`useless`) that installs itself as a systemd unit or an init.d script and runs as root from `/usr/local/bin/useless`. Upgrades are manual today: extract a new archive, rerun `useless install`, restart the service. There is no CI, no release workflow, and no version command.

Constraints inherited from the daemon's safety model:

- The daemon runs as root and is the first process the host sacrifices. An update path that executes unauthenticated or unreviewed code is a larger risk than a stale binary.
- Production targets are Linux only. The crate compiles on other platforms; the release surface must stay Linux.
- `scripts/package.sh` is the only packaging path today. Its archive layout (`bin/useless` inside a version-named directory) does not match what an updater expects.

## Requirements

**R1 — Mature updater dependency.** Use `self_update` for the GitHub Release protocol, download, digest verification, and atomic replacement. Do not hand-roll the GitHub API or the replace step.

**R2 — Update is explicit and root-gated.** No background or automatic installation. `useless update` requires an effective uid of 0 before it writes; `--check` works unprivileged and performs no write.

**R3 — Correct install target.** The updater writes `/usr/local/bin/useless` (`INSTALL_BIN_PATH`), not whichever executable happened to be invoked, so running a development build cannot silently replace the installed daemon.

**R4 — Integrity check.** Downloaded archives are verified against the SHA-256 digest GitHub publishes for each release asset. A digest mismatch aborts before extraction.

**R5 — Version pinning and no-op semantics.** `--version <semver>` installs a specific release, accepting both `0.2.0` and `v0.2.0`. When the installed version is already current, the command exits successfully without writing or restarting.

**R6 — Service restart.** After a successful replacement, restart the detected service manager (systemd, init.d) if the service is active. `--no-restart` suppresses this. A failed restart must not be reported as a failed update.

**R7 — Release asset contract.** For each published target `T`, the release contains exactly one archive named `useless-T.tar.gz` whose root contains the `useless` executable, plus `README.md` and `LICENSE`. A sha256 sidecar is published for manual verification. The four targets are:

| Target | Purpose |
|---|---|
| `x86_64-unknown-linux-gnu` | x86_64 hosts with a recent glibc |
| `x86_64-unknown-linux-musl` | static x86_64, recommended for mixed/older hosts |
| `aarch64-unknown-linux-gnu` | ARM64 servers and Raspberry Pi OS |
| `aarch64-unknown-linux-musl` | static ARM64 |

Publishing every target matters because `self_update` matches the asset against the compile-time target of the running binary.

**R8 — Tag-driven release.** A `v<semver>` tag runs the same checks as CI, verifies that the tag matches `CARGO_PKG_VERSION`, builds all four targets, and uploads archives, checksums, and extra files to a GitHub Release.

**R9 — CI gate.** Pull requests and pushes to `main`/`dev` run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all-targets --locked` on Linux; a macOS job keeps the non-Linux compile path honest. The repository currently has seven pre-existing clippy findings that must be fixed for this gate to be meaningful.

**R10 — Documentation.** README documents `useless version`, `useless update` flags, the artifact/target table, manual checksum verification, the root/systemd interaction, and how to cut a release.

## Acceptance Criteria

- [ ] AC1 `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` pass on the working tree.
- [ ] AC2 `useless version` and `useless --version` print the crate version; `useless help` lists `update` and the new flags.
- [ ] AC3 Unit tests cover CLI parsing for `update`, `update --check`, `update --version`, `update --no-restart`, `version`, and `--version`.
- [ ] AC4 `useless update` from a non-root uid fails with a permission message before any network download; `useless update --check` does not require root.
- [ ] AC5 The updater is configured with `bin_install_path = /usr/local/bin/useless`, `bin_name = useless`, `bin_path_in_archive = useless`, the compile-time version, and digest verification enabled.
- [ ] AC6 `--version vX.Y.Z` and `--version X.Y.Z` produce the same `release_tag("vX.Y.Z")`.
- [ ] AC7 After a successful update, an active systemd service is restarted; an inactive or absent service produces a warning, not an error. `--no-restart` skips the attempt.
- [ ] AC8 `.github/workflows/ci.yml` and `.github/workflows/release.yml` exist, parse as YAML, and contain the required jobs (ci: fmt/clippy/test + macOS check; release: test gate, version guard, four-target build/upload).
- [ ] AC9 `scripts/package.sh` produces the same root layout and archive name as CI for the host target, so local artifacts and release artifacts do not diverge.
- [ ] AC10 README contains the update/release instructions and the target table.

## Out of scope

- Automatic/background updates, systemd timers, staged rollouts, or update channels.
- Cryptographic signing (zipsign/cosign) of release artifacts. Digest verification is integrity, not authenticity; signing is a follow-up.
- Windows and macOS release artifacts.
- Publishing to crates.io, Homebrew, apt, or other package managers.

## Notes

- `self_update` 1.3.3 requires Rust 1.88 and edition 2024; the local toolchain is 1.94.
- The daemon's systemd unit uses `Restart=on-failure`; replacing the file does not restart the running process, which is why R6 exists.
- A release published without the matching target asset leaves that installed variant unable to self-update; the error must say so instead of falling back to a foreign architecture.
