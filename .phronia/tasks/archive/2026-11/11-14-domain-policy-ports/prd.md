# refactor: extract pure policy domain and ports

## Goal

Create the stable core of the new architecture without changing runtime behavior: a pure domain layer (`Signal`, `PolicyState`, reducer, budget math, burst schedule), port traits, a generation-based store, and an in-memory transition journal. Rewire the current daemon, sensor, and workers through this core.

This child exists so that every later change is testable in isolation and so that policy can never again grow hidden filesystem or clock dependencies.

## Background

* Current policy lives in `src/policy.rs` and mixes pure decisions with `Instant`, RNG seeding, and burst state.
* Current observation lives in `src/sensor.rs` and directly shells out to `who` and `ps`.
* Current sharing uses `Arc<RwLock<RuntimeState>>`; workers poll it every second (`src/workers/*.rs`).
* Current supervisor samples every `check_interval_secs` and stores a `PolicySnapshot` (`src/daemon.rs:62-83`).

These four concerns are entangled today. Later children add event sources and a memory ledger; without this extraction they would each grow new ad-hoc state.

## Requirements

* R1.1 `src/domain/` contains only pure code: `Signal`, `SampleSnapshot`, `SessionSet`, `Mode`, `ReasonCode`, `PolicyState`, `PolicyOutput`, `reduce`, budget math, burst schedule.
* R1.2 `reduce` takes `(state, signal, now)` and returns directives. It performs no I/O, no allocation of OS resources, no logging, and no internal clock reads.
* R1.3 `src/ports.rs` declares the traits used later: `EventSource`, `Sampler`, `Clock`, `StatePublisher`, `JournalSink`. Only `Sampler` and `Clock` have concrete users in this child.
* R1.4 `src/runtime/store.rs` replaces `RuntimeState`'s raw `RwLock` usage with a generation counter and a condvar. `src/runtime/journal.rs` provides an in-memory ring of transitions (file sink arrives in child 2).
* R1.5 `src/adapters/procfs.rs` wraps the existing `sensor.rs` logic as a `Sampler`; the existing parsing code and its six tests move with it, unchanged.
* R1.6 The daemon becomes a composition root: one timer-driven pump emits `Signal::Sample` / `Signal::Sessions` at `check_interval_secs`, calls `reduce`, publishes directives.
* R1.7 Workers consume directives from the store. They may keep a 1 s wait timeout in this child; the condvar wake-up path is wired but behavior must remain identical.
* R1.8 No config key, default, CLI output, or systemd unit changes in this child. `print-default-config` output must be byte-identical.
* R1.9 No new dependencies.

## Acceptance criteria

* [ ] AC1.1 `cargo fmt --check`, `cargo check`, `cargo test` pass; all nine pre-existing tests still pass.
* [ ] AC1.2 A behavior-parity test drives both the pre-refactor policy logic (kept as a test fixture) and `reduce` over a table of samples covering idle, caution, pause-by-load, pause-by-memory, interactive, grace expiry, and burst/rest boundaries; mode and all three directives match.
* [ ] AC1.3 Deterministic unit tests cover budget arithmetic, burst schedule transitions, session grace, and reason ordering.
* [ ] AC1.4 Production code under `src/domain/` contains no `Instant::now`, filesystem, process, or sleep calls. Test modules may call `Instant::now()` to establish a base time; the review distinguishes `#[cfg(test)]` code from production code.
* [ ] AC1.5 `grep -rn "reduce(" src/adapters src/workers src/runtime` returns nothing outside the daemon composition root.

## Out of scope

* epoll, PSI, inotify, `libc`, systemd unit changes, new config keys.
* Behavior changes of any kind. Any improvement discovered during extraction is recorded as a note for a later child, not applied here.

## Open questions

None.
