# rove-ops

持久运维任务与进程内全局 worker 调度。`TaskStore` 定义元数据存储，`JournalTaskStore` 将事件保存在本机 Journal，`OpsSupervisor` 提供状态迁移与租约；应用的模型执行器和任务界面作为 Consumer。参考见 [ops](../../docs/subsystems/ops.md)。

## 契约

- `OpsRequestId` 是有界、非空的来源标识；同一请求重复创建返回已有任务，改变原请求的主机范围、来源或会话返回冲突。
- 任务保存冻结的 `HostId` 与非秘密目标指纹。正文、报告、验收和检查点只存 `PrivateRef`，调用方须先加密保存其引用的正文。
- `TaskStore` 以 revision 比较并替换；更新不能扩大授权范围或更改来源。Journal 读取分页回放，遇到错误、未来事件版本或不连续 revision 时失败。
- `lease_worker` 在取得全局许可并持久写入 `running` 后才返回；同一子任务不能并行执行，`WorkerLease.cancellation` 用于停止实际 worker。放弃租约将未完成运行记为中断。
- `append_input` 只保存显式新输入；Consumer 使用 `requeue_subtask` 选择要继续的主机。已完成子任务必须收到新输入才能重新排队；会话和检查点引用保持不变。
- `cancel_subtask` 只停止指定主机的 worker，`cancel_conversation` 返回本次取消的任务。已完成的子任务保留结果。
- `retry_failed` 批量重排失败、受阻或中断的子任务，`retry_subtask` 重排指定主机。远端修改仍待核对时，调用方须先通过 `resolve_effect` 记录已持久保存的核对证据。
- `mark_effect_started` 必须在远端修改之前成功记录；只读任务不能登记修改。该记录不替代执行器的逐操作 Policy 审批。
- worker 只有带证据引用且没有未确认修改时才能完成。请求上限为受阻，取消、失败、中断分别保存；所有目标主机完成后任务才完成。
- 启动恢复将 `running` 转为中断，存在未确认修改的转为受阻；失去 worker 的待审批项转为等待用户决定的受阻状态。一次性批准不恢复，恢复项不自动执行。
- 元数据、加密正文或检查点索引写入失败时，`halt_on_storage_failure` 停止全局派发并取消租约令牌。`shutdown` 停止排队、撤销令牌并记录中断；插件卸载撤销服务注册。

## 配置（`[plugin.ops]`）

| 字段 | 默认 | 校验 |
|---|---|---|
| `max_workers` | 4 | 1–64，所有任务共享 |
| `max_subtasks` | 16 | 1–1024，每任务 |
| `max_inputs` | 1024 | 1–16384，每任务 |

每个 Journal 由一个应用内 `OpsSupervisor` 调度。任务事件不进入对话同步投影；复制报告不会复制执行权。模型、主机连接、审批、实际命令和加密正文的生命周期由 Consumer 提供。
