---
name: subdispatch-delegation
description: 指导主 LLM 如何使用 SubDispatch 进行委派、并行任务拆分、worker 选择、证据审查和 worktree 清理。用户或系统一旦提到 SubDispatch、MCP 工具、child task、start_task、poll_tasks、collect_task、delete_worktree、worktree、branch、并行代理、任务拆分、worker 选择、证据回收或清理流程，就应优先使用这个 skill 来规范 SubDispatch 的操作方式。
---

# SubDispatch 委派规范

这是 SubDispatch 的操作手册。它讲的是怎么用，不讲系统必须在什么时候强制委派。

## 目标

优先优化：

- 降低主代理自己的 token 和时间消耗
- 提高并行吞吐
- 保证最终结果正确可靠

子代理的 token 成本是次要因素。不要为了追求并行，就牺牲证据、边界和正确性。

## 基本判断

当任务可以拆成独立、边界清晰的子工作，并且子代理完成后不会阻塞主代理的下一步关键路径时，适合委派。

保持本地处理的情况包括：

- 跨很多文件或层级紧耦合
- 风险高，错了会比省下的时间损失更大
- 依赖还没确定的设计决策
- 太小，协调成本比收益更高

如果任务本身还不清楚，先把问题说清楚，再考虑委派。不要把模糊问题丢给子代理碰运气。

## 是否委派

下面大多数条件满足时，才考虑委派：

1. 任务能拆成一个或多个独立子任务。
2. 每个子任务的 `write_scope` 很窄。
3. 结果可以通过 diff、日志、测试或 manifest 验证。
4. 主代理可以审查结果，而不是重做一遍。
5. 并行执行能带来真实的时间收益。

不要因为“子代理多”就委派。

## 并行拆分

并行时，宁可拆成几个小任务，也不要塞成一个大任务。

适合的拆法：

- 不同文件
- 不同模块
- 不同测试片段
- 不同 UI 区块
- 不同文档段落

不适合的拆法：

- 两个代理同时改同一组文件
- 一个任务里混着重构、功能、文档
- `write_scope` 互相重叠的并行任务

## 选择 worker

根据成本、速度和任务匹配度选 worker：

- 简单编辑、文档、搜索、机械改动，用快且便宜的 worker
- 逻辑不清、跨文件、推理重的实现，用更强的 worker
- 很多小任务并行时，用高并发 worker

优先选择“足够便宜但仍能正确完成”的 worker。

## 任务写法

调用 `start_task` 时，任务要写清楚：

- `instruction`：子代理要做的具体改动
- `goal`：更高层目标，必要时再写
- `worker`：选择的 worker id
- `base` / `base_branch`：从哪个 checkpoint 分支出去
- `read_scope`：子代理需要查看的文件
- `write_scope`：子代理允许修改的文件
- `forbidden_paths`：绝不能碰的路径
- `context`：只有子代理无法自己推断时才补充
- `context_files`：只有 worktree 里看不到时才传

`write_scope` 越小越好。不要过度喂上下文。

## 执行流程

推荐按这个顺序做：

1. 确认主工作区是干净且已提交的。
2. 每个独立切片启动一个子任务。
3. 用 `poll_tasks` 全局轮询状态。
4. 用 `collect_task` 收集证据。
5. 审查 diff、scope check、日志和 manifest。
6. 决定保留哪些改动。
7. 任务不再需要时删除 worktree。

## 审查顺序

不要只信子代理的 manifest。

优先级如下：

1. Git diff
2. scope check 和 forbidden-path check
3. 日志和 hook summary
4. worker manifest

如果结果不够明确，就继续审查，或者缩小任务后重跑。

## 清理规则

只有在主代理已经不再需要这个 worktree 时才删除。

如果任务已经完成，但还可能要回看细节，就先留着，等 review 结束再清。

## 提示方式

给主代理传达的原则应该是：

- 用 SubDispatch 节省主代理自己的 token 和时间
- 正确性优先，不是单纯追求速度
- 只有边界清晰时才拆任务
- 并行任务必须互不干扰
- 任何结果进入主分支前都要先验证
