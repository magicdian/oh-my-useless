# Implementation plan: reactor and push-based host pressure

## Checklist

1. [ ] Add `libc` to `Cargo.toml`; record the ADR reference in the commit message.
2. [ ] Implement `src/sys.rs` with epoll, timerfd, signalfd, `pread`, non-blocking setup, and `AppError` conversion.
3. [ ] Implement `src/runtime/reactor.rs` with the `EventSource` trait, token map, deadline timer, and bounded drain.
4. [ ] Implement `SignalfdSource` and `TimerSource`; block process signals before spawning workers.
5. [ ] Implement `src/adapters/psi.rs` with `PsiOps`, trigger registration, `EBUSY` reopen, `POLLERR` rebuild, and capability probe.
6. [ ] Implement `MemInfoSampler`/`LoadSampler` and the adaptive cadence; wire `SamplerSource`.
7. [ ] Extend the domain with `PressureReading`, `PressureClass`, `PressureScope`, `SourceHealth`, and the pressure/quiet-window rules; add reducer tests.
8. [ ] Implement the journal file sink and atomic state snapshot; extend `status` to read the state file.
9. [ ] Replace the daemon sleep loop with the reactor; delete raw signal handlers and `sleep_with_interrupts`.
10. [ ] Add reactor/PSI/journal/cadence tests.
11. [ ] Update `.phronia/spec/backend/logging-guidelines.md` (journal file, transition-only logging) and `daemon-install-contracts.md` (status lines, state file path, capability reporting).

## Validation

```bash
cargo fmt --check
cargo check
cargo test

# steady state must block, not spin
sudo strace -f -e trace=epoll_wait,poll -p "$(cat /run/useless.pid)" -c -w

# capability + mode reporting
useless status
cat /run/useless/state.json
tail -n 5 /run/useless/journal.jsonl
```

Pressure smoke test on the VM:

```bash
# allocate until pressure appears, then release
python3 - <<'PY'
import time
blocks=[]
try:
    while True:
        blocks.append(bytearray(8*1024*1024))
        time.sleep(0.01)
        if len(blocks) > 120: break
finally:
    blocks.clear()
PY
```

Expected: `journal.jsonl` shows a `psi.memory` transition to `suspended` within 100 ms of the pressure onset.

## Risky files

* `src/daemon.rs` — lifecycle and shutdown; verify worker join on SIGTERM.
* `src/sys.rs` — all unsafe code is concentrated here; review every constant and error path.
* `Cargo.toml` — the first dependency in the project; keep the justification explicit.

## Rollback point

One commit. Revert returns to the child 1 sampler loop. `libc` stays because later children require it; only remove it when reverting the whole parent.
