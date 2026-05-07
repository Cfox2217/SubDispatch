# CodexSaver

> Make Codex cheaper without making it dumber.

![CodexSaver](./CodexSaver.png)

CodexSaver is an MCP tool that turns Codex into a cost-aware router.
It pushes low-risk development work to DeepSeek, keeps high-risk judgment in Codex,
and returns enough interaction detail that you can feel when the tool is active.

- Lower-cost execution for tests, docs, search, and explanation work
- Codex stays responsible for architecture, security, protected domains, and final review
- One-time local API key setup, then real delegated calls without re-exporting env vars
- Verified with live calls, end-to-end checks, and a five-task benchmark

---

## Why This Exists

Most coding sessions contain two very different kinds of work:

- expensive thinking
- cheap execution

Codex is excellent at the first one. It is overqualified for much of the second.

CodexSaver splits the flow on purpose:

- `Codex` handles reasoning, ambiguity, protected domains, and approval
- `DeepSeek` handles low-risk throughput work

That gives you a practical pattern:

```text
Use the expensive model for judgment.
Use the cheaper model for volume.
Never confuse the two.
```

---

## What It Feels Like

When CodexSaver is active, tool responses are not silent blobs of JSON.
They include an `interaction` block that makes the routing decision visible:

```json
{
  "interaction": {
    "tool": "codexsaver.delegate_task",
    "mode": "delegated_execution",
    "headline": "CodexSaver delegated this task to DeepSeek.",
    "route_label": "[CodexSaver] route=deepseek task_type=write_tests risk=low",
    "next_step": "Review the worker result and apply it only if the patch looks safe."
  }
}
```

Three states matter:

- `preview`: routing preview only, no external model call
- `delegated_execution`: delegated run completed
- `codex_takeover`: task stayed with Codex because risk was too high or the task was ambiguous

---

## Quick Start

### Manual Install

```bash
git clone https://github.com/yourname/codexsaver
cd codexsaver

python cli.py auth set --api-key YOUR_DEEPSEEK_API_KEY
python cli.py install --project
python cli.py doctor
```

If you also want CodexSaver available outside this repo:

```bash
python cli.py install --global
python cli.py doctor
```

If you prefer a temporary one-shell-session setup instead of saving the key locally:

```bash
export DEEPSEEK_API_KEY=YOUR_DEEPSEEK_API_KEY
python cli.py install --project
python cli.py doctor
```

### One Message To Codex

If Codex is already open in this repository, you can just say:

```text
Save my DeepSeek API key for CodexSaver, run `python cli.py auth set --api-key ...`, then run `python cli.py install --project` and `python cli.py doctor`, and tell me whether it is ready.
```

For project plus global setup:

```text
Save my DeepSeek API key for CodexSaver, install CodexSaver for this repo and globally, run `python cli.py auth set --api-key ...`, `python cli.py install --project`, `python cli.py install --global`, then `python cli.py doctor`, and summarize the result.
```

Ready means:

- `.codex/config.toml` exists in the repo
- `codexsaver_mcp.py` exists
- `python cli.py doctor` reports the workspace is ready
- a DeepSeek API key is available from either `DEEPSEEK_API_KEY` or local CodexSaver config

---

## 60-Second Demo

Project MCP config:

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["./codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

Then tell Codex:

```text
Use CodexSaver for safe low-risk tasks.
Add unit tests for user service.
```

Or call the CLI directly:

```bash
python cli.py delegate "Explain the routing logic briefly" --files codexsaver/router.py --workspace .
```

Dry run:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts --workspace . --dry-run
```

Real run:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts --workspace .
```

---

## Verified Setup Flow

Measured on May 7, 2026 with the local-key workflow:

| Check | Command | Result |
|---|---|---|
| Full test suite | `pytest -q` | `71 passed in 0.31s` |
| Project install | `python cli.py install --project --workspace .` | `status=ok`, project config already correct |
| Local key persistence | `python cli.py auth set --api-key ...` | saved to `~/.codexsaver/config.json` |
| Workspace doctor | `python cli.py doctor --workspace .` | `deepseek_api_key_source=local_config`, workspace ready |
| Real delegated call | `python cli.py delegate "Explain the routing logic briefly" --files codexsaver/router.py --workspace .` | `route=deepseek`, `status=success`, verification passed |

This is the intended workflow:

1. Save the key once
2. Install CodexSaver into the workspace
3. Confirm readiness with `doctor`
4. Use real delegated calls without re-exporting `DEEPSEEK_API_KEY`

---

## Post-Setup Usage Ratio

After setup completed, I measured the actual routed tasks in this working session.
I only counted tasks that truly entered model routing, not local commands like `pytest`,
`git`, `install`, `doctor`, or README editing.

Result:

- `DeepSeek`: `7 / 8 = 87.5%`
- `Codex`: `1 / 8 = 12.5%`

Why not 100%?

One test-writing prompt originally included the phrase `production logic`.
That triggered the router's intentional high-risk keyword guard and returned the task to Codex.
This was not a failure. It was the protection logic working as designed.

If you only count the later standardized five-task benchmark with natural low-risk phrasing,
the delegation ratio was:

- `DeepSeek`: `5 / 5 = 100%`
- `Codex`: `0 / 5 = 0%`

Takeaway:

- In real usage, CodexSaver defaulted to DeepSeek for most low-risk work
- It still preserved a strict fallback path for risky wording and protected domains

---

## Five-Task A/B Benchmark

Method:

- **A** = counterfactual `Codex-only` baseline with normalized cost index fixed at `1.00`
- **B** = `CodexSaver` mode with the live router and DeepSeek worker
- latency is wall-clock time for the real CodexSaver execution
- savings come from the current `CostEstimator`, so this is a reproducible routing benchmark, not invoice-grade billing data

Summary:

- All 5 tasks were typical low-risk development chores: explanation, docs, tests, and README maintenance
- All 5 delegated successfully after using natural low-risk phrasing
- Average live latency was `6.18s`
- Average estimated savings were `48.4%`
- Average normalized cost moved from `1.00` to `0.52`
- Estimated relative reduction was `48.0%`

| Task | Type | Route | Latency | A: Codex-only Cost Index | B: CodexSaver Cost Index | Estimated Savings | Output Shape |
|---|---|---|---:|---:|---:|---:|---|
| Explain router logic | `explain` | `deepseek` | `2.13s` | `1.00` | `0.55` | `45%` | read-only summary |
| Document router module | `docs` | `deepseek` | `3.13s` | `1.00` | `0.55` | `45%` | 1-file patch |
| Add cost tests | `write_tests` | `deepseek` | `9.29s` | `1.00` | `0.55` | `45%` | test patch |
| Explain verifier flow | `explain` | `deepseek` | `2.30s` | `1.00` | `0.55` | `45%` | read-only summary |
| Update install docs | `docs` | `deepseek` | `14.06s` | `1.00` | `0.38` | `62%` | README patch |

![Five-task benchmark](./assets/ab-test-benchmark.svg)

Figure:
Gray bars are the `Codex-only` baseline fixed at `100`.
Green bars are the `CodexSaver` cost index for the same task.
Lower bars mean lower estimated Codex spend.

Interpretation:

- Read-only explain tasks were the fastest, cleanest wins
- Small docs edits delegated well and returned compact, reviewable patches
- Test generation had higher latency than explanation, but still stayed in the low-risk savings band
- Larger-context documentation work produced the biggest estimated savings because the Codex-only context cost would be higher

---

## Routing Rules

### Good Tasks To Delegate

- repo scanning and code search
- code explanation and summarization
- writing unit tests
- fixing lint or type errors
- documentation updates
- boilerplate generation
- small localized refactors

### Tasks Kept In Codex

- architecture decisions
- auth, security, payment, billing, or permissions logic
- database migrations
- deployment and production operations
- ambiguous product requests
- final review before applying changes

### Why Some Medium-Risk Tasks Still Delegate

CodexSaver does not just ask:

```text
Is this code work?
```

It asks:

```text
Is this code work cheap enough to delegate without losing judgment quality?
```

That creates a deliberate asymmetry:

- read-only understanding can be cheap
- writes in sensitive domains are expensive in risk even if the diff is small
- ambiguity defaults to Codex, not delegation

That is why `Explain auth code` may still delegate while `Refactor auth service` stays in Codex.

---

## How It Works

```text
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

Core modules:

- `Router`: classify tasks and assign risk
- `ContextPacker`: bound file context before delegation
- `DeepSeekClient`: call the worker model
- `Verifier`: validate output shape, protected paths, and suggested commands
- `CostEstimator`: estimate relative savings bands

---

## Security And Persistence

- `python cli.py auth set --api-key ...` saves the key to `~/.codexsaver/config.json`
- `doctor` shows whether the key comes from the environment or the local config
- live calls use local config automatically if `DEEPSEEK_API_KEY` is not exported
- if verification fails, CodexSaver falls back to `needs_codex`

---

## Commands

```bash
python cli.py auth set --api-key YOUR_DEEPSEEK_API_KEY
python cli.py install --project
python cli.py install --global
python cli.py doctor
python cli.py delegate "Explain the routing logic briefly" --files codexsaver/router.py --workspace .
```

---

## Roadmap

- [x] MCP server
- [x] rule-based routing
- [x] bounded context packing
- [x] DeepSeek integration
- [x] local API key persistence
- [x] interaction-aware tool responses
- [x] end-to-end verification flow
- [ ] cost-aware dynamic routing
- [ ] multi-model support

---

## If This Saves You Money

Star the repo.
