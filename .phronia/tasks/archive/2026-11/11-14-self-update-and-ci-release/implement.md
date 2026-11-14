# Implementation plan: self-update and release pipeline

Ordered checklist. Each step ends with a validation command; stop and fix before moving on.

## Step 1 — Green baseline

- [ ] Fix the seven pre-existing clippy findings so `-D warnings` is usable:
  - `src/adapters/cgroups.rs:30` — collapsible `if`
  - `src/adapters/netstat.rs:55` — redundant closure, use `std::io::Error::other`
  - `src/adapters/procfs.rs:337` — manual char comparison → `[')', ',', ':', ']']`
  - `src/cli.rs:29` — `while let` → `for flag in args.by_ref()`
  - `src/daemon.rs:134`, `src/daemon.rs:164`, `src/daemon.rs:449` — collapsible `if`
- [ ] Baseline command: `cargo fmt --all && cargo clippy --all-targets --locked -- -D warnings && cargo test --all-targets --locked`

Review gate: no behavior change, only lint-shaped rewrites. `cargo test` stays at 70 passing.

## Step 2 — Crate metadata and dependency

- [ ] `Cargo.toml`: add `description`, `repository`, `license = "Apache-2.0"`, `rust-version = "1.88"`.
- [ ] Add `self_update = { version = "1.3", features = ["checksums"] }`.
- [ ] `cargo update -p useless` is unnecessary; run `cargo check` to resolve the lockfile.
- [ ] Validation: `cargo check --locked` (regenerate `Cargo.lock` first with `cargo check`, then `git diff --stat Cargo.lock`).

Review gate: inspect `cargo tree -p self_update` to confirm `rustls` and no `openssl-sys`.

## Step 3 — CLI surface

- [ ] `src/cli.rs`
  - Add `Command::Update { check, version, no_restart }` and `Command::Version`.
  - Parse `update [--check] [--version <semver>] [--no-restart]`.
  - Parse top-level `version` and `--version` / `-V`.
  - Add both to `usage()`.
- [ ] `src/main.rs` — add `mod update;` and dispatch.
  - `Command::Version` → `println!("useless {}", env!("CARGO_PKG_VERSION"))`.
  - `Command::Update { .. }` → `update::run(...)`.
- [ ] Add `#[cfg(test)]` parsing tests: default update, `--check`, `--version 0.2.0`, `--version v0.2.0`, `--no-restart`, `version`, `--version`, unknown flag rejection.

Validation: `cargo test cli::` and `cargo run -- help`.

## Step 4 — Service restart helpers

- [ ] `src/install.rs`
  - `pub(crate) fn ensure_root()`.
  - `pub(crate) fn restart_service_if_active() -> AppResult<bool>` implementing the systemd/init.d/none matrix from `design.md`.
  - Keep `ServiceManager` detection private; only the new function is exposed.
- [ ] Unit-test the pure parts only (manager selection cannot be unit-tested portably). No new test required if the function is thin composition.

Validation: `cargo check` (Linux-only branches must still compile on macOS; guard with existing `cfg` patterns if needed).

## Step 5 — Update module

- [ ] New `src/update.rs`:
  - `pub struct UpdateOptions { check: bool, version: Option<String>, no_restart: bool }`
  - `pub fn run(options: UpdateOptions) -> AppResult<()>`
  - Constants: `REPO_OWNER = "magicdian"`, `REPO_NAME = "oh-my-useless"`, `BIN_NAME = "useless"`.
  - Build the updater exactly as in `design.md` §3; set `bin_path_in_archive(BIN_NAME)` explicitly.
  - `--check`: use `is_update_available()` / `get_latest_release()`; print installed + latest; return `Ok(())`.
  - Non-check: `ensure_root()`, `create_dir_all` the install parent, run `update()`, match `VersionStatus`.
  - Optional token: `std::env::var("USELESS_GITHUB_TOKEN")`.
  - `--version`: normalise with a small pure helper and add a unit test.
  - Restart decision: only when updated and `!no_restart`; report restart failure as a distinct error after printing the successful install.
- [ ] Error mapping: `self_update` errors are `std::error::Error + Send + Sync`; if the `?` conversion fails to compile, wrap with `boxed_error(format!("update failed: {error}"))`.
- [ ] Do not print secrets; do not log tokens.

Validation: `cargo test update::` (tag normalisation), `cargo clippy --all-targets -- -D warnings`, and a manual `cargo run -- update --check` (network-dependent; failure is acceptable offline, but the error must be clear and must not panic).

## Step 6 — Packaging alignment

- [ ] Rewrite `scripts/package.sh`:
  - Detect target: `${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}`.
  - `cargo build --release --locked`.
  - Stage `useless`, `README.md`, `LICENSE` at the archive root (no `bin/`, no version directory).
  - Emit `dist/useless-$TARGET.tar.gz` and `dist/useless-$TARGET.tar.gz.sha256`.
  - Keep it POSIX-friendly bash, `set -euo pipefail`.
- [ ] Validation: `./scripts/package.sh && tar -tzf dist/useless-$(rustc -vV | sed -n 's/^host: //p').tar.gz && sha256sum -c` (on macOS, `shasum -a 256 -c`; write the script to use `shasum`/`sha256sum` whichever exists).

Review gate: archive layout matches the CI artifact contract in `design.md` §5.

## Step 7 — CI workflow

- [ ] Create `.github/workflows/ci.yml` per `design.md` §5.
- [ ] Use current stable action majors: `actions/checkout@v7`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`.
- [ ] Validation: parse YAML with Python; confirm job names and commands with `grep`.

## Step 8 — Release workflow

- [ ] Create `.github/workflows/release.yml` per `design.md` §5.
- [ ] Jobs: `test`, `create-release`, `upload` (four-target matrix).
- [ ] Add the tag/version guard before release creation.
- [ ] `permissions` least-privilege; `contents: write` only where needed.
- [ ] Validation: parse YAML; grep the four targets and the guard.

## Step 9 — Documentation

- [ ] `README.md`
  - Add `useless version` / `useless --version` to Operations.
  - Add an **Updating** section: root requirement, `--check`, `--version`, `--no-restart`, restart behavior, token env var, rollback by pinning.
  - Add a **Releases and CI** section: tag flow, target table, artifact layout, manual `sha256sum -c`, recommendation of musl for older hosts, glibc caveat.
  - Note that `scripts/package.sh` now mirrors the release layout.
- [ ] Validation: markdown renders; no stale references to `bin/useless`.

## Step 10 — Final verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] `./scripts/package.sh` and inspect the archive + checksum.
- [ ] `python3 -c "import yaml,sys; [yaml.safe_load(open(p)) for p in sys.argv[1:]]" .github/workflows/*.yml`
- [ ] `cargo run -- version` and `cargo run -- help`
- [ ] `cargo run -- update --check` (expect a clear network/unsupported error or a real check result; must not panic)
- [ ] `git diff --stat` review; ensure `.phronia` task artifacts are the only unrelated changes.

## Rollback points

- After Step 1: `git checkout -- src/` restores the lint baseline.
- After Step 2: remove the `self_update` dependency and `cargo update`.
- After Step 5: delete `src/update.rs` and revert `cli.rs`/`main.rs`; no schema or on-disk state changes are involved.
- Workflows and docs are additive; deleting the two files and reverting README is safe.
- There is no data migration and no config-key change, so runtime rollback is just reinstalling the previous binary (`useless update --version <older>`).
