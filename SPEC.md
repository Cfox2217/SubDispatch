# SubDispatch — Spec Document

## 1. Concept & Vision

**SubDispatch** is a local scaffold for a primary LLM to run child coding agents
in parallel. The primary LLM owns planning, review, merge decisions, and conflict
resolution. SubDispatch only provides isolated execution, status polling,
artifact collection, and worktree cleanup.

The philosophy: _let the primary agent think; let SubDispatch handle execution._

**Tagline:** _Parallel child agents, isolated worktrees._

---

## 2. Design Language

### Visual Identity

- **Name:** SubDispatch
- **Tagline:** Parallel child agents, isolated worktrees.
- **Primary palette:** quiet local-tool UI, readable structured status, minimal chrome
- **Font:** system UI for Web, structured JSON for CLI/MCP
- **UI boundary:** Setup and Activity only. No human task creation, review, merge,
  or approval panel in v1.

### Log Output Aesthetic

```
[SubDispatch] Starting run abc123 with 4 tasks
[SubDispatch] Task t1 queued, worker claude-code available
[SubDispatch] Task t2 started in worktree /tmp/subdispatch/run-abc123/t2
[claude-code] Task t2 completed in 23s
[SubDispatch] Task t2 completed, collecting artifacts
[SubDispatch] Run abc123: 4 completed, 0 failed
```

### README Tone

Engineer-first: no fluff, concise structure, interface-focused.

---

## 3. Architecture

```
Primary LLM (Codex / Claude Code)
  ↓
SubDispatch MCP Server / CLI
  ├─ list_workers    (capacity + availability)
  ├─ start_run       (create tasks, dispatch workers)
  ├─ poll_run        (refresh status, start queued)
  ├─ collect_task    (git diff, artifact collection)
  └─ delete_worktree (cleanup, preserve artifacts)
  ↓
Claude Code (default worker)
  ↓
Isolated Git Worktrees
```

### MCP Integration

SubDispatch registers as an MCP server via project-level `.codex/config.toml`.
The Rust binary is the forward path:

```toml
[mcp_servers.subdispatch]
command = "subdispatch"
args = ["mcp", "--workspace", "."]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

The Python MVP entry is retained only as a migration reference:

```toml
[mcp_servers.subdispatch]
command = "python"
args = ["subdispatch_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

### Data Flow

1. **MCP Input** (tool call from primary):
   ```json
   {
     "tasks": [
       {"task_id": "t1", "instruction": "Add unit tests for user service", "branch": "feat/t1"},
       {"task_id": "t2", "instruction": "Update README", "branch": "feat/t2"}
     ],
     "base_commit": "abc123"
   }
   ```

2. **MCP Output** (tool response):
   ```json
   {
     "run_id": "run-xyz",
     "tasks": [
       {"task_id": "t1", "status": "queued"},
       {"task_id": "t2", "status": "running", "worktree": "/tmp/subdispatch/run-xyz/t2"}
     ]
   }
   ```

3. **poll_run** response:
   ```json
   {
     "run_id": "run-xyz",
     "status": "completed",
     "tasks": [
       {"task_id": "t1", "status": "completed", "artifacts": "..."},
       {"task_id": "t2", "status": "failed", "reason": "..."}
     ]
   }
   ```

---

## 4. Entities

### Worker

- `id`: unique worker identifier
- `command`: runner command template
- `model`: configured model (optional)
- `max_concurrency`: slot limit
- `running`: current running count
- `queued`: pending count
- `available_slots`: max_concurrency - running
- `status`: `available` | `busy` | `disabled`

### Run

- `run_id`: unique run identifier
- `base_commit`: shared base commit for all tasks
- `tasks`: list of task records
- `status`: `running` | `completed` | `failed` | `cancelled`

### Task

- `task_id`: unique task identifier within a run
- `branch`: dedicated git branch
- `worktree`: isolated git worktree path
- `status`: `queued` | `running` | `completed` | `failed` | `cancelled` | `missing`
- `instruction`: original prompt
- `result_manifest_path`: worker output path
- `artifact_dir`: collected artifacts directory
- `pid`: worker process id
- `logs`: stdout/stderr tails
- `context`: optional primary-agent supplied inline context
- `context_files`: optional primary-worktree files to embed into the prompt

---

## 5. Interfaces

### `list_workers`

Returns worker capacity and availability:

```json
{
  "workers": [
    {
      "id": "claude-code",
      "command": "claude -p $prompt ...",
      "model": "claude-sonnet-4-5",
      "max_concurrency": 2,
      "running": 1,
      "queued": 0,
      "available_slots": 1,
      "status": "available"
    }
  ]
}
```

### `start_run`

Creates tasks, branches, worktrees, and dispatches workers:

```json
{
  "run_id": "run-xyz",
  "tasks": [
    {"task_id": "t1", "status": "queued"},
    {"task_id": "t2", "status": "running", "worktree": "/tmp/subdispatch/run-xyz/t2"}
  ]
}
```

### `poll_run`

Refreshes task status and starts queued tasks:

```json
{
  "run_id": "run-xyz",
  "status": "completed",
  "tasks": [
    {"task_id": "t1", "status": "completed", "artifacts": "/tmp/subdispatch/run-xyz/t1"},
    {"task_id": "t2", "status": "failed", "reason": "non-zero exit"}
  ]
}
```

### `collect_task`

Returns Git-based artifact for one task:

```json
{
  "task_id": "t1",
  "instruction": "Add unit tests",
  "status": "completed",
  "base_commit": "abc123",
  "branch": "feat/t1",
  "changed_files": ["src/user/service.ts", "tests/user/service.test.ts"],
  "diff": "...",
  "patch_path": "/tmp/subdispatch/run-xyz/t1/patch.diff",
  "worker_manifest": {"files": ["tests/user/service.test.ts"]},
  "stdout_tail": "...",
  "stderr_tail": "..."
}
```

### `delete_worktree`

Deletes a task worktree:

```json
{
  "task_id": "t1",
  "deleted": true,
  "preserved_branch": true,
  "preserved_artifacts": true
}
```

---

## 6. Hard Constraints

1. Child agents never run in the primary worktree.
2. Every task has its own branch.
3. Every task has its own worktree.
4. Every task records a base commit.
5. `collect_task` uses Git as the source of truth.
6. Worktree deletion verifies the target is under the SubDispatch worktree root.
7. Artifacts are preserved by default.
8. Worker concurrency limits are enforced.

---

## 7. Configuration

SubDispatch reads from `.env` in the workspace root:

| Variable | Description |
|---|---|
| `SUBDISPATCH_WORKER_MODE` | `trusted-worktree` (MVP only) |
| `SUBDISPATCH_CLAUDE_ENABLED` | Enable claude-code worker |
| `SUBDISPATCH_CLAUDE_PERMISSION_MODE` | `bypassPermissions` (MVP default) |
| `SUBDISPATCH_CLAUDE_COMMAND` | Worker command template |
| `SUBDISPATCH_CLAUDE_MODEL` | Optional model override |
| `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY` | Max parallel tasks |
| `ANTHROPIC_API_KEY` | Optional API key |
| `ANTHROPIC_BASE_URL` | Optional base URL |

---

## 8. File Structure

```
subdispatch/
├── SPEC.md
├── README.md
├── README_zh.md
├── docs/
│   └── subdispatch-mvp.md
├── subdispatch/
│   ├── __init__.py
│   ├── models.py
│   ├── runner.py
│   ├── worktree.py
│   └── cli.py
├── subdispatch_mcp.py
├── .env.example
└── AGENTS.md
```

---

## 9. Out of Scope (MVP)

- Learning-based routing
- Web dashboard
- Multi-workspace support
- Worker session resume
- Non-git isolation

---

_This spec defines the SubDispatch MVP._
