# Rust Release Path

SubDispatch's Rust binary is the forward runtime. The packaged tool has no
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

The UI provides setup checks, `.env` editing, worker status, and read-only run
activity. It does not create tasks, review diffs, merge branches, or approve
work.

## Migration Notes

Python remains only as a behavior reference during the migration. Remove it
after these Rust paths are stable:

- CLI command parity for the five core task interfaces
- MCP project/global install and doctor
- fake-worker integration tests for worktree, artifact, and cleanup flows
- Web Setup/Activity smoke tests
- manual real-worker smoke test against Claude Code
