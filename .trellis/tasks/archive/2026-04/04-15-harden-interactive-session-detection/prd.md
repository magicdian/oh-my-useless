# brainstorm: harden interactive session detection

## Goal

Make `useless` detect real active interactive sessions more accurately so SSH/TTY protection remains conservative for real users but avoids false positives from duplicate utmp entries, nested shells such as `sudo -s`, and stale `pts/*` records.

## What I already know

* The current detector lives entirely in `src/sensor.rs`.
* Current logic reads `who -u --ips`, parses idle time, classifies `pts/*` as potential remote SSH sessions, and uses `active_tty_idle_secs` as the active-session threshold.
* This already fixed the largest issue from earlier versions: historical `pts/*` sessions no longer stay active forever.
* A remaining edge case existed: a single SSH session could temporarily appear as two active sessions after `sudo -s`, because the detector trusted raw `who` lines rather than deduplicating by terminal identity.
* The final implementation now deduplicates active sessions by tty name and lightly correlates remote `pts/*` entries with `sshd: user@pts/N` process data when available.

## Assumptions (temporary)

* We still want a std-only implementation with no new dependencies.
* We should keep `who -u --ips` as one input source because it already exposes idle time and remote host data.
* We do not need perfect PAM/session accounting; we need robust operator-facing behavior.

## Open Questions

* None. Requirements are implemented.

## Requirements (evolving)

* Keep interactive detection inside the current sensor layer.
* Preserve the existing config knobs:
  * `active_tty_idle_secs`
  * `interactive_grace_secs`
* Prevent a single terminal from being counted multiple times.
* Keep `tty` counts meaningful for active terminals.
* Keep `ssh` counts meaningful for active remote terminals.
* Continue to favor false negatives less than false positives for user experience, but reduce obvious duplicate counts.
* Add regression tests for duplicate `pts/*` / nested shell scenarios.

## Acceptance Criteria (evolving)

* [x] A single SSH session followed by `sudo -s` should not reliably inflate the interactive count by +1.
* [x] Multiple active SSH sessions on distinct `pts/*` lines should still count correctly.
* [x] Stale or old `pts/*` sessions should remain excluded once idle time exceeds `active_tty_idle_secs`.
* [x] The detector remains configurable through `active_tty_idle_secs`.
* [x] Unit tests cover duplicate tty and remote session parsing cases.

## Definition of Done

* `cargo fmt --check` passes
* `cargo check` passes
* `cargo test` passes
* Manual SSH verification is described

## Out of Scope

* eBPF-based session tracing
* PAM integration
* Perfect accounting for every distro/session manager combination

## Research Notes

### What similar tools effectively do

Most practical terminal/session detectors treat the terminal identity (`pts/N` or `ttyN`) as the stable unit of interaction rather than trusting raw login rows blindly.

### Constraints from this repo

* The crate is intentionally std-only.
* Install/runtime behavior is already working and should not be disturbed.
* The change should stay localized to sensing and its tests.

### Feasible approaches here

**Approach A: deduplicate by active tty name**

* How it works:
  * Parse `who -u --ips`
  * Filter to active sessions by idle threshold
  * Store unique active `tty` values in a set
  * Derive `tty` and `ssh` counts from the deduplicated set
* Pros:
  * Small, local change
  * Directly targets the `sudo -s` / duplicate utmp symptom
  * Easy to test
* Cons:
  * Still trusts `who` as the primary source
  * Cannot distinguish some unusual duplicated SSH accounting cases

**Approach B: deduplicate by tty name and cross-check with sshd session data** (Chosen)

* How it works:
  * Build the active tty set from `who`
  * Cross-check remote `pts/*` entries against `ps` or `/proc` session data
* Pros:
  * Stronger confidence for SSH counts
  * More resilient to odd `who` formats
* Cons:
  * More parsing complexity
  * More platform variance
  * Higher maintenance cost for moderate gain

## Technical Approach

Implemented path:

1. Introduce an internal representation for active sessions keyed by tty name.
2. Deduplicate active `tty` entries before counting.
3. Derive SSH counts from deduplicated terminal entries and cross-check them with parsed `sshd: user@pts/N` process strings when available.
4. Fall back to remote `pts/*` classification if `ps` data cannot confirm SSH terminals.
5. Add focused tests for:
   * duplicate `pts/*` rows
   * active vs stale remote sessions
   * `sudo -s`-like duplicated tty observations
   * `sshd` tty parsing and fallback behavior

## Technical Notes

Relevant files:

* `src/sensor.rs`
* `src/config.rs`
* `src/policy.rs` only if logging semantics change

## Decision (ADR-lite)

**Context**: `who -u --ips` provides useful active-session and idle information, but raw rows can duplicate the same active terminal in shell nesting scenarios. Pure `who`-based counting caused false-positive session inflation.

**Decision**: Keep `who -u --ips` as the primary activity source, deduplicate by tty identity, and lightly correlate remote `pts/*` terminals with `sshd: user@pts/N` process strings from `ps`. If the process view is unavailable or inconclusive, fall back to remote `pts/*` classification.

**Consequences**:

* Much better handling of `sudo -s` and duplicate utmp rows
* Minimal architectural impact outside the sensor layer
* Slightly more parsing logic and platform variance in the SSH count path
