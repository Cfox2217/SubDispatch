# SubDispatch

[中文](README.md) | [English](README_en.md) | [日本語](README_ja.md) | [한국어](README_ko.md) | [Français](README_fr.md)

SubDispatch 是一个本地脚手架，用于让主 LLM 并行运行子编码代理。主 LLM 负责规划、审查、合并决策和冲突解决。SubDispatch 仅提供隔离执行、状态轮询、产物收集和工作树清理。
它以 Rust 单二进制形式提供 CLI、MCP stdio、worker 调度、git worktree 管理、
Claude hook 记录和本地 Setup/Activity UI。

## 为什么有五国语言

因为这个项目的核心卖点就是“把活分出去”。如果 README 还只有一种语言，
那就像做了一个并行代理调度器，最后让一个代理坐在角落里手搓说明书。
现在它至少会用中文默认开门，同时用英语、日语、韩语和法语假装自己很国际化。

运行时依赖刻意保持很小：

- `git`
- 用户配置的外部 code agent CLI，默认 `claude`
- 工作区 `.env` 中的模型 API 配置

运行时不需要 Python 或 Node。

## 非目标

- 自动任务规划
- 自动审查
- 自动合并或拣选
- 冲突解决
- 多 provider 抽象

## 核心模型

SubDispatch 追踪两个实体：

- `Worker`：已配置的外部编码代理命令。默认使用 `claude-code`。
- `Task`：在独立分支和 git 工作树中执行的子代理。

每个任务记录其基础提交、分支、工作树路径、进程 ID、日志、结果清单路径和产物目录。

## 配置

SubDispatch 从工作区根目录的 `.env` 读取项目本地配置。`.env` 被 git 忽略。
`.env.example` 记录了支持的键。

使用 Rust CLI 创建本地文件：

```bash
subdispatch init-env
```

然后直接编辑 `.env`。SubDispatch 支持默认的 `claude-code` worker：

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

提示词配置单独存放在 `.subdispatch/prompts.json`。该文件是可选的；
不存在时使用内置默认值。Web UI 的 Prompts 页面可以编辑：

- MCP 工具描述
- 子代理提示词模板、安全规则和 manifest schema
- worker 选择策略和 collect/review 指导

Worker metadata 只在 Setup/.env 中配置，避免双重事实源。`description`、
`strengths`、`cost`、`speed` 和 `delegation_trust` 都以 `.env` 为准。
`delegation_trust` 是给主代理看的调度倾向，不是安全保证。

提示词改动会作用于新的 MCP tool listing 和新启动的子任务；已有 task 不会被重写。

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
- 委派可信度
- 不可用原因（如有）

MCP 工具名是 `list_workers`；CLI 对应命令是
`subdispatch workers --workspace <path>`。

### `start_task`

启动一个主 LLM 提供的 child task。SubDispatch 为该任务创建分支和工作树、
写入任务提示词，并在容量可用时启动已配置的 worker。超过 worker 并发限制时，该任务保持排队。

委托前必须有一个干净的已提交 checkpoint。主代理自己决定使用哪个分支或工作树，
并在调用 `start_task` 前提交当前改动。SubDispatch 不管理隐藏的集成分支。
如果工作区存在未提交改动，`start_task` 会直接返回错误，不创建子工作树。
未传 `base`/`base_branch` 时，任务默认从当前 `HEAD` 启动；显式传入 `base`
仍可覆盖这个默认行为。

并行是显式行为：主代理多次调用 `start_task`，根据 available slots 和任务适配度选择
worker，然后分别 poll、collect、review，并自行决定如何合并。

任务可以包含可选的 `context` 或 `context_files`，由主代理显式提供。
这是把未提交 diff、临时审计说明或其他不在子 worktree 基础提交中的信息交给子代理的正确方式。

`read_scope`/`write_scope` 不能和 `forbidden_paths` 重叠。SubDispatch 会在创建
task worktree 前拒绝这种自相矛盾的 scope contract。子任务唯一应该写入的内部
`.subdispatch` 文件是受管理的 result manifest 路径。

### `poll_tasks`

返回 task 的事实状态，可通过 `task_ids`、`status` 或 `active_only` 过滤。
轮询会刷新进程状态，并在 worker 槽位打开时启动排队任务。

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
- Claude transcript 中压缩后的验证命令结果
- task-scoped hook 观察到的禁止路径尝试
- 变更文件
- 不一致
- 补丁路径
- 基础提交
- 任务分支
- 写作用域检查
- 禁止路径检查

worker 清单只是子代理自述。Git diff、scope checks、
`transcript_tool_results_tail` 和 `forbidden_path_attempts_tail` 是更强的审查证据。

### `delete_worktree`

删除一个 SubDispatch 管理的任务工作树。除非强制执行，否则拒绝删除运行中的任务。
默认保留分支和产物目录。

## 硬约束

- 子代理永不运行在主工作树中。
- 每个任务有独立的分支。
- 每个任务有独立的工作树。
- 每个任务记录基础提交。
- `start_task` 拒绝脏主工作区。
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

Web UI 不是任务创建控制台，只做 Setup 检查、`.env` 初始化、worker 容量、task
状态、变更文件数量和 Claude hook 活动展示。主 LLM 仍然通过 MCP 或 CLI 创建任务。

## 安装与发布

全局安装 MCP 入口和内置 skill 一次：

```bash
subdispatch install-skill
subdispatch install --global
```

然后在每个项目中初始化：

```bash
cd /path/to/project
subdispatch init-env --workspace .
subdispatch doctor --workspace .
```

创建本地发布包：

```bash
scripts/release.sh
```

发布细节见 [docs/rust-release.md](docs/rust-release.md)，Python MVP 移除记录见
[docs/python-removal-plan.md](docs/python-removal-plan.md)。
