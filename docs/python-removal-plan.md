# Python MVP Removal Plan

Python code should be removed only after the Rust binary has replaced the
runtime, install, setup, and observability paths.

## Keep Until Rust Proves

- `subdispatch/subdispatch.py` behavior is covered by Rust CLI/MCP smoke tests.
- `subdispatch_mcp.py` behavior is covered by Rust MCP stdio tests.
- `.env` worker parsing supports the GLM and MiniMax configurations in use.
- Claude hook events are visible through `poll-run`, `collect-task`, and Web
  Activity.
- `delete-worktree` preserves artifacts and refuses unsafe paths.

## Removal Scope

When ready, remove:

- `cli.py`
- `subdispatch_mcp.py`
- `subdispatch/`
- Python tests that only cover removed code
- Python packaging metadata in `pyproject.toml`

Keep or replace:

- README and docs
- `.env.example`
- Rust integration tests
- release scripts

## Exit Criteria

- `cargo test`
- `scripts/release.sh`
- fake-worker CLI end-to-end smoke
- MCP `initialize`, `tools/list`, and each core tool call smoke
- Web `/api/setup`, `/api/env`, and `/api/snapshot` smoke
- one manual real Claude Code worker run, if credentials and quota are available
