# SubDispatch

SubDispatch is a local scaffold for a primary LLM to run child coding agents in
parallel. The primary LLM owns planning, review, merge decisions, and conflict
resolution. SubDispatch only provides isolated execution, status polling,
artifact collection, and worktree cleanup. It ships as a Rust single-binary
local tool for CLI, MCP stdio, worker dispatch, git worktree management, Claude
hook recording, and the local Setup/Activity UI.

Runtime dependencies are intentionally small:

- `git`
- a configured external code-agent CLI, default `claude`
- model API credentials in the workspace `.env`

No Python or Node runtime is required.

## Non-goals

- Automatic task planning
- Automatic review
- Automatic merge or cherry-pick
- Conflict resolution
- Multi-provider abstraction

## Core model

SubDispatch tracks two entities:

- `Worker`: a configured external coding-agent command. The default is
  `claude-code`.
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

Then edit `.env` directly. SubDispatch supports the default `claude-code` worker:

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
scope, logs, and post-task artifact review rather than pre-execution containment.

Prompt configuration is stored separately in `.subdispatch/prompts.json`.
The file is optional; built-in defaults are used when it does not exist. The Web
UI Prompts page can edit:

- MCP tool descriptions
- child-agent prompt template, safety rules, and manifest schema
- worker selection and collect/review guidance

Worker metadata is configured only in Setup/.env. This keeps `description`,
`strengths`, `cost`, `speed`, and `delegation_trust` as one source of truth.
`delegation_trust` is a routing hint for the primary agent, not a safety
guarantee.

Prompt changes apply to new MCP tool listings and newly started child tasks.
Existing tasks are not rewritten.

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
- delegation trust
- unavailable reason, if any

MCP exposes this interface as `list_workers`; the CLI command is
`subdispatch workers --workspace <path>`.

### `start_task`

Starts one primary-LLM supplied child task. SubDispatch creates a branch and
worktree, writes a task prompt, and starts the configured worker when capacity
is available. A task over the worker concurrency limit stays queued.

Delegation requires a clean committed checkpoint. The primary agent owns its own
branch/worktree strategy and must commit in-progress changes before calling
`start_task`. SubDispatch does not manage a hidden integration branch. If the
workspace has uncommitted changes, `start_task` returns an error instead of
creating a child worktree. When `base`/`base_branch` is omitted, the task starts
from the current `HEAD`.

Parallelism is explicit: the primary agent calls `start_task` multiple times,
selects workers based on available slots and task fit, then reviews each result
independently.

Tasks may include optional `context` or `context_files` supplied by the primary
agent. This is the right way to give a child agent uncommitted diffs, temporary
audit notes, or other context that is not present in the child worktree's base
commit.

`read_scope`/`write_scope` must not overlap `forbidden_paths`. SubDispatch
rejects contradictory scope contracts before creating a task worktree. The
managed result manifest path is the only internal `.subdispatch` write that a
child task is expected to perform.

### `poll_tasks`

Returns factual task status globally, optionally filtered by `task_ids`,
`status`, or `active_only`. Polling refreshes process state and starts queued
tasks when worker slots open.

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
- compact validation command results from the Claude transcript
- compact forbidden-path attempts observed by task-scoped hooks
- changed files
- diff
- patch path
- base commit
- task branch
- write-scope check
- forbidden-path check

Treat the manifest as worker self-report. Git diff, scope checks,
`transcript_tool_results_tail`, and `forbidden_path_attempts_tail` are stronger
review evidence.

### `delete_worktree`

Deletes one SubDispatch-managed task worktree. It refuses to delete a running
task unless forced. By default it preserves the branch and artifact directory.

## Hard constraints

- Child agents never run in the primary worktree.
- Every task has its own branch.
- Every task has its own worktree.
- Every task records a base commit.
- `start_task` refuses dirty primary workspaces.
- `collect_task` uses Git as the source of truth.
- Worktree deletion verifies the target is under the SubDispatch worktree root.
- Artifacts are preserved by default.
- Worker concurrency limits are enforced.

## Rust CLI

During local development:

```bash
cargo run -- workers --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

Packaged usage is the same without `cargo run --`:

```bash
subdispatch workers --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

The Web UI is intentionally not a task console. It provides Setup checks,
`.env` initialization, worker capacity, task status, changed-file counts,
and Claude hook activity. The primary LLM still creates tasks through MCP or
CLI.

## Install And Release

Install the global MCP entry and bundled skill once:

```bash
subdispatch install-skill
subdispatch install --global
```

Then initialize each project:

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

Create a local release archive:

```bash
scripts/release.sh
```

See [docs/rust-release.md](docs/rust-release.md) for packaging details and
[docs/python-removal-plan.md](docs/python-removal-plan.md) for the Python MVP
removal record.
