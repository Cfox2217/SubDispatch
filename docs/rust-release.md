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

Place `subdispatch` on `PATH`, install the global MCP entry and bundled skill
once:

```bash
subdispatch install --global
subdispatch install-skill
```

Then initialize each project separately:

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

Project-local MCP install is still available when a project should pin a
specific binary or workspace:

```bash
subdispatch install --project --workspace .
```

The release archive includes the bundled Codex skill under
`skills/subdispatch-delegation/SKILL.md`. `subdispatch install-skill` copies it
to `~/.codex/skills/subdispatch-delegation/SKILL.md`.

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
