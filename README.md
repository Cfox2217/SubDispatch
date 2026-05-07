# 💸 CodexSaver

> Cut your Codex cost by 40–70% — without sacrificing quality.

![CodexSaver](./CodexSaver.png)

---

## What is CodexSaver?

CodexSaver turns Codex into a cost-aware router.
Low-risk coding work goes to DeepSeek API.
High-value reasoning and final approval stay in Codex.

- 🧠 Codex → reasoning, architecture, validation
- ⚡ DeepSeek → execution, search, boilerplate, tests

---

## Demo

```
Input:
"Add unit tests for user service"

[Codex] Low-risk test generation task detected
[Codex] Calling codexsaver.delegate_task

[CodexSaver] route=deepseek
[Router] task_type=write_tests risk=low
[DeepSeek] Generated patch
[Verifier] Structural verification passed
[Saved] Estimated Codex saving: 62%

[Codex] Reviewing patch
[Codex] Approved after verification
```

---

## Cost Comparison

```
Task: "Add tests + fix lint"

Before:
  Codex only: $0.52

After:
  CodexSaver: $0.18

Saved: 65%
```

Current estimator bands in code:

| Delegated context size | Estimated savings |
|---|---:|
| `< 8k` chars | `45%` |
| `8k–50k` chars | `62%` |
| `> 50k` chars | `70%` |

These percentages are heuristic outputs from the current `CostEstimator`, not live
billing data from Codex or DeepSeek. They are useful for routing comparisons and README
demos, but not yet for finance-grade reporting.

---

## Architecture

```
User
  ↓
Codex
  ↓ MCP tool call
CodexSaver
  ├─ Router
  ├─ Context Packer
  ├─ DeepSeek API Worker
  ├─ Verifier
  └─ Cost Estimator
  ↓
Codex review / apply / finalize
```

---

## Install

```bash
git clone https://github.com/yourname/codexsaver
cd codexsaver

export DEEPSEEK_API_KEY=xxx
```

---

## Use with Codex

Project config (`.codex/config.toml`):

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["./codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

Global Codex config (`~/.codex/config.toml`) also works:

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["./codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

If you want to use it from outside the repo directory, point `args` at the cloned
project path on your own machine:

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["/absolute/path/to/codexsaver/codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

Verified on May 7, 2026:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"codexsaver","version":"0.2.0"}}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"delegate_task"}]}}
```

Then tell Codex:

```
Use CodexSaver for safe low-risk tasks.
Add unit tests for user service.
```

---

## CLI Test

Dry run:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts --workspace . --dry-run
```

Real call:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts --workspace .
```

CodexSaver resolves relative file paths from `workspace` and executes worker-proposed
`commands_to_run` during verification. If any verification command fails, the task falls
back to Codex with `needs_codex`.

---

## Verified Routing Contrast

Low-risk task on May 7, 2026:

```bash
python cli.py "add unit tests for user service" --files cli.py --workspace . --dry-run
```

```json
{
  "route": "deepseek",
  "status": "dry_run",
  "decision": {
    "task_type": "write_tests",
    "risk": "low"
  },
  "estimated_savings_percent": 45
}
```

High-risk task on May 7, 2026:

```bash
python cli.py "fix security vulnerability in auth flow" --files codexsaver/router.py --workspace . --dry-run
```

```json
{
  "route": "codex",
  "status": "needs_codex",
  "decision": {
    "risk": "high",
    "protected_hits": ["security"]
  },
  "estimated_savings_percent": 0
}
```

This is the intended split:

- Low-risk execution gets delegated.
- High-risk/security-sensitive work stays in Codex.

### Quantified Routing Samples

Measured with `python cli.py ... --dry-run` on May 7, 2026:

| Task | Task Type | Risk | Route | Estimated Savings |
|---|---|---|---|---:|
| `Add unit tests for user service` | `write_tests` | `low` | `deepseek` | `45%` |
| `Explain the routing logic` | `explain` | `low` | `deepseek` | `45%` |
| `Update README usage docs` | `docs` | `low` | `deepseek` | `45%` |
| `Explain auth code` | `explain` | `medium` | `deepseek` | `45%` |
| `Add tests across six files` | `write_tests` | `medium` | `deepseek` | `45%` |
| `Refactor auth service` | `simple_refactor` | `high` | `codex` | `0%` |
| `Fix security vulnerability in auth flow` | `unknown` | `high` | `codex` | `0%` |
| `Design new authentication architecture` | `unknown` | `high` | `codex` | `0%` |

In this sample set:

- `5 / 8` tasks were delegated.
- `3 / 8` tasks were kept in Codex.
- All `high` risk tasks stayed in Codex.
- `medium` risk read-only work still delegated.

### Live API Report

Measured with real DeepSeek-backed invocations on May 7, 2026:

| Case | Task | Route | Status | Latency | Changed Files | Patch Size | Response Size | Estimated Savings |
|---|---|---|---|---:|---:|---:|---:|---:|
| Read-only analysis | `Explain the routing logic and summarize protected path handling` | `deepseek` | `success` | `1.55s` | `0` | `0 chars` | `778 chars` | `45%` |
| Small docs edit | `Add concise module-level documentation to router.py without changing behavior` | `deepseek` | `success` | `3.22s` | `1` | `277 chars` | `1108 chars` | `45%` |

Observed from these live calls:

- CodexSaver completed a read-only delegated task in `1.55s`.
- CodexSaver completed a small patch-producing delegated task in `3.22s`.
- The patch-producing call returned a compact `277` character diff for one file.
- Both calls passed verification, but neither suggested follow-up commands.

This gives a practical split for README claims:

- `dry_run` demonstrates routing policy.
- real API calls demonstrate actual delegated execution.
- protected/high-risk tasks can still be shown locally without making unnecessary outbound calls.

### Routing Logic Analysis

CodexSaver does not ask "is this coding work?" first. It asks a stricter question:
"is this coding work cheap enough to delegate without losing judgment quality?"

That logic currently has four layers:

1. **Task classification**
   Low-risk categories such as `write_tests`, `docs`, `code_search`, `explain`,
   `fix_lint`, `boilerplate`, and `simple_refactor` are eligible for delegation.

2. **Instruction risk scan**
   Keywords like `security`, `authentication`, `billing`, `migration`, `deploy`,
   `encrypt`, and `token` immediately raise risk because they usually require more than
   syntax-level correctness.

3. **Path/domain risk scan**
   File paths containing domains such as `auth`, `payments`, `billing`, `infra`,
   `migrations`, `.github/workflows`, or secrets-related terms are treated as protected.
   That blocks or limits delegation even when the task wording looks harmless.

4. **Safe exceptions for read-only work**
   Some `medium` risk tasks still delegate when they are mostly observational:
   `explain`, `docs`, `code_search`, and `write_tests`. This is why
   `Explain auth code` can still route to DeepSeek while `Refactor auth service` stays
   in Codex.

This produces a deliberate asymmetry:

- Read-only understanding can be cheap.
- Write access to sensitive domains is expensive in risk, even if the code change is small.
- Ambiguity defaults to Codex, not to delegation.

---

## Task Routing

### Delegated to DeepSeek

- repo scanning and code search
- code explanation and summarization
- writing unit tests
- fixing lint/type errors
- documentation updates
- boilerplate generation
- small, localized refactors

### Kept in Codex

- architecture decisions
- auth/security/payment logic
- database migrations
- permissions or access-control changes
- production deployment logic
- ambiguous requirements
- final review before applying changes

---

## Roadmap

- [x] MCP server (`codexsaver.delegate_task`)
- [x] Rule-based router
- [x] DeepSeek API integration
- [x] Context packing
- [ ] Cost-aware routing
- [ ] Multi-model support

---

## If this saves you money

Give it a star ⭐
