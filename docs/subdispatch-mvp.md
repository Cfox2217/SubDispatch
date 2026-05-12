# SubDispatch Core Runtime

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

SubDispatch tracks two entities:

- `Worker`: a configured external coding-agent command. The default is
  `claude-code`.
- `Task`: one child-agent execution in its own branch and git worktree.

Each task records its base commit, branch, worktree path, process id, logs,
result manifest path, Claude hook/session evidence, and artifact directory.

## Configuration

SubDispatch reads project-local configuration from `.env` in the workspace root.
`.env` is git-ignored. `.env.example` documents the supported keys.

Create the local file with:

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

Prompt configuration is stored in `.subdispatch/prompts.json`. It is optional:
SubDispatch uses built-in defaults until the user saves overrides from the Web
UI. Prompt configuration covers primary-agent guidance, MCP tool descriptions,
child-agent template/safety rules/manifest schema, review guidance, and worker
profile hints. Changes apply to new MCP tool listings and new child tasks; they
do not rewrite existing tasks.

The child-agent template supports these placeholders:

- `{{goal}}`
- `{{instruction}}`
- `{{read_scope}}`
- `{{write_scope}}`
- `{{forbidden_paths}}`
- `{{result_path}}`
- `{{manifest_schema}}`
- `{{safety_rules}}`
- `{{context_block}}`

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

`delegation_trust` is a routing hint for the primary agent. It describes how
willing the primary agent should be to delegate a fitting task to that worker;
it does not replace scope control, review, or validation.

### `start_task`

Starts one primary-LLM supplied child task. SubDispatch creates a branch and
worktree, writes a task prompt, and starts the configured worker when capacity
is available. A task over the worker concurrency limit stays queued.

Delegation requires a clean committed checkpoint. The primary agent owns its own
branch/worktree strategy and must commit any in-progress changes before calling
`start_task`. SubDispatch does not manage a hidden integration branch. If the
workspace has uncommitted changes, `start_task` returns an error instead of
creating a child worktree.

If `base`/`base_branch` is omitted, the task starts from the current `HEAD`.
Passing `base` remains an explicit override for special cases.

Parallelism is explicit: the primary agent calls `start_task` multiple times,
selects workers based on available slots and task fit, then reviews each result
independently.

### `poll_tasks`

Returns factual task status globally, optionally filtered by `task_ids`,
`status`, or `active_only`. Polling refreshes process state and starts queued
tasks when worker slots open.

`poll_tasks` is also the primary observability surface. A running child agent may
spend a long time planning before stdout, stderr, or git diff changes. The
primary agent should not infer failure from silence. SubDispatch reports:

- runtime seconds
- idle seconds for running tasks, measured since the latest hook event or task start
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
