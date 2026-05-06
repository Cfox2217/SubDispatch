# LeanCodex — Spec Document

## 1. Concept & Vision

**LeanCodex** is a hybrid AI coding router that delegates low-cost, verifiable tasks to DeepSeek while keeping Codex in charge of high-quality reasoning and final validation. The philosophy: _don't replace Codex, shrink it_. Let cheap models do the work; let expensive models do the thinking.

The tool is delivered as a standalone CLI wrapper (`leancodex`) that speaks JSONL over stdio, making it drop-in compatible with any Codex-like agent via shell tool calls. The primary interface is the Unix pipe: Codex calls `leancodex < task.json`, LeanCodex routes and executes, streams JSONL progress to stdout, and returns a validated result.

**Slogan:** _Make Codex cheaper without making it dumber._

---

## 2. Design Language

### Visual Identity

- **Name:** LeanCodex
- **Tagline:** Make Codex cheaper without making it dumber.
- **Primary palette:** terminal-native, monospace-first, minimal chrome
- **Font:** system monospace (no web fonts needed for CLI tool)
- **No external UI** — this is a CLI tool; all feedback is text/JSONL

### Log Output Aesthetic

Every invocation produces structured, human-readable log lines that feel "intelligent":

```
[Router] Delegating to DeepSeek (low-risk task, score=2)
[DeepSeek] Generated 6 tests in 3.2s
[Verifier] All tests passed
[Codex] Final review complete
[Saved] 62% token cost reduction
```

### README Tone

Engineer-first: no fluff, numbers first, demo immediately visible.

---

## 3. Architecture

```
User
  ↓
Codex (Primary Agent)
  ↓ shell tool call / MCP / local command
LeanCodex Router
  ├─ Task Classifier     (rule-based: delegate or keep)
  ├─ Risk Scorer        (file_risk + task_risk + diff_size + test_confidence)
  ├─ Context Packer     (prune workspace context for DeepSeek)
  ├─ Task Runner        (calls DeepSeek-TUI via CLI or HTTP)
  ├─ Verifier           (run tests, lint, parse diff)
  └─ Fallback Engine    (on failure → escalate to Codex)
  ↓
DeepSeek-TUI
  ├─ one-shot CLI:  deepseek --model auto "task"
  ├─ HTTP runtime:  deepseek serve --http
  └─ ACP stdio:     deepseek serve --acp
  ↓
Codex (validates and finalizes output)
```

### Data Flow

1. **Input** (`task.json` on stdin or file):
   ```json
   {
     "id": "task_001",
     "workspace": "/repo",
     "mode": "plan|edit|test",
     "model": "auto",
     "instruction": "add unit tests for user service",
     "allowed_paths": ["src/user", "tests/user"],
     "forbidden_paths": ["infra", ".env", "migrations"],
     "acceptance": ["do not modify production logic", "tests must pass"]
   }
   ```

2. **Output** (JSONL on stdout, stderr reserved for DeepSeek debug logs):
   ```jsonl
   {"type":"started","id":"task_001"}
   {"type":"progress","message":"scanning src/user"}
   {"type":"command","cmd":"npm test -- user"}
   {"type":"file_change","path":"tests/user/service.test.ts","kind":"created"}
   {"type":"completed","status":"success","summary":"added 6 tests","diff":"..."}
   ```

3. **Final Result** (wrapped JSON):
   ```json
   {
     "status": "success | failed | needs_codex",
     "summary": "added 6 tests",
     "changed_files": ["tests/user/service.test.ts"],
     "commands_run": ["npm test -- user"],
     "tests": {"command": "npm test -- user", "result": "passed"},
     "risk_notes": [],
     "patch": "git diff output"
   }
   ```

---

## 4. Task Routing Strategy

### Rule-Based Classification

**Delegate to DeepSeek** (low-risk, high-volume):

```yaml
- summarize repository
- locate files
- explain code
- write tests for existing function
- update docs
- simple refactor under 5 files
- fix lint/type errors
- generate migration draft
```

**Keep in Codex** (high-risk, complex judgment):

```yaml
- architecture decision
- security-sensitive change
- auth/payment/permission logic
- database migration with data loss risk
- ambiguous product requirement
- final review before commit
- failed DeepSeek attempt
```

### Risk Scoring

```
risk = file_risk + task_risk + diff_size + test_confidence
```

| Score  | Action                                  |
|--------|-----------------------------------------|
| ≤ 3    | DeepSeek executes directly              |
| 4–6    | DeepSeek executes, Codex validates      |
| ≥ 7    | Codex handles it                        |

**High-risk file paths** (never delegated directly):
```
auth/*
security/*
billing/*
payments/*
migrations/*
infra/*
.github/workflows/*
```

---

## 5. Core Components

### 5.1 Task Classifier

Reads task description and workspace, outputs a routing decision with risk score. Uses keyword matching + path scanning.

### 5.2 Context Packer

Prunes workspace context to fit within DeepSeek's context window. Removes boilerplate, node_modules, build artifacts. Outputs a focused prompt with file references.

### 5.3 Task Runner

Supports three execution modes (priority order):

1. **CLI one-shot** (default, simplest): `deepseek --model auto "<task>"`
2. **HTTP runtime**: `deepseek serve --http` with SSE streaming
3. **ACP stdio**: `deepseek serve --acp` for new sessions

### 5.4 Verifier

After DeepSeek completes:
1. Parse changed files from JSONL output
2. Check forbidden paths were not touched
3. Run project test suite (`npm test`, `pytest`, etc.)
4. Run linter if available
5. Produce final status + diff

### 5.5 Fallback Engine

If any of these occur, LeanCodex returns `needs_codex`:
- Test failures after retry
- Diff touches forbidden paths
- DeepSeek crashes or times out
- Risk score ≥ 7

---

## 6. CLI Interface

```
leancodex [--task-json <path>] [--deepseek-model <model>] [--verbose] [--json]
```

| Flag              | Description                          | Default        |
|-------------------|--------------------------------------|----------------|
| `--task-json`     | Path to task JSON file               | stdin          |
| `--deepseek-model`| Model to pass to DeepSeek-TUI        | `auto`         |
| `--verbose`       | Emit debug logs to stderr             | false          |
| `--json`          | Emit machine-readable JSON on stdout | false          |

When called without arguments, reads `task.json` from stdin.

---

## 7. MVP Implementation Plan

### Week 1: CLI Wrapper

- `deepseek-worker` script (shell + Python)
- Basic task classifier (keyword matching)
- One-shot CLI delegation
- JSONL stdout streaming
- Git diff + test verification

### Week 2: Router Engine

- Task classifier (improved)
- Risk scorer
- Context packer
- Cost tracker
- Fallback to Codex logic

### Week 3: Verification Loop

- Diff parser
- Test runner
- Lint runner
- Changed-file risk policy
- Codex final review prompt

### Week 4: HTTP/SSE Runtime

- Replace one-shot with `deepseek serve --http`
- Streaming state updates
- Task cancellation
- Session resume

---

## 8. Success Metrics

| Metric                              | Target                      |
|-------------------------------------|-----------------------------|
| Codex token cost reduction           | 40–70%                      |
| Task success rate delta              | < 3% degradation             |
| Average completion time delta        | < 20% increase               |
| DeepSeek output re-do rate by Codex  | < 25%                       |
| Test pass rate                       | ≥ current Codex-only baseline |
| High-risk file DeepSeek direct edits | 0                           |

---

## 9. Design Principles

1. **DeepSeek can write, Codex must validate.** The split is by _verification difficulty_, not by _task type alone_.
2. **Fail fast and escalate.** One DeepSeek failure → escalate to Codex. Never burn tokens on retry loops.
3. **Zero friction for Codex.** LeanCodex is invoked as a single shell command; Codex sees only stdout.
4. **Observable.** Every step logs its decision so operators can audit routing behavior.
5. **Token savings first.** Every feature is justified by cost reduction or quality maintenance.

---

## 10. File Structure

```
leancodex/
├── SPEC.md
├── README.md
├── src/
│   ├── __init__.py
│   ├── router.py          # Task classification + risk scoring
│   ├── packer.py          # Context pruning
│   ├── runner.py          # DeepSeek execution
│   ├── verifier.py        # Diff + test verification
│   └── fallback.py        # Escalation logic
├── bin/
│   └── leancodex          # Entry point CLI script
├── tests/
│   └── ...
└── docs/
    └── ...
```

---

## 11. Out of Scope (MVP)

- MCP server (Phase 2)
- Learning-based routing
- Web dashboard
- Multi-workspace support
- DeepSeek session resume (Phase 2)

---

_This spec was designed collaboratively and approved before implementation._
