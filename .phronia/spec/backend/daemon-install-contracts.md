# Daemon Install Contracts

> Executable contracts for the `useless` CLI, config file, service-manager integration, and daemon runtime.

---

## Scenario: Rust Daemon MVP

### 1. Scope / Trigger

Use this spec when a task changes any of the following:

* CLI subcommands in `src/cli.rs` or `src/main.rs`
* Config fields or defaults in `src/config.rs`
* systemd or init.d integration in `src/install.rs`
* Self-update behavior in `src/update.rs` or the release workflow under `.github/workflows/`
* Daemon lifecycle, PID, signal, or reload behavior in `src/daemon.rs`
* Policy transitions in `src/domain/` or observation sources in `src/adapters/`
* Sensor/runtime behavior that changes operator-visible state transitions

This is infra/cross-layer because one change can affect:

* the operator command surface,
* `/etc/useless/config.toml`,
* `/usr/local/bin/useless`,
* `/etc/systemd/system/useless.service` or `/etc/init.d/useless`,
* `/run/useless.pid`,
* runtime pause/resume behavior.

---

### 2. Signatures

#### Binary path

* Installed binary: `/usr/local/bin/useless`

#### CLI commands

* `useless install [--config <path>]`
* `useless uninstall [--purge|--purge-config]`
* `useless reload`
* `useless status [--config <path>]`
* `useless daemon [--config <path>]`
* `useless update [--check] [--version <semver>] [--no-restart]`
* `useless version` (also `--version`, `-V`)
* `useless print-default-config`
* `useless help`

#### Service-manager outputs

* systemd unit path: `/etc/systemd/system/useless.service`
* init.d script path: `/etc/init.d/useless`
* PID file path: `/run/useless.pid`

#### Default config path

* `/etc/useless/config.toml`

---

### 3. Contracts

#### Config fields

Global fields currently supported:

* `preset`
* `check_interval_secs`
* `interactive_grace_secs`
* `active_tty_idle_secs`
* `caution_duty_percent`
* `caution_load_percent`
* `pause_load_percent`
* `caution_mem_available_percent`
* `pause_mem_available_percent`
* `pressure_enter_ms`
* `pressure_window_ms`
* `resume_quiet_secs`
* `resume_ramp_secs`
* `reserve_mb`
* `reserve_percent`

CPU section:

* `[cpu].enabled`
* `[cpu].max_percent`
* `[cpu].burst_secs_min`
* `[cpu].burst_secs_max`
* `[cpu].rest_secs_min`
* `[cpu].rest_secs_max`

Memory section:

* `[memory].enabled`
* `[memory].max_percent`
* `[memory].max_absolute_mb`
* `[memory].chunk_mb`
* `[memory].burst_secs_min`
* `[memory].burst_secs_max`
* `[memory].rest_secs_min`
* `[memory].rest_secs_max`

Network section:

* `[network].enabled`
* `[network].max_mbps`
* `[network].external_traffic_kbps`
* `[network].probe_window_ms`
* `[network].connect_timeout_secs`
* `[network].burst_secs_min`
* `[network].burst_secs_max`
* `[network].rest_secs_min`
* `[network].rest_secs_max`
* `[network].targets`

Sensor section:

* `[sensor].watch_slices`

#### Runtime behavior

* Policy evaluation is pure: `domain::reduce(state, signal, config, now)` performs no IO and reads no clock.
* Adapters emit `Signal` values and never call the reducer; workers consume directives from `runtime::Store` and never compute policy.
* The procfs sampler in `src/adapters/procfs.rs` is the current observation source and the permanent fallback for later event-driven sources.
* The daemon samples host state every `check_interval_secs`.
* Session detection order: inotify on `/run/systemd/sessions` (preferred), inotify on `/var/run/utmp` (fallback), then the `who -u --ips` + `ps -eo args` sampler at `check_interval_secs`.
* The normal logind path only reads session files and must not spawn `who`/`ps`. The utmp backend runs the parser on watch events; the sampler is the last resort.
* Session identity is the tty name (`pts/N` or `ttyN`); duplicate rows for the same tty count once.
* A tty atime probe runs only while sessions exist: 5 s while any session is active, 30 s otherwise. It re-emits a `SessionSet` filtered to sessions within `active_tty_idle_secs`, and re-emits non-empty sets every tick so `last_interactive` stays fresh. A failed `stat` counts as active.
* A tty that goes idle leaves the active set but stays tracked; the next probe can re-activate it without a new login event.
* A non-empty session set keeps `interactive_active` true regardless of the grace timer; `interactive_grace_secs` only governs resume after logout, followed by `resume_quiet_secs` and the slow-start ramp.
* PSI triggers on `/proc/pressure/{cpu,memory,io}` drive suspension. A pressure event suspends immediately, and the host stays suspended for `resume_quiet_secs` after the last event before resuming.
* After a suspension, duty ramps through 25%, 50%, and 100% across `resume_ramp_secs`.
* When PSI is unavailable or trigger registration fails, the procfs sampler remains the fallback and `status` reports `psi=unavailable`.
* `SIGHUP` reloads the config and rebuilds the reactor; `SIGINT`/`SIGTERM` shut down. On Linux these are read through `signalfd` after `SIG_BLOCK`.
* `useless status` appends a `daemon_state:` block read from `/run/useless/state`: `mode`, `reasons`, `scope`, `budget_mb`, `reserve_mb`, `available_mb`, `capabilities` (including `sessions=inotify|who`), `last_transition`, and `summary`.
* State and journal paths are `/run/useless/state` (line-based `key=value`) and `/run/useless/journal.jsonl` (transition-only JSON Lines).
* The memory budget is `min(MemTotal × max_percent × duty, MemAvailable − reserve, max_absolute_mb)`, where `reserve = max(reserve_mb, MemTotal × reserve_percent / 100)` and `0` disables the absolute cap.
* The memory worker is a chunked `mmap` ledger: it grows one `chunk_mb` at a time only when a fresh `MemAvailable − reserve >= current + chunk` check passes, shrinks with `MADV_DONTNEED` + `munmap`, and releases everything when the directive is not allowed.
* The daemon watches each configured cgroup slice's `{cpu,memory,io}.pressure`, excludes its own cgroup and ancestors, and refreshes the slice set every 60 s and on `SIGHUP`.
* Per-cgroup pressure writes the slice name to `scope` in the state file and appends `scope=<slice>` to the summary.
* Before each network burst, the worker samples interface counters twice over `probe_window_ms`, subtracts its own bytes, and computes ingress/egress kbit/s.
* If either direction exceeds `external_traffic_kbps`, the burst is skipped and logged; the next schedule retries. No mode transition is involved.
* A counter read failure leaves the gate open, logs once, and `max_mbps` still caps the burst.
* Worker scheduling uses randomized burst/rest windows from the relevant module section.

#### Install behavior

* `install` must require root.
* `install` must ensure a config file exists before enabling the service.
* systemd is preferred when `/run/systemd/system` exists and `systemctl` is available.
* init.d is the fallback when `/etc/init.d` exists.
* The systemd unit must contain `OOMScoreAdjust=1000`, `IOSchedulingClass=idle`, `CPUWeight=10`, and `Nice=19`.
* The unit's `MemoryHigh` is `ceiling + 64M` and `MemoryMax` is `ceiling + 128M`, where `ceiling = min(MemTotal × memory.max_percent / 100, memory.max_absolute_mb?)`, clamped to at least 64 MB; a disabled memory module yields 64 MB.
* The unit uses `Restart=on-failure`, `RestartSec=10`, `StartLimitIntervalSec=300`, and `StartLimitBurst=5`.
* `install` regenerates the unit from the current config; `reload` does not.
* `uninstall --purge` may remove `/etc/useless`; plain `uninstall` must leave config in place.

#### Update behavior

* Updates are explicit. There is no background updater, timer, or automatic install.
* The updater is the `self_update` crate's GitHub backend with `checksums` enabled, so the GitHub per-asset SHA-256 digest is verified before extraction.
* Release assets must be named `useless-<target>.tar.gz`, contain `useless` at the archive root, and exist for every target the project publishes. The running binary matches the asset by its compile-time target triple.
* `useless update --check` performs no write and does not require root.
* `useless update` requires root and writes `/usr/local/bin/useless` (`INSTALL_BIN_PATH`), not `current_exe()`.
* `--version` accepts `X.Y.Z` and `vX.Y.Z`; it pins the release tag and may install an older version (rollback).
* After a successful install, restart the service only when it is active. `--no-restart` skips the restart. A failed restart is reported as an error after the binary has already been replaced.
* `USELESS_GITHUB_TOKEN`, when set and non-empty, is passed to the GitHub API.
* Tag-driven releases must reject a tag that does not match `CARGO_PKG_VERSION`, because the updater compares semver values.

---

### 4. Validation & Error Matrix

| Case | Expected behavior |
|------|-------------------|
| Unsupported CLI subcommand | Return `AppResult` error with the bad command name |
| Missing `--config` value | Return `AppResult` error with explicit message |
| Unknown config key | Return parse error with line number and section/key context |
| `active_tty_idle_secs = 0` | Config validation must fail |
| Empty `network.targets` while network is enabled | Config validation must fail |
| Non-root `install` or `uninstall` | Fail with explicit root-required error |
| Non-root `useless update` | Fail with explicit root-required error before any network request |
| Non-root `useless update --check` | Allowed; performs no write |
| `useless update --version` with no value or a flag value | Return an explicit parse error |
| Missing release asset for the running target | Fail with the updater's asset error; never install a foreign architecture |
| Digest mismatch | Abort before extraction; installed binary unchanged |
| Update succeeds but service restart fails | Report the restart failure separately; the new binary is already in place |
| Release tag does not match `CARGO_PKG_VERSION` | Release workflow fails before creating the release |
| Existing live PID in `/run/useless.pid` | Daemon start must fail instead of double-running |
| Config reload parse failure | Keep old config active and log failure to stderr |
| Missing systemd and missing init.d | `install` must fail with supported-service-manager error |
| Duplicate active `who` rows for the same `pts/N` | Detector must count one active terminal, not multiple |
| `ps` misses SSH tty correlation on a host | Detector must still fall back to remote `pts/*` counting instead of reporting zero SSH sessions |
| `pressure_window_ms = 0` | Config validation must fail |
| `pressure_enter_ms > pressure_window_ms` | Config validation must fail |
| `reserve_percent >= 100` | Config validation must fail |
| `[memory].chunk_mb = 0` | Config validation must fail |
| `[network].probe_window_ms` outside 1..=2000 | Config validation must fail |

---

### 5. Good / Base / Bad Cases

#### Good

```toml
preset = "balanced"
check_interval_secs = 5
interactive_grace_secs = 30
active_tty_idle_secs = 30

[cpu]
enabled = true
max_percent = 35
burst_secs_min = 15
burst_secs_max = 50
rest_secs_min = 40
rest_secs_max = 180
```

Expected result:

* config parses,
* active SSH/TTY sessions older than 30 seconds idle no longer count as active,
* duplicate observations of the same active `pts/N` count once,
* daemon resumes roughly 30 seconds after the last active interaction sample.

#### Base

```toml
preset = "balanced"
```

Expected result:

* defaults from `Config::for_preset(Preset::Balanced)` fill the rest,
* generated default config remains valid,
* `install` works without further edits.

#### Bad

```toml
active_tty_idle_secs = 0

[network]
enabled = true
targets = []
```

Expected result:

* config validation fails,
* daemon or status command must not continue with invalid settings.

---

### 6. Tests Required

At minimum, keep these checks passing when this contract changes:

* `cargo fmt --all --check`
* `cargo clippy --all-targets --locked -- -D warnings`
* `cargo check`
* `cargo test --all-targets --locked`

Unit-test expectations:

* config parsing tests must assert new fields or validation rules in `src/config.rs`
* sensor tests must assert remote/active TTY classification, tty deduplication, `sshd` tty parsing, and fallback behavior in `src/adapters/procfs.rs`
* policy tests must assert pause behavior for interactive sessions, pressure quiet-window/slow-start behavior, and parity with the pre-refactor decision table in `src/domain/reducer.rs`
* PSI parsing and adapter tests live in `src/adapters/psi.rs`; reactor dispatch/timer tests in `src/runtime/reactor.rs`; journal/state tests in `src/runtime/journal.rs`
* memory budget/ledger tests must assert the reserve/headroom/absolute caps and that growth stops when fresh headroom is insufficient (`src/domain/budget.rs`, `src/workers/memory.rs`)
* unit-generation tests must assert the exact `MemoryHigh`/`MemoryMax` and `OOMScoreAdjust=1000` values (`src/install.rs`); cgroup discovery tests live in `src/adapters/cgroups.rs`
* session tests must cover logind directory scanning, tty dedup, variable-length inotify event parsing, probe filtering/re-emission, atime handling, and watch re-add (`src/adapters/sessions.rs`); Linux-only cases are `cfg(target_os = "linux")`
* network tests must cover the counter reader, estimator subtraction/own-equals-total/wrap, the pure `should_start_burst` gate, and the new config keys (`src/adapters/netstat.rs`, `src/domain/network.rs`, `src/config.rs`)
* update tests must cover CLI parsing for `update`/`version`, tag normalisation (`X.Y.Z` and `vX.Y.Z`), and the configured install path, archive path, current version, and pinned tag (`src/cli.rs`, `src/update.rs`)
* Linux-only tests are `cfg(target_os = "linux")`; run `cross check --all-targets --target x86_64-unknown-linux-gnu` (Docker required, because `ring` needs a Linux C toolchain) when developing on another host

Manual-test expectations for install/runtime changes:

* `useless install`
* `useless reload`
* `useless status`
* `useless version`
* `useless update --check`
* `useless uninstall`
* real SSH session test verifying pause and recovery timing

Assertion points:

* systemd service starts successfully,
* `Nice=19` remains in the unit or runtime path,
* daemon logs interactive detection when SSH sessions are active,
* daemon returns to `pressure=Idle` logs after idle timeout + grace window expires.

---

### 7. Wrong vs Correct

#### Wrong

* Change daemon/session timing behavior in code without documenting the new config keys.
* Add or rename install paths in `src/install.rs` without updating path contracts here.
* Count all historical `pts/*` sessions as active forever.
* Count duplicate rows for the same active `pts/N` as multiple interactive sessions.
* Make `update` replace `current_exe()` instead of `INSTALL_BIN_PATH`.
* Add a background auto-update path without a signing and rollout design.
* Publish a release asset name that the updater cannot match to its compile-time target.

#### Correct

* Add new config fields to `src/config.rs`, default config text, parser, validation, and this spec in one task.
* Treat SSH detection as active-session classification, not raw historical login counting.
* Deduplicate tty identities before deriving user-visible `tty` and `ssh` counters.
* Keep install behavior explicit across binary path, config path, service-manager path, and PID path.
* Keep the release archive contract (`useless-<target>.tar.gz`, binary at root) in sync across `scripts/package.sh`, the release workflow, and `src/update.rs`.
