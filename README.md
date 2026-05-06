# LeanCodex

> Make Codex cheaper without making it dumber.

LeanCodex is a hybrid AI coding router that delegates low-cost, verifiable tasks to DeepSeek while keeping Codex in charge of high-quality reasoning and final validation.

**Cut your Codex cost by 40–70% with near-zero quality loss.**

---

## Why LeanCodex?

Codex is powerful — but expensive. LeanCodex introduces a hybrid execution model:

- **🧠 Codex** = reasoning, architecture, validation
- **⚡ DeepSeek** = execution, boilerplate, search, tests

**Result:**
- 💰 40–70% lower Codex token usage
- ⚖️ Same (or better) output quality
- 🔁 Automatic fallback to Codex when needed

---

## Architecture

```
User
  ↓
Codex (Primary Agent)
  ↓ shell tool call
LeanCodex Router
  ├─ Task Classifier
  ├─ Risk Scorer
  ├─ Context Packer
  ├─ Task Runner → DeepSeek-TUI
  ├─ Verifier
  └─ Fallback Engine
  ↓
DeepSeek does the work. Codex makes the decisions.
```

---

## Task Routing Strategy

### Delegated to DeepSeek
- code search / repo scan
- writing tests
- simple refactors
- fixing lint/type errors
- documentation updates

### Kept in Codex
- architecture decisions
- security-sensitive logic
- migrations
- ambiguous requirements
- final review before commit

---

## Quick Start

### 1. Install DeepSeek-TUI

```bash
git clone https://github.com/Hmbown/DeepSeek-TUI
cd DeepSeek-TUI
cargo build --release
```

### 2. Run LeanCodex

```bash
./leancodex --task-json task.json
```

---

## Task Protocol (JSONL over stdio)

### Input (`task.json`)
```json
{
  "id": "task_001",
  "instruction": "add unit tests for user service",
  "workspace": "./repo",
  "constraints": ["do not modify production logic", "tests must pass"]
}
```

### Output (JSONL streaming)
```jsonl
{"type":"started","id":"task_001"}
{"type":"progress","message":"scanning files"}
{"type":"file_change","path":"tests/user.test.ts"}
{"type":"completed","status":"success","diff":"..."}
```

---

## Risk Control

LeanCodex prevents low-quality delegation:

- Sensitive paths are protected (auth/*, billing/*, migrations/*)
- Large diffs trigger Codex takeover
- Failed tests → fallback to Codex
- High-risk tasks never delegated

**DeepSeek can write code. Codex must approve it.**

---

## Cost Reduction

| Scenario | Cost |
|----------|------|
| Codex only | $0.42 |
| LeanCodex  | $0.13 |

Typical results:
- ↓ 40–70% Codex token usage
- ↓ large-context prompts
- ↑ throughput via cheap parallel tasks

---

## MVP Roadmap

- [x] CLI delegation (deepseek exec)
- [x] Basic router (rule-based)
- [x] Test verification
- [ ] HTTP runtime support
- [ ] Smarter cost-aware routing
- [ ] Learning-based policy

---

## Philosophy

> Don't replace Codex. Shrink it.

Let cheap models do the work. Let expensive models do the thinking.
