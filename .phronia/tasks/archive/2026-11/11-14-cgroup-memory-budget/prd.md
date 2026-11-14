# feat: cgroup-aware PSI and headroom memory ledger

## Goal

Make memory actuation structurally incapable of causing an OOM, and make the daemon see real work inside business cgroups. The memory budget is derived from live headroom, applied in chunks with a just-in-time check, and released immediately on suspension. The installed unit turns the daemon into the first OOM casualty instead of the business pods.

This child contains the fix for the 2026-11-13 incident.

## Background

* `src/workers/memory.rs:20-30` sizes the buffer from `MemTotal`, never from `MemAvailable`.
* `src/workers/memory.rs:48-50` commits the entire buffer in one `Vec::resize`.
* `src/install.rs:162-181` has no `OOMScoreAdjust` and no cgroup memory bounds.
* Child 2 provides PSI watches and the reactor; child 1 provides the pure reducer and store.

## Requirements

* R3.1 Discover cgroup v2 slices from `/sys/fs/cgroup`, intersect with `[sensor].watch_slices` (default `["kubepods.slice", "user.slice"]`), and watch `<slice>/{cpu,memory,io}.pressure` through the existing PSI adapter. Resolve the daemon's own cgroup from `/proc/self/cgroup` and exclude it and its descendants. Missing slices are a health note.
* R3.2 Refresh the slice set every 60 s and on `SIGHUP`; add and remove watches without restarting the reactor.
* R3.3 `domain::budget` implements `reserve = max(reserve_mb, MemTotal × reserve_percent / 100)`, `ratio_cap`, `headroom_cap = max(0, MemAvailable − reserve)`, `absolute_cap`, and `budget = min(ratio_cap, headroom_cap, absolute_cap?)`.
* R3.4 The memory worker becomes a chunked `mmap` ledger: grow one `chunk_mb` chunk at a time only when a fresh check proves `MemAvailable − reserve ≥ current + chunk` and pressure is below the enter threshold; shrink with `madvise(MADV_DONTNEED)` + `munmap`; release everything immediately on `Suspended`.
* R3.5 New config keys and validation: `reserve_mb`, `reserve_percent`, `[memory].max_absolute_mb`, `[memory].chunk_mb`, `[sensor].watch_slices`. Old configs must parse unchanged; `print-default-config` includes the new keys.
* R3.6 `install.rs` derives the unit's `MemoryHigh`/`MemoryMax` from the effective memory ceiling and `MemTotal`, and adds `OOMScoreAdjust=1000`, `IOSchedulingClass=idle`, `CPUWeight=10`, `Restart=on-failure`, and start limits while keeping `Nice=19`.
* R3.7 `status` reports the current budget, headroom, reserve, and the primary suspension scope.
* R3.8 A per-cgroup pressure event suspends synthetic work and reports the slice in the state file's `scope` line and in the summary, so an operator can tell business pressure from host pressure.
* R3.9 Hosts without cgroup v2 or without configured slices keep working with host PSI plus headroom.

## Acceptance criteria

* [ ] AC3.1 `cargo fmt --check`, `cargo check`, `cargo test` pass.
* [ ] AC3.2 Budget table tests cover: ratio below headroom, headroom below ratio, absolute cap below both, negative headroom, zero reserve, and `reserve_percent` dominating `reserve_mb`.
* [ ] AC3.3 A ledger test with a scripted headroom sequence proves that after every growth step `current + chunk ≤ available − reserve`; a step that would violate it leaves `current` unchanged.
* [ ] AC3.4 `Suspended` releases all chunks within one worker wake-up; a test asserts `current == 0` immediately after applying a zero directive.
* [ ] AC3.5 On a 2 vCPU / 2 GiB VM with a 500 MB hoarder and a Flux-like reconcile loop, a 4-hour soak produces no OOM kill whose log line names `useless` or `useless-memory`.
* [ ] AC3.6 `systemctl show useless -p OOMScoreAdjust -p MemoryHigh -p MemoryMax -p IOSchedulingClass` shows the derived values; a unit-generation test asserts the exact text for a known config and `MemTotal`.
* [ ] AC3.7 `grep -rn "Vec::resize\|resize(" src/workers/` shows no large-buffer resize; the ledger is the only allocator.
* [ ] AC3.8 A missing `kubepods.slice` produces `status` output with `cgroups: degraded (missing: kubepods.slice)` and no repeated log spam.
* [ ] AC3.9 Backward compatibility: the pre-change config file parses and produces the old behavior except for the new safety cap.

## Out of scope

* eBPF, per-process accounting, swap tuning, systemd unit changes beyond the listed keys.
* Automatic unit regeneration on `reload` (documented: rerun `install`).
* Changing the default presets' aggressiveness.

## Open questions

None.
