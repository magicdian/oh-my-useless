# oh-my-useless

`useless` is a Rust-based Linux daemon that coordinates intermittent CPU, memory, and network occupation from one process.

## Current direction

- One binary: `useless`
- Config directory: `/etc/useless`
- Service install: systemd first, init.d fallback
- User activity protection: pause or cool down when SSH / TTY activity or host pressure is detected
- Scheduler style: randomized intermittent bursts, not constant flat load

## Development

```bash
cargo fmt
cargo check
cargo test
```

## Packaging

Build a release tarball with:

```bash
./scripts/package.sh 0.1.0
```

The archive layout is:

```text
useless-0.1.0/
├── bin/
│   └── useless
└── README.md
```

After extraction, install with:

```bash
./bin/useless install
```

## Interaction Tuning

Two config keys control when `useless` resumes work after human activity:

- `active_tty_idle_secs`: how long an SSH or TTY session may stay idle before it is no longer treated as active
- `interactive_grace_secs`: extra cooldown time after the last active interaction is seen

Example:

```toml
interactive_grace_secs = 30
active_tty_idle_secs = 30
```
