# Journal - jdjingdian (Part 1)

> AI development session journal
> Started: 2026-11-14

---



## Session 1: Self-update and release pipeline
<!-- phronia-session: v=2 fp=1a4f87f427172428 -->

**Date**: 2026-11-14
**Task**: Self-update and release pipeline
**Branch**: `dev`

### Summary

Added a root-safe self-updater and a tag-driven GitHub Actions release pipeline, then archived the task into dev.

### Main Changes

- Added useless update/version backed by self_update (ureq + rustls/ring, GitHub asset digest, root gate, --check/--version/--no-restart, service restart).
- Added CI and release workflows: fmt/clippy/tests, version guard, four Linux targets with sha256 sidecars.
- Aligned scripts/package.sh with the release archive layout and fixed seven legacy clippy findings.

### Git Commits

| Hash | Message |
|------|---------|
| `7a53383` | feat(update): add self-update and GitHub Actions release pipeline |
| `cb38865` | chore(task): record self-update plan and spec delta |

### Testing

- [OK] cargo fmt --all --check, cargo clippy --all-targets --locked -- -D warnings, cargo test --all-targets --locked (77 passed).
- [OK] package.sh produced the expected root layout and checksum; workflow YAML parsed; update --check reached GitHub; non-root update refused before network.

### Status

[OK] **Completed**

### Next Steps

- Push dev or tag v0.1.0 to exercise the release workflow, then verify update end to end on a test host.


## Session 2: OMV pipeline versioning
<!-- phronia-session: v=2 fp=781e8b13c1e4271d -->

**Date**: 2026-11-14
**Task**: OMV pipeline versioning
**Branch**: `dev`

### Summary

Adopted OMV as the single version truth in pipeline mode and moved release assignment into GitHub Actions.

### Main Changes

- Bootstrapped .omv/ at 2611.14.0 (pipeline mode) and synced Cargo.toml, Cargo.lock, and src/generated/version.rs; first pipeline release is 2611.14.1.
- Added a CI OMV drift job and PR version-truth guard; replaced the tag-triggered release with a main/manual bump→commit→tag→upload workflow.
- Normalized self-update version pinning to bare OMV tags and updated README/spec.

### Git Commits

| Hash | Message |
|------|---------|
| `05413ec` | feat(release): adopt OMV pipeline versioning |
| `e26b0cd` | chore(task): record OMV pipeline plan and spec delta |

### Testing

- [OK] omv sync --check exits 0; dry-run bump plans 2611.14.1; unauthorized bump/sync are denied.
- [OK] cargo fmt/clippy/test pass (77 tests); package.sh, workflow YAML, and a full release transaction simulation succeed.

### Status

[OK] **Completed**

### Next Steps

- Push dev, merge into main, and watch the Release workflow produce 2611.14.1.


## Session 3: Unblock and ship the OMV release pipeline
<!-- phronia-session: v=2 fp=3cfd3df44ff3cbbb -->

**Date**: 2026-11-14
**Task**: Unblock and ship the OMV release pipeline
**Branch**: `dev`

### Summary

Fixed the two CI blockers and published the first OMV-managed release, 2611.14.1.

### Main Changes

- Satisfied Rust 1.100 clippy lints in the Linux-only PSI/session/reactor code and replaced the fake PSI epoll descriptor (/dev/null returns EPERM) with eventfd.
- The Release workflow bumped 2611.14.0 to 2611.14.1, committed chore(release), tagged it, and uploaded four Linux archives with sha256 sums.

### Git Commits

| Hash | Message |
|------|---------|
| `38e3d29` | fix(ci): satisfy clippy 1.100 lints on Linux paths |
| `2a5c7d8` | fix(test): use eventfd for fake PSI watches |
| `fe41833` | chore(release): 2611.14.1 |

### Testing

- [OK] Linux Rust 1.100 container: clippy -D warnings and all 94 tests pass.
- [OK] dev CI green; Release run green; useless update --check reports 2611.14.0 -> 2611.14.1.

### Status

[OK] **Completed**

### Next Steps

- Continue work on dev at 2611.14.1; the next main merge produces 2611.14.2.
