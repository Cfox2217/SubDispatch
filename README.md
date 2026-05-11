# SubDispatch

SubDispatch is being migrated from the Python MVP to a Rust single-binary local
tool. The Rust binary is now the forward path for CLI, MCP stdio, worker
dispatch, git worktree management, artifact collection, Claude hook recording,
and the local Setup/Activity UI. The Python implementation remains in the repo
as a behavior reference until Rust parity is fully validated.

SubDispatch is a local scaffold for a primary LLM to run child coding agents in
parallel. The primary LLM owns planning, review, merge decisions, and conflict
resolution. SubDispatch only provides isolated execution, status polling,
artifact collection, and worktree cleanup.

Runtime dependencies are intentionally small:

- `git`
- a configured external code-agent CLI, default `claude`
- model API credentials in the workspace `.env`

No Python or Node runtime is required for the Rust binary.

## Non-goals

- Automatic task planning
- Automatic review
- Automatic merge or cherry-pick
- Conflict resolution
- Multi-provider abstraction

## Core model

SubDispatch tracks three entities:

- `Worker`: a configured external coding-agent command. The MVP default is
  `claude-code`.
- `Run`: one group of child tasks launched from a shared base commit.
- `Task`: one child-agent execution in its own branch and git worktree.

Each task records its base commit, branch, worktree path, process id, logs,
result manifest path, and artifact directory.

## Configuration

SubDispatch reads project-local configuration from `.env` in the workspace root.
`.env` is git-ignored. `.env.example` documents the supported keys.

Create the local file with the Rust CLI:

```bash
subdispatch init-env
```

Then edit `.env` directly. The MVP supports the default `claude-code` worker:

- `SUBDISPATCH_WORKER_MODE`
- `SUBDISPATCH_CLAUDE_ENABLED`
- `SUBDISPATCH_CLAUDE_PERMISSION_MODE`
- `SUBDISPATCH_CLAUDE_COMMAND`
- `SUBDISPATCH_CLAUDE_MODEL`
- `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_BASE_URL`

The default worker mode is `trusted-worktree` with Claude Code
`bypassPermissions`. This is intentional for delegated coding loops where the
primary agent transfers execution ownership to the child agent. It is not a
security sandbox. SubDispatch relies on isolated git worktrees, explicit task
scope, logs, and post-run artifact review rather than pre-execution containment.

## Interfaces

### `list_workers`

Returns available workers and current capacity:

- worker id
- runner command
- configured model
- max concurrency
- running count
- queued count
- available slots
- unavailable reason, if any

### `start_run`

Starts a run from a primary-LLM supplied task list. For every task,
SubDispatch creates a branch and worktree, writes a task prompt, and starts the
configured worker when capacity is available. Tasks over the worker concurrency
limit stay queued.

Tasks may include optional `context` or `context_files` supplied by the primary
agent. This is the right way to give a child agent uncommitted diffs, temporary
audit notes, or other context that is not present in the child worktree's base
commit.

### `poll_run`

Returns factual task status for a run. Polling refreshes process state and
starts queued tasks when worker slots open.

Task statuses are:

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `missing`

### `collect_task`

Collects one task artifact. SubDispatch computes changed files and diffs from
Git instead of trusting the worker manifest. It includes uncommitted worktree
changes because child agents are not required to commit.

The returned artifact includes:

- original instruction
- worker manifest, if present
- stdout/stderr tails
- changed files
- diff
- patch path
- base commit
- task branch
- write-scope check
- forbidden-path check

### `delete_worktree`

Deletes one SubDispatch-managed task worktree. It refuses to delete a running
task unless forced. By default it preserves the branch and artifact directory.

## Hard constraints

- Child agents never run in the primary worktree.
- Every task has its own branch.
- Every task has its own worktree.
- Every task records a base commit.
- `collect_task` uses Git as the source of truth.
- Worktree deletion verifies the target is under the SubDispatch worktree root.
- Artifacts are preserved by default.
- Worker concurrency limits are enforced.

## Rust CLI

During local development:

```bash
cargo run -- workers --workspace .
cargo run -- init-integration --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

Packaged usage is the same without `cargo run --`:

```bash
subdispatch workers --workspace .
subdispatch init-integration --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

The Web UI is intentionally not a task console. It provides Setup checks,
`.env` initialization, worker capacity, run/task status, changed-file counts,
and Claude hook activity. The primary LLM still creates tasks through MCP or
CLI.

## Install And Release

Install MCP config for the current project:

```bash
subdispatch install --project --workspace .
subdispatch doctor --workspace .
```

Create a local release archive:

```bash
scripts/release.sh
```

See [docs/rust-release.md](docs/rust-release.md) for packaging details and
[docs/python-removal-plan.md](docs/python-removal-plan.md) for the Python MVP
retirement plan.
