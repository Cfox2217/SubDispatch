# 💸 CodexSaver

> **Cut your Codex cost by 40–70% — without sacrificing quality.**

---

## ✨ What is CodexSaver?

CodexSaver is a **cost-aware AI coding router**.

It turns Codex into a **smart orchestrator** that delegates low-cost tasks to DeepSeek API, while keeping high-value reasoning in Codex.

- 🧠 Codex → reasoning, architecture, validation
- ⚡ DeepSeek → execution, search, boilerplate, tests

👉 Same quality. Much lower cost.

---

## 🎬 Demo (Simulated)

![Demo GIF](./assets/demo.gif)

```text
Input:
"Add unit tests for auth service"

[Router] Task classified as low-risk
[Router] Delegating to DeepSeek

[DeepSeek] Generated 6 test cases
[Verifier] Running tests...
[Verifier] Tests passed

[Codex] Reviewing patch...
[Codex] Approved

[Saved] 62% Codex token cost
```

---

## 📉 Cost Comparison (Simulated)

![Cost Comparison](./assets/cost.png)

```text
Task: "Add tests + fix lint"

Before (Codex only):
  Cost: $0.52

After (CodexSaver):
  Cost: $0.18

Saved: 65%
```

---

## ⚡ One-line Install

```bash
curl -sSL codexsaver.sh | bash
```

---

## 🧠 Core Idea

> **Don't replace Codex. Shrink it.**

---

## 🏗 Architecture

```text
User
 ↓
Codex (Router / Brain)
 ↓
CodexSaver
 ├─ Task Classifier
 ├─ Risk Scorer
 ├─ Context Packer
 ├─ DeepSeek API Worker
 ├─ Verifier (tests / diff / lint)
 └─ Fallback (Codex takeover)
 ↓
Final Output
```

---

## ⚙️ How It Works

1. Codex receives a task
2. Decide: handle or delegate
3. DeepSeek executes via API
4. CodexSaver verifies
5. Codex finalizes

---

## 🎯 Task Routing Strategy

### Delegated to DeepSeek

- repo scanning
- writing tests
- simple refactors
- lint fixes
- docs
- boilerplate

### Kept in Codex

- architecture decisions
- security-sensitive logic
- migrations
- ambiguous tasks
- final review

---

## 🛡 Risk Control

- protected paths (`auth/`, `payments/`)
- large diff detection
- test failure → fallback
- high-risk → never delegated

> **DeepSeek does the work. Codex makes the decisions.**

---

## 🔌 DeepSeek API Example

```ts
import OpenAI from "openai";

const client = new OpenAI({
  apiKey: process.env.DEEPSEEK_API_KEY,
  baseURL: "https://api.deepseek.com"
});
```

---

## ⚡ Quick Start

```bash
git clone https://github.com/yourname/codexsaver
cd codexsaver

export DEEPSEEK_API_KEY=xxx

./codexsaver "add unit tests for auth service"
```

---

## 🛠 Roadmap

- [x] API delegation
- [x] rule-based router
- [ ] cost-aware routing
- [ ] multi-model support
- [ ] dashboard

---

## ⭐ If this saves you money

Give it a star.

---

# 💸 CodexSaver（中文版）

> **在不降低效果的前提下，降低 40–70% 的 Codex 成本**

---

## ✨ 什么是 CodexSaver？

CodexSaver 是一个**成本感知的 AI 编程调度器**。

它让 Codex 变成"总控大脑"，把低价值任务交给 DeepSeek API，把高价值推理保留给 Codex。

- 🧠 Codex → 推理 / 架构 / 决策
- ⚡ DeepSeek → 执行 / 搜索 / 生成 / 测试

👉 **效果基本不变，成本显著下降**

---

## 🎬 演示（模拟）

![Demo GIF](./assets/demo.gif)

```text
输入：
"给 auth service 添加单元测试"

[Router] 判定为低风险任务
[Router] 分配给 DeepSeek

[DeepSeek] 生成 6 个测试用例
[Verifier] 运行测试...
[Verifier] 测试通过

[Codex] 审查 patch...
[Codex] 审核通过

[节省] 62% Codex 成本
```

---

## 📉 成本对比（模拟）

![Cost](./assets/cost.png)

```text
任务："写测试 + 修 lint"

Codex 单独执行：
  成本：$0.52

CodexSaver：
  成本：$0.18

节省：65%
```

---

## ⚡ 一键安装

```bash
curl -sSL codexsaver.sh | bash
```

---

## 🧠 核心思想

> **不是替代 Codex，而是减少它的使用成本**

---

## 🏗 架构

```text
用户
 ↓
Codex（大脑 / Router）
 ↓
CodexSaver
 ├─ 任务分类
 ├─ 风险判断
 ├─ 上下文裁剪
 ├─ DeepSeek API 执行
 ├─ 结果验证
 └─ 回退机制
 ↓
最终结果
```

---

## ⚙️ 工作流程

1. Codex 接收任务
2. 判断是否外包
3. DeepSeek 执行
4. 系统验证结果
5. Codex 最终确认

---

## 🎯 任务分配

### 交给 DeepSeek

- 搜索代码
- 写测试
- 简单重构
- 修 lint
- 写文档

### Codex 负责

- 架构设计
- 安全逻辑
- 数据迁移
- 模糊需求
- 最终验收

---

## 🛡 风险控制

- 敏感目录保护
- 大改动检测
- 测试失败回退
- 高风险任务不外包

> **DeepSeek 负责干活，Codex 负责决策**

---

## ⚡ 快速开始

```bash
git clone https://github.com/yourname/codexsaver
cd codexsaver

export DEEPSEEK_API_KEY=xxx

./codexsaver "给 auth service 写单元测试"
```

---

## 🛠 Roadmap

- [x] API 调度
- [x] 规则路由
- [ ] 成本感知调度
- [ ] 多模型支持
- [ ] 可视化面板

---

## ⭐ 如果它帮你省钱了

点个 Star ⭐ 就行
