# Implementation plan: event-driven sensor architecture

## Execution order

The five children are serialized because `daemon.rs`, `config.rs`, and the contract spec are shared. Each child must leave the tree green before the next starts.

| Step | Task | Exit condition |
|---|---|---|
| 1 | `11-14-domain-policy-ports` | Pure reducer proves byte-for-byte policy parity; no dependency changes. |
| 2 | `11-14-reactor-push-sensors` | Daemon runs on the reactor; PSI events drive suspension; samplers remain as fallback. |
| 3 | `11-14-cgroup-memory-budget` | Headroom invariant holds; unit contains self-containment; 4-hour VM soak has no useless-attributable OOM. |
| 4 | `11-14-event-driven-sessions` | Login/logout is event-driven; idle probe is bounded; `who` fallback verified. |
| 5 | `11-14-network-jit-and-docs` | External-traffic gate demonstrably skips bursts; docs and default config are current. |

## Parent-level checklist

- [ ] Confirm each child's `prd.md`, `design.md`, and `implement.md` exist before starting it.
- [ ] After each child: `cargo fmt --check`, `cargo check`, `cargo test`.
- [ ] After each child: review that no adapter calls `reduce`, no worker reads config directly, and no domain type touches the filesystem or `Instant::now()`.
- [ ] After child 2: confirm `strace -f -e epoll_wait,poll` shows blocking waits rather than a spin loop, and that a synthetic pressure event suspends in under 100 ms.
- [ ] After child 3: confirm `systemctl show useless -p OOMScoreAdjust -p MemoryHigh -p MemoryMax -p IOSchedulingClass`.
- [ ] After child 4: confirm login pauses and logout resumes with the configured grace and quiet window.
- [ ] After child 5: confirm README, `print-default-config`, and `daemon-install-contracts.md` agree with the code.

## Integration validation

### Automated

```bash
cargo fmt --check
cargo check
cargo test
python3 ./.phronia/scripts/task.py validate <task-dir>
```

### Manual on a disposable VM (2 vCPU / 2 GiB, no swap, K3s-like memory baseline)

1. Build and install: `./scripts/package.sh dev && ./bin/useless install`.
2. Verify the unit settings with `systemctl show useless`.
3. Start a memory hoarder that holds ~500 MB, then confirm `useless` stays `Suspended` or caps its budget to the remaining headroom.
4. Run a CPU hog pinned to one core; confirm `useless` yields CPU pressure within one event.
5. Log in over SSH and hold the session; confirm suspension. Log out; confirm it waits for grace plus the quiet window.
6. Saturate the uplink with an external download; confirm network bursts are skipped and resume afterwards.
7. Soak for 4 hours with the hoarder and a Flux-like controller loop; check `dmesg -T | grep -E 'invoked oom-killer|Out of memory'` is empty for `useless`.
8. Inspect `/run/useless/journal.jsonl` and `useless status` for coherent reasons and budgets.
9. `useless reload` after a config edit; confirm new thresholds take effect without restarting.
10. `useless uninstall`; confirm the unit, binary, and pid file are removed and the config is preserved.

## Risk register

| Risk | Mitigation | Rollback |
|---|---|---|
| PSI threshold too tight on a noisy host, causing permanent suspension | Conservative default (50 ms per 1 s), journal-driven tuning, `status` explains the reason | Raise `pressure_enter_ms` or disable PSI via config |
| cgroup layout differs across distros | Discovery by path, missing slices are health notes, host PSI always present | Empty `watch_slices` |
| inotify on utmp is unreliable | logind session directory first, utmp second, `who` sampler last | Disable the inotify source |
| `libc` ABI mistake | Use `libc` types and constants directly; no hand-copied structs | Revert child 2 |
| Memory ledger bug | Pure budget math tests plus a scripted-headroom ledger test; the invariant is checked before every chunk | Revert child 3 and set `memory.enabled=false` |
| Journal writes on a tiny tmpfs | Hard cap by halving on overflow; failures never fatal | Disable the file sink, keep the in-memory ring |
| Reactor blocks on a misbehaving fd | All fds non-blocking; each drain bounded; `POLLERR` handled | Rebuild the fd set; degrade to sampler mode |

## Definition of done (parent)

- [ ] All five children are completed and archived.
- [ ] Cross-child acceptance criteria AC1-AC10 are verified with evidence in the child check records.
- [ ] `.phronia/spec/backend/daemon-install-contracts.md`, `directory-structure.md`, and `logging-guidelines.md` reflect the new architecture.
- [ ] README documents the new config keys, `status` output, journal location, and capabilities.
- [ ] No `Vec::resize` of a large synthetic buffer remains; `grep -rn "resize(" src/` is reviewed.
- [ ] No `thread::sleep` remains in the supervisor control path; `grep -rn "sleep(" src/` is reviewed.
- [ ] `git log` shows one commit per child with a message naming the layer it changes.
