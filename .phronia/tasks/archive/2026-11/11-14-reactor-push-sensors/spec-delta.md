# Spec Delta

## Target files
- `.phronia/spec/backend/daemon-install-contracts.md`
- `.phronia/spec/backend/logging-guidelines.md`
- `.phronia/spec/backend/quality-guidelines.md`

## Intended changes

### `daemon-install-contracts.md`

- Add the new global fields to the supported config list:
  - `pressure_enter_ms` (defaults: 50 low/balanced, 40 aggressive)
  - `pressure_window_ms` (default 1000)
  - `resume_quiet_secs` (defaults: 120 low, 90 balanced, 60 aggressive)
  - `resume_ramp_secs` (defaults: 120 low, 90 balanced/aggressive)
- Add validation rows: `pressure_window_ms = 0` fails; `pressure_enter_ms > pressure_window_ms` fails.
- Under **Runtime behavior**, add:
  - PSI triggers on `/proc/pressure/{cpu,memory,io}` drive suspension; a pressure event suspends immediately and the host stays suspended for `resume_quiet_secs` after the last event, then resumes through a 25/50/100% ramp over `resume_ramp_secs`.
  - When PSI is unavailable or unwritable, the procfs sampler remains the fallback and `status` reports `psi=unavailable`.
  - `useless status` appends a `daemon_state:` block read from `/run/useless/state`, with `mode`, `reasons`, `budget_mb`, `available_mb`, `capabilities`, `last_transition`, and `summary`.
  - State and journal paths: `/run/useless/state` (line-based `key=value`) and `/run/useless/journal.jsonl` (transition-only JSONL).
  - The daemon blocks `SIGHUP`/`SIGINT`/`SIGTERM` and reads them through `signalfd` on Linux; `SIGHUP` rebuilds the reactor with the new config.
- Unit-test expectations: add PSI parsing, pressure quiet-window/slow-start, adaptive cadence, reactor dispatch, journal file bounds, and state snapshot tests. Linux-only tests must be `cfg(target_os = "linux")`.

### `logging-guidelines.md`

- Document the transition-only journal at `/run/useless/journal.jsonl` and the line-based state snapshot at `/run/useless/state`.
- Journal write or trim failures disable the file sink for the process lifetime and log once; they never crash the daemon.
- Keep the existing "log on summary change" rule; the daemon also logs one `[useless] sensors: ...` line at startup and after a reload.

### `quality-guidelines.md`

- Add the Linux cross-check to the required checks:
  `cargo check --tests --target x86_64-unknown-linux-gnu`.
- Record that `libc` is the only allowed new dependency and exists for ABI-correct epoll/timerfd/signalfd bindings.

## Rationale

Child 2 moves pressure detection from 1-minute loadavg to PSI triggers and adds a state/journal surface for operators. The specs describe the pre-reactor behavior and would mislead child 3/4 and future maintenance. The state file is line-based rather than JSON so `status` stays dependency-free.

## Verification

- After applying: `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo check --tests --target x86_64-unknown-linux-gnu` pass.
- `useless status` prints the `daemon_state` block from a fixture state file.
- `grep -n "state.json" .phronia/spec/` returns nothing after the delta.
