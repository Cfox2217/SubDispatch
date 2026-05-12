# Rust Release Path

SubDispatch ships as a Rust single-binary local tool. The packaged tool has no
Python or Node runtime dependency; runtime still requires `git`, a configured
external code-agent CLI, and model API credentials.

## Build

```bash
cargo test
cargo build --release
```

The binary is:

```bash
target/release/subdispatch
```

## Package

```bash
scripts/release.sh
```

The script creates:

```bash
dist/subdispatch-<version>-<host-triple>.tar.gz
```

Cross-platform releases should be built on each target host or by CI runners
for:

- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

## Install

Place `subdispatch` on `PATH`, then initialize a workspace:

```bash
subdispatch init-env --workspace .
subdispatch install --project --workspace .
subdispatch doctor --workspace .
```

Global MCP install is available when the user intentionally wants SubDispatch
for all Codex workspaces:

```bash
subdispatch install --global --workspace /absolute/path/to/project
```

## Web UI

```bash
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

The UI provides setup checks, `.env` editing, worker status, and read-only task
activity. It does not create tasks, review diffs, merge branches, or approve
work.

## Python Removal

The Python MVP has been removed. See
[python-removal-plan.md](python-removal-plan.md) for the removal record.
