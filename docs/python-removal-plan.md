# Python MVP Removal Record

The Python MVP has been removed. SubDispatch now has one runtime path: the Rust
single binary.

## Removed

- Python CLI entry (`cli.py`)
- Python MCP entry (`subdispatch_mcp.py`)
- Python package directory (`subdispatch/`)
- Python tests (`tests/`)
- Python packaging metadata (`pyproject.toml`)

## Replacement

- CLI: `subdispatch ...`
- MCP stdio server: `subdispatch mcp --workspace .`
- Web UI: `subdispatch serve --workspace .`
- Release package: `scripts/release.sh`

## Verification

Use the Rust checks:

```bash
cargo fmt --check
cargo test
cargo build
git diff --check
```

For MCP contract smoke:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | target/debug/subdispatch mcp --workspace .
```
