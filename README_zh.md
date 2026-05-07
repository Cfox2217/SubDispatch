# 💸 CodexSaver

> 在不降低效果的前提下，降低 40–70% 的 Codex 成本

![CodexSaver](./CodexSaver.png)

---

## CodexSaver 是什么？

CodexSaver 让 Codex 作为"总控大脑"，将低价值任务交给 DeepSeek API，高价值推理保留给 Codex。

- 🧠 Codex → 推理 / 架构 / 决策 / 验收
- ⚡ DeepSeek → 执行 / 搜索 / 生成 / 测试

---

## 演示

```
输入：
"给 user service 添加单元测试"

[Codex] 判定为低风险测试生成任务
[Codex] 调用 codexsaver.delegate_task

[CodexSaver] route=deepseek
[Router] task_type=write_tests risk=low
[DeepSeek] 生成 patch
[Verifier] 结构验证通过
[节省] 预计节省 Codex 成本：62%

[Codex] 审查 patch
[Codex] 审核通过
```

---

## 成本对比

```
任务："写测试 + 修 lint"

Codex 单独执行：
  成本：$0.52

CodexSaver：
  成本：$0.18

节省：65%
```

当前代码中的估算区间：

| 被委派上下文大小 | 预计节省 |
|---|---:|
| `< 8k` 字符 | `45%` |
| `8k–50k` 字符 | `62%` |
| `> 50k` 字符 | `70%` |

这些百分比目前来自内置 `CostEstimator` 的启发式估算，不是 Codex 或 DeepSeek 的
真实结算账单。它适合做路由对比和 README 演示，但还不适合作为财务口径的数据。

---

## 架构

```
用户
  ↓
Codex
  ↓ MCP 工具调用
CodexSaver
  ├─ Router（路由决策）
  ├─ Context Packer（上下文裁剪）
  ├─ DeepSeek API Worker
  ├─ Verifier（验证）
  └─ Cost Estimator（成本估算）
  ↓
Codex 审查 / 应用 / 最终确认
```

---

## 安装

```bash
git clone https://github.com/yourname/codexsaver
cd codexsaver

export DEEPSEEK_API_KEY=xxx
```

---

## Codex 集成

项目配置（`.codex/config.toml`）：

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["./codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

全局 Codex 配置（`~/.codex/config.toml`）也可以直接使用：

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["./codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

如果你想在仓库目录之外使用它，可以把 `args` 改成你自己机器上的克隆路径：

```toml
[mcp_servers.codexsaver]
command = "python"
args = ["/absolute/path/to/codexsaver/codexsaver_mcp.py"]
startup_timeout_sec = 10
tool_timeout_sec = 120
```

已在 2026 年 5 月 7 日验证：

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"codexsaver","version":"0.2.0"}}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"delegate_task"}]}}
```

然后告诉 Codex：

```
对低风险任务使用 CodexSaver。
给 user service 添加单元测试。
```

---

## CLI 测试

试运行（不调用 API）：

```bash
python cli.py "添加单元测试" --files src/user/service.ts --workspace . --dry-run
```

真实调用：

```bash
python cli.py "添加单元测试" --files src/user/service.ts --workspace .
```

CodexSaver 会基于 `workspace` 解析相对路径，并在验证阶段执行 worker 返回的
`commands_to_run`。只要验证命令失败，就会回退为 `needs_codex`，交还给 Codex 处理。

---

## 已验证的路由对比

低风险任务（2026 年 5 月 7 日）：

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

高风险任务（2026 年 5 月 7 日）：

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

这就是它应该做到的分工：

- 低风险执行任务交给 DeepSeek。
- 高风险、安全敏感任务保留给 Codex。

### 量化路由样例

以下样例来自 2026 年 5 月 7 日执行的 `python cli.py ... --dry-run`：

| 任务 | 任务类型 | 风险 | 路由 | 预计节省 |
|---|---|---|---|---:|
| `Add unit tests for user service` | `write_tests` | `low` | `deepseek` | `45%` |
| `Explain the routing logic` | `explain` | `low` | `deepseek` | `45%` |
| `Update README usage docs` | `docs` | `low` | `deepseek` | `45%` |
| `Explain auth code` | `explain` | `medium` | `deepseek` | `45%` |
| `Add tests across six files` | `write_tests` | `medium` | `deepseek` | `45%` |
| `Refactor auth service` | `simple_refactor` | `high` | `codex` | `0%` |
| `Fix security vulnerability in auth flow` | `unknown` | `high` | `codex` | `0%` |
| `Design new authentication architecture` | `unknown` | `high` | `codex` | `0%` |

在这组样例里：

- `8` 个任务里有 `5` 个被委派。
- `8` 个任务里有 `3` 个保留给 Codex。
- 所有 `high` 风险任务都没有被下放。
- `medium` 风险但偏只读的任务仍然可以委派。

### 真实 API 调用报告

以下结果来自 2026 年 5 月 7 日的真实 DeepSeek 联网调用：

| 场景 | 任务 | 路由 | 状态 | 延迟 | 修改文件数 | Patch 大小 | 返回体大小 | 预计节省 |
|---|---|---|---|---:|---:|---:|---:|---:|
| 只读分析 | `Explain the routing logic and summarize protected path handling` | `deepseek` | `success` | `1.55s` | `0` | `0 chars` | `778 chars` | `45%` |
| 小型文档修改 | `Add concise module-level documentation to router.py without changing behavior` | `deepseek` | `success` | `3.22s` | `1` | `277 chars` | `1108 chars` | `45%` |

从这两次真实调用里，可以直接看到：

- 一个只读型委派任务可以在 `1.55s` 内完成。
- 一个会产出 patch 的小型委派任务可以在 `3.22s` 内完成。
- 产出 patch 的调用返回了一个只有 `277` 字符的紧凑 diff，只修改 `1` 个文件。
- 两次调用都通过了验证，但都没有附带后续验证命令。

这也让 README 里的证据分层更清楚：

- `dry_run` 用来展示路由策略。
- 真实 API 调用用来展示委派执行真的能跑通。
- 受保护 / 高风险任务则可以通过本地路由结果展示，无需额外发起不必要的外网请求。

### 路由逻辑分析

CodexSaver 的核心问题不是“这是不是编码任务”，而是：
“这是不是一个足够便宜、又不会损失判断质量的编码任务？”

当前逻辑分四层：

1. **任务分类**
   `write_tests`、`docs`、`code_search`、`explain`、`fix_lint`、`boilerplate`、
   `simple_refactor` 这些低风险类型，才有资格被委派。

2. **指令级风险扫描**
   像 `security`、`authentication`、`billing`、`migration`、`deploy`、`encrypt`、
   `token` 这类关键词会直接抬高风险，因为这类任务通常不只是“代码写对”这么简单。

3. **路径 / 领域风险扫描**
   如果文件路径包含 `auth`、`payments`、`billing`、`infra`、`migrations`、
   `.github/workflows` 或密钥相关词汇，就会被视为受保护区域。即便任务描述看起来普通，
   也会阻止或限制委派。

4. **只读任务的安全例外**
   某些 `medium` 风险任务如果主要是观察型工作，仍然可以委派，例如 `explain`、
   `docs`、`code_search` 和 `write_tests`。这也是为什么 `Explain auth code`
   还能走 DeepSeek，而 `Refactor auth service` 必须留给 Codex。

这会形成一个刻意的不对称：

- 只读理解型任务可以尽量便宜。
- 对敏感域的写操作，哪怕改动很小，风险也会迅速上升。
- 一旦任务模糊不清，默认交回 Codex，而不是默认下放。

---

## 任务分配

### 交给 DeepSeek

- 搜索代码
- 解释代码
- 写单元测试
- 修 lint/type error
- 写文档
- 生成模板代码
- 小范围重构

### Codex 负责

- 架构设计
- 安全逻辑
- 支付/账单
- 权限/认证
- 数据库迁移
- 生产部署
- 模糊需求
- 最终验收

---

## Roadmap

- [x] MCP 服务器（`codexsaver.delegate_task`）
- [x] 规则路由
- [x] DeepSeek API 集成
- [x] 上下文裁剪
- [ ] 成本感知调度
- [ ] 多模型支持

---

## 如果它帮你省钱了

点个 Star ⭐
