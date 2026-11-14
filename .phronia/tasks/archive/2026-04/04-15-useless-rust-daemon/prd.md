# brainstorm: useless rust daemon

## Goal

Build a Rust-based replacement for the current shell-based keep-alive toolkit so CPU, memory, and network occupation are coordinated inside one process, run at the lowest scheduler priority, and back off automatically when real user workloads become active.

## What I already know

* The current repository is mostly an imported shell toolkit under `Oracle-server-keep-alive-script-main/`.
* The current toolkit splits resource occupation into separate scripts for CPU, memory, and bandwidth, then wires them into systemd units.
* CPU occupation today is effectively unlimited inside the script itself and relies on `CPUQuota` in the systemd unit.
* Memory occupation today allocates a file in `/dev/shm` when observed memory usage is below 25%, then sleeps for 300 seconds.
* Network occupation today periodically downloads a large file with `wget`, using a rate derived from `speedtest-cli` or `speedtest-go`.
* You want the Rust binary to be named `useless`.
* Distribution is expected to be `tar.gz`; after extraction, users should be able to run `./bin/useless install`.
* Installation should prefer systemd and fall back to init.d when systemd is unavailable.

## Assumptions (temporary)

* MVP targets Linux only.
* Running as root during install is acceptable because service files and install paths must be written to system locations.
* The installed service will run as root unless we decide to create a dedicated system user later.
* User-experience protection is more important than maximizing synthetic resource occupation.

## Open Questions

* None at the moment. Ready for final confirmation.

## Requirements (evolving)

* Provide a single Rust executable named `useless`.
* Coordinate CPU, memory, and network occupation from one long-running process.
* Run work at the lowest Unix nice priority of `19`.
* Pause or reduce occupation when real workload on the host is high.
* Treat interactive usage as highest priority: SSH login sessions and recent terminal activity should quickly suppress synthetic load so the host does not feel laggy to a human operator.
* Avoid a continuous flat occupation pattern; synthetic load should run in randomized intermittent bursts so behavior looks less like a constant abuse workload.
* Support installation through `./bin/useless install`.
* Support `status`, `reload`, and `uninstall` operational commands in the MVP.
* Prefer systemd service installation; use init.d as fallback.
* Be distributable as a tarball without requiring Cargo on the target machine.
* Use an editable, low-friction config format rather than an overly engineered schema.
* Default config location should be `/etc/useless`.
* The installed daemon is a root-managed system service rather than a per-user service.
* Include simple resource profile presets in the MVP so operators can choose a less or more aggressive behavior without editing many knobs.
* Use a finer pause/resume policy than simple on/off so occupation can cool down smoothly after real user activity.
* Structure configuration around three top-level modules: CPU, memory, and network.
* Each module should have an independent enable/disable switch.
* CPU and memory modules should support configurable maximum percentage limits.
* The network module should support a configurable absolute bandwidth ceiling and must not exceed that ceiling while active.
* The scheduler may run CPU, memory, and network modules concurrently when policy allows, rather than forcing one-at-a-time rotation.
* Different resource modules can be managed by separate worker threads or tasks, coordinated by a shared supervisor.

## Acceptance Criteria (evolving)

* [ ] A Rust project exists in the repository and builds a binary named `useless`.
* [ ] `useless install` installs the executable into a system path and configures a service manager.
* [ ] On a host with systemd, installation writes a working unit and enables the service.
* [ ] On a host without systemd but with init.d, installation writes a working init script.
* [ ] The running daemon sets or inherits nice level `19`.
* [ ] CPU, memory, and network occupation can be started from one daemon process.
* [ ] The daemon can suspend or reduce occupation when host pressure crosses configured thresholds.
* [ ] The daemon suppresses or pauses occupation shortly after interactive activity is detected.
* [ ] Resource occupation occurs in randomized intervals rather than a fixed continuous pattern.
* [ ] Installation creates a human-editable config file with sensible defaults.
* [ ] Installed service reads config from `/etc/useless`.
* [ ] `useless status`, `useless reload`, and `useless uninstall` exist in the MVP.
* [ ] At least one preset-based configuration path exists for selecting aggression level.
* [ ] Config exposes separate CPU, memory, and network sections with module-level toggles and limits.
* [ ] CPU and memory enforcement respect configured maximum percentages.
* [ ] Network activity respects a configured absolute bandwidth cap.
* [ ] Multiple resource modules can run concurrently under supervisor control.

## Definition of Done (team quality bar)

* Tests added or updated where practical for resource policy and installer logic.
* `cargo fmt`, `cargo check`, and targeted tests pass.
* CLI usage and install flow are documented.
* Service install and uninstall paths are explicit and reversible.

## Technical Approach

Use a single Rust binary with subcommands for lifecycle management and a long-running daemon mode.

The daemon will have:

* a supervisor loop that samples host pressure and interactive session signals,
* a shared policy state with hysteresis and randomized duty windows,
* three independently configurable workers for CPU, memory, and network occupation,
* worker coordination that allows concurrent activity when the host is idle enough,
* process priority lowering to `nice 19` during daemon startup.

Configuration will be read from `/etc/useless`, with a simple human-editable file that exposes:

* global preset selection and scheduler knobs,
* `cpu`, `memory`, and `network` sections,
* per-module `enabled` switches,
* per-module ceilings such as percentage or bandwidth cap.

Operational commands in the MVP:

* `useless install`
* `useless uninstall`
* `useless reload`
* `useless status`

Installer behavior:

* copy the binary into a system path,
* create `/etc/useless` with default config,
* install a systemd unit when available,
* fall back to init.d script generation when systemd is unavailable.

## Decision (ADR-lite)

**Context**: The shell version uses separate scripts and service units, which makes coordinated throttling, user-priority protection, and intermittent scheduling weak and inconsistent.

**Decision**: Replace the shell toolkit with one Rust daemon that supervises three resource workers, supports concurrent multi-resource occupation, and uses shared pause/resume policy based on system pressure plus SSH/TTY activity.

**Consequences**:

* Better control and smoother behavior under real user activity.
* More internal complexity than one-script-per-resource design.
* Cleaner install and service management story.
* Easier future extension for presets, policy tuning, and richer status reporting.

## Out of Scope (explicit)

* Non-Linux platforms.
* GUI management or web dashboard.
* Packaging into native `.deb` or `.rpm` in the first iteration.
* Perfect detection of all human activity patterns.

## Technical Notes

* Current source material:
  * `Oracle-server-keep-alive-script-main/oalive.sh`
  * `Oracle-server-keep-alive-script-main/cpu-limit.sh`
  * `Oracle-server-keep-alive-script-main/memory-limit.sh`
  * `Oracle-server-keep-alive-script-main/bandwidth_occupier.sh`
  * `Oracle-server-keep-alive-script-main/*.service`
  * `Oracle-server-keep-alive-script-main/*.timer`
* Current install model downloads scripts to `/usr/local/bin` and writes systemd units directly under `/etc/systemd/system/`.
* Current bandwidth occupation is timer-driven, while CPU and memory are daemonized services.
* The Rust redesign likely needs:
  * a unified runtime loop,
  * install/uninstall subcommands,
  * service template generation,
  * OS pressure sensing based on `/proc` plus interactive session heuristics.
* Product direction confirmed so far:
  * Favor "pressure + interactive session" detection over pressure-only detection.
  * Protect SSH and terminal responsiveness ahead of synthetic occupation.
  * Prefer randomized intermittent occupation windows to avoid a constant abuse signature.
  * Prefer a simple editable config under `/etc/useless`.
  * Prefer a root-managed background service model.
  * Include operational commands plus preset-based behavior in the MVP.
  * Expose config around CPU / memory / network modules with independent toggles and limits.
  * Allow concurrent multi-resource occupation under shared policy control.
  * Do not introduce eBPF in the MVP, but keep the sensor layer abstract enough to support a future eBPF-backed observer.
