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

Then tell Codex:

```
Use CodexSaver for safe low-risk tasks.
Add unit tests for user service.
```

---

## CLI Test

Dry run:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts --dry-run
```

Real call:

```bash
python cli.py "add unit tests for user service" --files src/user/service.ts
```

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
