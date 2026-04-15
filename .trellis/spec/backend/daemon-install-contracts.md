# Daemon Install Contracts

> Executable contracts for the `useless` CLI, config file, service-manager integration, and daemon runtime.

---

## Scenario: Rust Daemon MVP

### 1. Scope / Trigger

Use this spec when a task changes any of the following:

* CLI subcommands in `src/cli.rs` or `src/main.rs`
* Config fields or defaults in `src/config.rs`
* systemd or init.d integration in `src/install.rs`
* Daemon lifecycle, PID, signal, or reload behavior in `src/daemon.rs`
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
* `[memory].burst_secs_min`
* `[memory].burst_secs_max`
* `[memory].rest_secs_min`
* `[memory].rest_secs_max`

Network section:

* `[network].enabled`
* `[network].max_mbps`
* `[network].connect_timeout_secs`
* `[network].burst_secs_min`
* `[network].burst_secs_max`
* `[network].rest_secs_min`
* `[network].rest_secs_max`
* `[network].targets`

#### Runtime behavior

* The daemon samples host state every `check_interval_secs`.
* Interactive-session detection uses active TTY/SSH sessions from `who -u --ips`.
* A session is considered active only if its idle time is `<= active_tty_idle_secs`.
* Once interaction is detected, the daemon remains in pause mode for `interactive_grace_secs` after the last active sample.
* Worker scheduling uses randomized burst/rest windows from the relevant module section.

#### Install behavior

* `install` must require root.
* `install` must ensure a config file exists before enabling the service.
* systemd is preferred when `/run/systemd/system` exists and `systemctl` is available.
* init.d is the fallback when `/etc/init.d` exists.
* `uninstall --purge` may remove `/etc/useless`; plain `uninstall` must leave config in place.

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
| Existing live PID in `/run/useless.pid` | Daemon start must fail instead of double-running |
| Config reload parse failure | Keep old config active and log failure to stderr |
| Missing systemd and missing init.d | `install` must fail with supported-service-manager error |

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

* `cargo fmt --check`
* `cargo check`
* `cargo test`

Unit-test expectations:

* config parsing tests must assert new fields or validation rules in `src/config.rs`
* sensor tests must assert remote/active TTY classification in `src/sensor.rs`
* policy tests must assert pause behavior for interactive sessions in `src/policy.rs`

Manual-test expectations for install/runtime changes:

* `useless install`
* `useless reload`
* `useless status`
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

#### Correct

* Add new config fields to `src/config.rs`, default config text, parser, validation, and this spec in one task.
* Treat SSH detection as active-session classification, not raw historical login counting.
* Keep install behavior explicit across binary path, config path, service-manager path, and PID path.
