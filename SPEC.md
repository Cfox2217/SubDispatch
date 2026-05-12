# SubDispatch Spec

## Concept

SubDispatch is a local scaffold for a primary LLM to run child coding agents in
parallel. The primary LLM owns planning, review, merge decisions, and conflict
resolution. SubDispatch only provides isolated execution, status polling,
artifact collection, and worktree cleanup.

The philosophy: let the primary agent think; let SubDispatch handle execution.

Tagline: Parallel child agents, isolated worktrees.

## Architecture

```text
Primary LLM (Codex / Claude Code)
  ↓
SubDispatch MCP Server / CLI
  ├─ list_workers    (capacity + availability)
  ├─ start_task      (create one task worktree and dispatch worker)
  ├─ poll_tasks      (refresh task status, start queued tasks)
  ├─ collect_task    (git diff, artifact collection)
  └─ delete_worktree (cleanup, preserve artifacts)
  ↓
Claude Code or compatible external code-agent CLI
  ↓
Isolated Git Worktrees
```

SubDispatch registers as an MCP server via project-level MCP configuration. The
Rust binary is the runtime path:

```toml
[mcp_servers.subdispatch]
command = "subdispatch"
args = ["mcp", "--workspace", "."]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

The Python MVP has been removed. The Rust binary is the only runtime path.

## Entities

### Worker

- `id`: unique worker identifier
- `command`: runner command template
- `model`: configured model, optional
- `max_concurrency`: slot limit
- `running`: current running count
- `queued`: pending count
- `available_slots`: max_concurrency minus running
- `enabled`: whether the worker can be selected

### Task

- `task_id`: unique task identifier
- `branch`: dedicated git branch
- `worktree`: isolated git worktree path
- `base_commit`: committed checkpoint used as the task base
- `status`: `queued` | `running` | `completed` | `failed` | `cancelled` | `missing`
- `instruction`: original child-agent prompt
- `result_manifest_path`: worker output path
- `artifact_dir`: collected artifacts directory
- `pid`: worker process id
- `logs`: stdout/stderr tails
- `context`: optional primary-agent supplied inline context
- `context_files`: optional primary-worktree files embedded into the prompt

There is no run, group, batch, or session entity in the public model. Parallelism
is expressed by calling `start_task` multiple times.

## Interfaces

### `list_workers`

Returns worker capacity and availability:

```json
{
  "workers": [
    {
      "id": "glm",
      "command": ["claude", "-p", "$prompt"],
      "model": "glm-5.1",
      "max_concurrency": 2,
      "running": 1,
      "queued": 0,
      "available_slots": 1,
      "delegation_trust": "high",
      "enabled": true
    }
  ]
}
```

`delegation_trust` is a primary-agent routing hint, not a security guarantee.
Use it with `strengths`, `cost`, `speed`, `risk`, and available slots to decide
how aggressively to delegate clear tasks to a worker.

### `start_task`

Creates one task branch/worktree and dispatches one worker:

```json
{
  "instruction": "Add focused tests for prompt validation",
  "goal": "Improve prompt config safety",
  "task_id": "prompt_validation_tests",
  "worker": "glm",
  "base": "HEAD",
  "read_scope": ["src/prompts.rs", "src/mcp.rs"],
  "write_scope": ["src/prompts.rs"],
  "forbidden_paths": [".env"],
  "context": "Keep the public interface task-first."
}
```

Response:

```json
{
  "status": "ok",
  "task_id": "prompt_validation_tests",
  "base_commit": "abc123",
  "task": {
    "task_id": "prompt_validation_tests",
    "status": "running",
    "branch": "agent/prompt_validation_tests",
    "worktree": ".subdispatch/worktrees/tasks/prompt_validation_tests"
  }
}
```

Delegation requires a clean committed primary workspace. If the workspace has
uncommitted changes, `start_task` fails without creating a child worktree.
`read_scope`/`write_scope` must not overlap `forbidden_paths`. Contradictory
scope contracts are rejected before task directory or worktree creation. The
managed result manifest path is the only expected internal `.subdispatch`
write by a child task.

### `poll_tasks`

Refreshes task status and starts queued tasks when worker slots open:

```json
{
  "task_ids": ["prompt_validation_tests"],
  "active_only": true
}
```

Response:

```json
{
  "status": "running",
  "tasks": [
    {
      "task_id": "prompt_validation_tests",
      "status": "running",
      "worker": "glm",
      "event_count": 7,
      "changed_files_count": 1
    }
  ]
}
```

### `collect_task`

Returns Git-based evidence for one task:

```json
{
  "task_id": "prompt_validation_tests"
}
```

Response includes:

- original instruction
- worker manifest, if present
- stdout/stderr tails
- Claude hook summary and recent hook events
- compact validation command results from the Claude transcript
- compact forbidden-path attempts observed by task-scoped hooks
- changed files
- diff
- patch path
- base commit
- task branch
- write-scope check
- forbidden-path check

Git diff is the source of truth. The worker manifest is only self-reported
evidence.

### `delete_worktree`

Deletes one managed task worktree:

```json
{
  "task_id": "prompt_validation_tests",
  "force": false,
  "delete_branch": false
}
```

The command refuses to delete a running task unless `force=true`. Branch deletion
must be explicit.

## Hard Constraints

1. Child agents never run in the primary worktree.
2. Every task has its own branch.
3. Every task has its own worktree.
4. Every task records a base commit.
5. `start_task` refuses dirty primary workspaces.
6. `collect_task` uses Git as the source of truth.
7. Worktree deletion verifies the target is under the SubDispatch worktree root.
8. Artifacts are preserved by default.
9. Worker concurrency limits are enforced.

## Configuration

SubDispatch reads from `.env` in the workspace root:

| Variable | Description |
|---|---|
| `SUBDISPATCH_WORKER_MODE` | `trusted-worktree` by default |
| `SUBDISPATCH_CLAUDE_ENABLED` | Enable default Claude Code worker |
| `SUBDISPATCH_CLAUDE_PERMISSION_MODE` | `bypassPermissions` by default |
| `SUBDISPATCH_CLAUDE_COMMAND` | Worker command template |
| `SUBDISPATCH_CLAUDE_MODEL` | Optional model override |
| `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY` | Max parallel tasks |
| `ANTHROPIC_API_KEY` | Optional API key |
| `ANTHROPIC_BASE_URL` | Optional base URL |

Prompt configuration is stored in `.subdispatch/prompts.json` and can be edited
from the Web UI. Prompt changes apply to new MCP tool listings and new tasks.

## UI Boundary

The Web UI is a configuration and observation surface:

- Setup checks
- `.env` editing
- prompt editing
- worker capacity
- task terminal activity
- Claude hook status

It does not create tasks, review diffs, merge, or approve results.
