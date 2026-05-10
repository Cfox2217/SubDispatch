# SubDispatch

SubDispatch 正在从 Python MVP 迁移为 Rust 单二进制本地工具。Rust 二进制是后续
CLI、MCP stdio、worker 调度、git worktree 管理、产物回收、Claude hook 记录和
本地 Setup/Activity UI 的主路径。Python 实现暂时保留在仓库中，作为行为参考，
直到 Rust 路径完成充分验收。

SubDispatch 是一个本地脚手架，用于让主 LLM 并行运行子编码代理。主 LLM 负责规划、审查、合并决策和冲突解决。SubDispatch 仅提供隔离执行、状态轮询、产物收集和工作树清理。

运行时依赖刻意保持很小：

- `git`
- 用户配置的外部 code agent CLI，默认 `claude`
- 工作区 `.env` 中的模型 API 配置

Rust 二进制运行时不需要 Python 或 Node。

## 非目标

- 自动任务规划
- 自动审查
- 自动合并或拣选
- 冲突解决
- 多 provider 抽象

## 核心模型

SubDispatch 追踪三个实体：

- `Worker`：已配置的外 部编码代理命令。MVP 默认使用 `claude-code`。
- `Run`：从同一基础提交启动的一组子任务。
- `Task`：在独立分支和 git 工作树中执行的子代理。

每个任务记录其基础提交、分支、工作树路径、进程 ID、日志、结果清单路径和产物目录。

## 配置

SubDispatch 从工作区根目录的 `.env` 读取项目本地配置。`.env` 被 git 忽略。
`.env.example` 记录了支持的键。

使用 Rust CLI 创建本地文件：

```bash
subdispatch init-env
```

然后直接编辑 `.env`。MVP 支持默认的 `claude-code` worker：

- `SUBDISPATCH_WORKER_MODE`
- `SUBDISPATCH_CLAUDE_ENABLED`
- `SUBDISPATCH_CLAUDE_PERMISSION_MODE`
- `SUBDISPATCH_CLAUDE_COMMAND`
- `SUBDISPATCH_CLAUDE_MODEL`
- `SUBDISPATCH_CLAUDE_MAX_CONCURRENCY`
- `ANTHROPIC_API_KEY`
- `ANTHROPIC_BASE_URL`

默认 worker 模式为 `trusted-worktree`，配合 Claude Code `bypassPermissions`。
这是有意为之，用于主代理将执行所有权转移给子代理的委托编码循环。
这不是安全沙箱。SubDispatch 依赖隔离的 git 工作树、明确的任务范围、
日志和运行后产物审查，而非执行前隔离。

## 接口

### `list_workers`

返回可用 worker 及当前容量：

- worker id
- runner 命令
- 配置的模型
- 最大并发数
- 运行中数量
- 排队数量
- 可用槽位
- 不可用原因（如有）

### `start_run`

从主 LLM 提供的任务列表启动一个 run。对每个任务，SubDispatch 创建分支和工作树、
写入任务提示词，并在容量可用时启动已配置的 worker。超过 worker 并发限制的任务保持排队。

任务可以包含可选的 `context` 或 `context_files`，由主代理显式提供。
这是把未提交 diff、临时审计说明或其他不在子 worktree 基础提交中的信息交给子代理的正确方式。

### `poll_run`

返回任务的事实状态。轮询刷新进程状态并在 worker 槽位打开时启动排队的任务。

任务状态：

- `queued`
- `running`
- `completed`
- `failed`
- `cancelled`
- `missing`

### `collect_task`

收集一个任务产物。SubDispatch 从 Git 计算变更文件和不一致，
而非信任 worker 清单。它包含未提交的工作树变更，因为子代理无需提交。

返回的产物包括：

- 原始指令
- worker 清单（如有）
- stdout/stderr 尾部
- 变更文件
- 不一致
- 补丁路径
- 基础提交
- 任务分支
- 写作用域检查
- 禁止路径检查

### `delete_worktree`

删除一个 SubDispatch 管理的任务工作树。除非强制执行，否则拒绝删除运行中的任务。
默认保留分支和产物目录。

## 硬约束

- 子代理永不运行在主工作树中。
- 每个任务有独立的分支。
- 每个任务有独立的工作树。
- 每个任务记录基础提交。
- `collect_task` 使用 Git 作为事实来源。
- 工作树删除验证目标位于 SubDispatch 工作树根目录下。
- 产物默认保留。
- Worker 并发限制被执行。

## Rust CLI

本地开发时：

```bash
cargo run -- workers --workspace .
cargo run -- mcp --workspace .
cargo run -- serve --workspace . --bind 127.0.0.1:8765
```

打包后二进制使用方式：

```bash
subdispatch workers --workspace .
subdispatch mcp --workspace .
subdispatch serve --workspace . --bind 127.0.0.1:8765
```

Web UI 不是任务创建控制台，只做 Setup 检查、`.env` 初始化、worker 容量、run/task
状态、变更文件数量和 Claude hook 活动展示。主 LLM 仍然通过 MCP 或 CLI 创建任务。
