# Implementation plan: reactor and push-based host pressure

## Checklist

1. [ ] Add `libc` to `Cargo.toml`; record the ADR reference in the commit message.
2. [ ] Implement `src/sys.rs` with epoll, timerfd, signalfd, `pread`, non-blocking setup, and `AppError` conversion.
3. [ ] Implement `src/runtime/reactor.rs` with the epoll set, token map, and timerfd deadline.
4. [ ] Implement `SignalSource` with signalfd; block process signals before spawning workers.
5. [ ] Implement `src/adapters/psi.rs` with `PsiOps`, trigger registration, `EBUSY` reopen, `POLLERR` rebuild, and capability probe.
6. [ ] Wire the procfs sampler in the daemon with `sampler_cadence`; no separate sampler source is needed.
7. [ ] Extend the domain with `PressureReading`, `PressureClass`, `PressureScope`, the quiet-window rule, and slow start; add reducer tests.
8. [ ] Implement the journal file sink and the line-based state snapshot; extend `status` to read the state file.
9. [ ] Replace the daemon sleep loop with the reactor; keep raw signal handlers only for non-Linux.
10. [ ] Add reactor/PSI/journal/cadence tests; gate Linux-only tests with `cfg(target_os = "linux")`.
11. [ ] Record the config, status, and state-path changes in `spec-delta.md`; do not edit `.phronia/spec/**`.

## Validation

```bash
cargo fmt --check
cargo check
cargo test
# Type-check the Linux-only reactor/PSI path from a macOS development host.
cargo check --tests --target x86_64-unknown-linux-gnu

# steady state must block, not spin
sudo strace -f -e trace=epoll_wait,poll -p "$(cat /run/useless.pid)" -c -w

# capability + mode reporting
useless status
cat /run/useless/state
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

Expected: `journal.jsonl` shows a `psi:memory` transition to `suspended` within 100 ms of the pressure onset.

## Risky files

* `src/daemon.rs` — lifecycle and shutdown; verify worker join on SIGTERM.
* `src/sys.rs` — all unsafe code is concentrated here; review every constant and error path.
* `Cargo.toml` — the first dependency in the project; keep the justification explicit.

## Rollback point

One commit. Revert returns to the child 1 sampler loop. `libc` stays because later children require it; only remove it when reverting the whole parent.
