# oh-my-useless

`useless` is a small Rust daemon that consumes idle CPU, memory, and network capacity in randomized bursts. It is designed to yield to real work: human sessions, host pressure, business cgroup pressure, and external network traffic.

It is **not** a benchmark or a load tester. The safety model is that the daemon is the first thing the host sacrifices, never the business.

## Install

```bash
curl -fsSL https://github.com/magicdian/oh-my-useless/releases/latest/download/install.sh | sudo sh
```

The installer detects `x86_64` or `aarch64`, downloads the static musl archive, verifies its SHA-256, and runs `useless install` (systemd or init.d). It requires a Linux host and root.

Pin a release with `USELESS_VERSION`; a leading `v` is accepted:

```bash
curl -fsSL https://github.com/magicdian/oh-my-useless/releases/latest/download/install.sh \
  | sudo env USELESS_VERSION=2611.14.1 sh
```

`install.sh` is published with every release. For the archive layout and manual checksum verification, see [Releases and CI](#releases-and-ci).

## Safety model

- **Event-first sensing.** PSI triggers, inotify session changes, signals, and deadlines arrive through one epoll reactor. Polling remains only for values the kernel cannot notify on, with a cadence tied to the policy mode.
- **Headroom-derived memory.** The budget is the smallest of the configured ratio, `MemAvailable − reserve`, and an optional absolute cap. Allocation is chunked `mmap`; each growth step re-reads fresh `MemAvailable`; suspension releases everything immediately.
- **Business-aware backoff.** Pressure inside `kubepods.slice` / `user.slice` suspends work even when no one is logged in.
- **Self-containment.** The systemd unit sets `OOMScoreAdjust=1000`, `MemoryHigh`/`MemoryMax`, and `IOSchedulingClass=idle`, so a runaway allocator is contained inside the daemon's own cgroup.
- **External-traffic gate.** Before a network burst, the daemon measures interface counters for a short window, subtracts its own bytes, and skips the burst when real traffic is present.

## Architecture

```text
kernel fds ──► runtime::Reactor ──► Signal ──► domain::reduce ──► Store ──► workers
(PSI, inotify,      (epoll+timerfd)            (pure reducer)    (generation    (CPU,
 signalfd)                                                        + condvar)     memory,
                                                                                 network)
                                                       │
                                                       ├──► /run/useless/journal.jsonl
                                                       └──► /run/useless/state
```

| Layer | Responsibility |
|---|---|
| `src/domain/` | Signals, policy state, pure reducer, budget math, burst schedule, network gate. No IO or clock reads. |
| `src/ports.rs` | `Sampler`, `EventSource`, `Clock`, `StatePublisher`, `JournalSink`. |
| `src/adapters/` | `procfs` host metrics, `psi` host/cgroup pressure, `cgroups` slice discovery, `sessions` inotify + tty probe, `signals`, `netstat`. Emit signals only. |
| `src/runtime/` | Reactor, generation-based store, bounded journal and state snapshot. |
| `src/workers/` | CPU burner, chunked memory ledger, network download with the JIT gate. Apply directives only. |

Only `daemon.rs` calls the reducer.

## Configuration

Default path: `/etc/useless/config.toml`. `useless print-default-config` emits the template.

### Global

| Key | Default | Meaning |
|---|---|---|
| `preset` | `balanced` | `low`, `balanced`, or `aggressive` defaults. |
| `check_interval_secs` | `5` | Base sampler cadence; slows to 2×/4× in caution/suspended modes. |
| `interactive_grace_secs` | `180` | Quiet time after logout before resuming. |
| `active_tty_idle_secs` | `300` | TTY idle time still counted as active. |
| `caution_duty_percent` | `45` | Duty in caution mode. |
| `caution_load_percent` | `55` | Load percentage that enters caution. |
| `pause_load_percent` | `82` | Load percentage that suspends. |
| `caution_mem_available_percent` | `20` | MemAvailable percentage that enters caution. |
| `pause_mem_available_percent` | `10` | MemAvailable percentage that suspends. |
| `pressure_enter_ms` | `50` | PSI stall milliseconds per window that triggers suspension. |
| `pressure_window_ms` | `1000` | PSI tracking window. |
| `resume_quiet_secs` | `90` | Clean time after the last pressure event before resuming. |
| `resume_ramp_secs` | `90` | 25/50/100% slow-start ramp after resume. |
| `reserve_mb` | `512` | Absolute memory reserve floor. |
| `reserve_percent` | `10` | Reserve as a share of MemTotal; `reserve` is the larger value. |

### `[memory]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Enable synthetic memory occupation. |
| `max_percent` | `15` | Ratio cap as a share of MemTotal, further capped by headroom. |
| `max_absolute_mb` | `0` | Absolute cap; `0` disables it. |
| `chunk_mb` | `16` | Growth/shrink granularity. |
| `burst_secs_min` / `burst_secs_max` | `40` / `120` | Burst window. |
| `rest_secs_min` / `rest_secs_max` | `90` / `360` | Rest window. |

The effective budget is `min(MemTotal × max_percent × duty, MemAvailable − reserve, max_absolute_mb?)`.

### `[network]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Enable synthetic network occupation. |
| `max_mbps` | `10` | Hard bandwidth cap. |
| `external_traffic_kbps` | `1024` | Skip a burst when external traffic exceeds this; `0` disables the gate. |
| `probe_window_ms` | `1000` | Two-sample window for the external-traffic estimate (1–2000). |
| `connect_timeout_secs` | `3` | TCP connect timeout. |
| `burst_secs_min` / `burst_secs_max` | `20` / `60` | Burst window. |
| `rest_secs_min` / `rest_secs_max` | `90` / `360` | Rest window. |
| `targets` | public mirrors | Plain `http://` download targets. |

### `[sensor]`

| Key | Default | Meaning |
|---|---|---|
| `watch_slices` | `["kubepods.slice", "user.slice"]` | cgroup v2 slices whose pressure suspends work. The daemon excludes its own cgroup and ancestors. |

`[cpu]` keeps the original `enabled`, `max_percent`, and burst/rest keys.

## Observability

`useless status` prints the config, service state, and a `daemon_state` block read from `/run/useless/state`:

```text
mode=suspended
reasons=memory_pressure
scope=kubepods.slice
budget_mb=0
reserve_mb=512
available_mb=612
capabilities=psi=ok trigger_write=ok cgroups=2 cgroup_v2=ok inotify=ok sessions=inotify
last_transition=running -> suspended (psi:memory)
summary=pressure=Pause load=12 mem_avail=31 ... scope=kubepods.slice
```

`/run/useless/journal.jsonl` records transitions only:

```json
{"ts_ms":1763128800123,"signal":"psi:memory","from":"running","to":"suspended","reasons":["memory_pressure"],"budget_mb":0,"available_mb":612}
```

Both files live on tmpfs and are bounded. journald still captures stderr.

## Degradation

| Condition | Behavior |
|---|---|
| PSI missing or trigger write fails | Sampler-only mode; `status` shows `psi=unavailable`. |
| No cgroup v2 | Host PSI and `MemAvailable` only. |
| inotify unavailable | `who -u --ips` + `ps` sampler; `sessions=who`. |
| A configured slice is missing | `cgroups=degraded(missing: ...)`; the host PSI path still works. |
| Network counters unreadable | Gate open; `max_mbps` still applies; one log line. |
| Journal unwritable | In-memory ring only; one log line; no crash. |

## Operations

```bash
useless install      # install the binary and service, regenerate the unit
useless status       # config, service state, daemon_state
useless reload       # reload config; does NOT rewrite the systemd unit
useless version      # print the installed version
useless update       # install the newest GitHub release and restart the service
useless uninstall    # remove the service/binary; keep /etc/useless
useless uninstall --purge   # also remove /etc/useless
```

`install` derives the unit's `MemoryHigh`/`MemoryMax` from the current config and `MemTotal`; rerun `install` after changing memory limits. The unit uses `Restart=on-failure` with start limits so an OOM-killed daemon does not loop forever.

### Interaction tuning

- `active_tty_idle_secs`: how long a TTY may stay idle before it no longer counts as active.
- `interactive_grace_secs`: quiet time after logout before resuming.

The normal session path is inotify-based on systemd hosts; `who`/`ps` is the fallback.

## Updating

```bash
useless version                       # print the installed version
useless update --check                # query GitHub Releases; no write, no root
sudo useless update                   # install the newest release and restart the service
sudo useless update --version 2611.14.1   # install or roll back to a specific release
sudo useless update --no-restart      # replace the binary, leave the service alone
```

`update` writes `/usr/local/bin/useless` (not the executable you invoked it from) and verifies the SHA-256 digest GitHub publishes for the release asset before extracting. It restarts the service only when systemd or init.d reports it active. A failed restart is reported separately: the new binary is already in place, so re-run `systemctl restart useless`.

The updater matches the release asset against the compile-time target of the running binary. Use the archive for your architecture; if a release does not contain that target, `update` fails instead of installing a foreign architecture. `--version` accepts both `2611.14.1` and `v2611.14.1`; both target the bare OMV tag.

Set `USELESS_GITHUB_TOKEN` to raise the GitHub API rate limit. There is no background updater; updates are always explicit.

## Versioning

Versions are owned by OMV. `.omv/state.toml` is the single truth; `Cargo.toml`, `Cargo.lock`, and `src/generated/version.rs` are synchronized outputs. The project runs in `release_mode = "pipeline"`, so only the release job writes versions.

Local development is read-only:

```bash
npm install -g @magicdian/omv@2611.6.1
omv current        # version, logical date, release mode
omv plan           # deterministic target drift and proposed writes
omv sync --check   # exit non-zero on drift; never mutates
```

Never edit the version in `Cargo.toml` by hand and never run `omv bump` or `omv sync` locally: pipeline mode rejects both without `--as-release`. A pull request that changes `.omv/state.toml` or `.omv/finalizations.toml`, or whose manifest drifts from state, fails CI.

The managed version is a date triplet, `YYMM.dd.BuildNumber` (for example `2611.14.1`), which is a valid semver value for the updater. The bootstrap state is `2611.14.0`; the first pipeline release is `2611.14.1`.

## Releases and CI

`.github/workflows/ci.yml` runs `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --all-targets --locked` on Linux, a macOS compile/test job, and a version job that installs a pinned OMV, prints `omv current`, runs `omv sync --check`, and on pull requests rejects changes to `.omv/state.toml` / `.omv/finalizations.toml` and requires `release_mode = "pipeline"`.

`.github/workflows/release.yml` runs on a push to `main` or a manual `workflow_dispatch`:

1. Runs the same tests.
2. Reads `omv current`, runs `omv bump --expect <current> --now <UTC date> --as-release`, verifies with `omv sync --check`, refreshes `Cargo.lock` with `cargo metadata`, and refuses any changed path outside `.omv/`, `Cargo.toml`, `Cargo.lock`, or `src/generated/version.rs`.
3. Commits `chore(release): <version>`, pushes to `main`, and creates the annotated tag `<version>`.
4. Creates the GitHub Release and uploads four archives.

Do not create tags manually: the build consumes the tag created by the pipeline in the same run, so a stray tag publishes nothing. `GITHUB_TOKEN` must be allowed to push to `main`; on a protected branch, substitute a GitHub App or fine-grained PAT with bypass permission as the release token.

| Target | Notes |
|---|---|
| `x86_64-unknown-linux-gnu` | Needs a reasonably recent glibc (built on `ubuntu-latest`). |
| `x86_64-unknown-linux-musl` | Static; recommended for older or mixed hosts. |
| `aarch64-unknown-linux-gnu` | ARM64 servers and Raspberry Pi OS. |
| `aarch64-unknown-linux-musl` | Static ARM64. |

Each release contains `useless-<target>.tar.gz`, `useless-<target>.sha256`, and `install.sh`. The archive has `useless`, `README.md`, and `LICENSE` at its root; extract it and run `sudo ./useless install`.

To release, merge to `main` or run the Release workflow manually from `main`; the version is assigned by the pipeline. Manual verification:

```bash
base="https://github.com/magicdian/oh-my-useless/releases/latest/download"
curl -fsSLO "$base/useless-x86_64-unknown-linux-musl.tar.gz"
curl -fsSLO "$base/useless-x86_64-unknown-linux-musl.sha256"
sha256sum -c useless-x86_64-unknown-linux-musl.sha256
```

Local packaging uses the same layout and naming:

```bash
./scripts/package.sh
# or cross-build for a release target
TARGET=aarch64-unknown-linux-musl ./scripts/package.sh
```

This produces `dist/useless-<host-target>.tar.gz` and `dist/useless-<host-target>.sha256`. The local script does not publish anything; pushes to `main` drive the release workflow.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
# Linux-only reactor/PSI/session code, when developing on another host
# (needs Docker; `cross` provides the Linux C toolchain that ring requires):
cross check --all-targets --target x86_64-unknown-linux-gnu
```

The event-driven path is Linux-only. On non-Linux hosts the daemon compiles and the sampler/legacy session path runs, but it is not a supported production target.

## Packaging

```bash
./scripts/package.sh
```

The archive contains `useless`, `README.md`, and `LICENSE` at its root; after extraction run `sudo ./useless install`. See [Releases and CI](#releases-and-ci) for the published target matrix.
