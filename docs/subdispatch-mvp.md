# SubDispatch MVP

SubDispatch is a local scaffold for a primary LLM to run child coding agents in
parallel. The primary LLM owns planning, review, merge decisions, and conflict
resolution. SubDispatch only provides isolated execution, status polling,
artifact collection, and worktree cleanup.

## Non-goals

- Automatic task planning
- Automatic review
- Automatic merge or cherry-pick
- Conflict resolution
- Product renaming
- Multi-provider abstraction

## Core model

SubDispatch tracks three entities:

- `Worker`: a configured external coding-agent command. The MVP default is
  `claude-code`.
- `Run`: one group of child tasks launched from a shared base commit.
- `Task`: one child-agent execution in its own branch and git worktree.

Each task records its base commit, branch, worktree path, process id, logs,
result manifest path, Claude hook/session evidence, and artifact directory.

## Configuration

SubDispatch reads project-local configuration from `.env` in the workspace root.
`.env` is git-ignored. `.env.example` documents the supported keys.

Create the local file with:

```bash
python cli.py init-env
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

### `poll_run`

Returns factual task status for a run. Polling refreshes process state and
starts queued tasks when worker slots open.

`poll_run` is also the primary observability surface. A running child agent may
spend a long time planning before stdout, stderr, or git diff changes. The
primary agent should not infer failure from silence. SubDispatch reports:

- runtime seconds
- changed file count
- hook event count
- last hook event name
- last hook event timestamp
- transcript path
- agent transcript path
- last tool name
- last assistant message tail

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
- Claude hook summary
- Claude hook event tail
- Claude transcript tail, when available
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
- Process state, hook events, session transcript, and Git artifacts are separate
  signals. The primary agent decides whether to wait, collect, or clean up.
