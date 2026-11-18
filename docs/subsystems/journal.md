# journal

`rove-journal` 定义 `JournalStore` seam，并提供基于 SQLite 的事件日志。Journal 事件是重启后仍然存在的事实，用于会话恢复和审计；记录只追加，唯一的例外是用户删除某个 AI 对话时移除它的记录。

## JournalStore seam

```rust
pub trait JournalStore: Send + Sync {
    fn append(&self, record: NewRecord) -> Result<i64, JournalError>;
    /// 追加一条发生在 `ts`（Unix 毫秒）的记录，如从其他设备同步来的记录。
    fn import(&self, ts: i64, record: NewRecord) -> Result<i64, JournalError>;
    fn read(&self, query: &JournalQuery) -> Result<Vec<JournalRecord>, JournalError>;
    /// 用户删除时移除会话 `session_id` 中 kind 以 `kind_prefix` 开头的记录，返回移除条数。
    fn delete_session(&self, session_id: &str, kind_prefix: &str) -> Result<usize, JournalError>;
}
```

| 字段 | 说明 |
|---|---|
| `seq` | 单调递增的序号 |
| `ts` | 追加时间，Unix 毫秒 |
| `kind` | `领域/名称` 形式，例如 `session/opened` |
| `session_id` | 所属会话，可为空 |
| `payload` | JSON 负载 |

`JournalQuery` 可以按 `after_seq`、`session_id`、`kind` 过滤，并用 `limit` 限制条数（默认 1000）。结果默认按 `seq` 升序；`newest_first` 改为降序，配合 `limit: Some(1)` 读取某类事件的最新一条。

## 记录的事件

| kind | 来源 Live 事件 | payload |
|---|---|---|
| `session/opened` | `SessionOpened` | `{ "info": SessionInfo }` |
| `session/exited` | `SessionExited` | `{ "status": ExitStatus }` |

新增需要持久化的事实时，定义新的 `kind`，并在 `Recorder` 或所属插件中追加记录。

SQLite 结构版本 2 在保留既有事件的同时增加本机审计正文表、同步回执表和定时任务到期时间唯一索引。

## 写入路径

`Recorder` 订阅 Live 事件，把记录放进队列，由 `journal-writer` 线程依次追加。`flush()` 阻塞到此前排队的记录全部写入，删除会话前用它保证仍在队列中的记录也一并删除。`events` 按 `(session_id, seq)` 建索引。本机审计密文与事件引用在同一事务中写入。

## 工作区快照

应用命令 `workspace_save(snapshot)` 把界面的标签与分屏布局作为 `ui/workspace` 记录追加到 journal（必须是对象，序列化后不超过 256 KiB，面板只以主机 id 引用已保存主机，不含秘密）；`workspace_load()` 返回最新一条，没有时返回 `null`。快照的结构由界面定义（[ui](ui.md)）。

## AI 会话的审计记录

`rove-ai` 的 `Auditor` 以 AI 会话 id 作为 `session_id` 写入以下记录，payload 都带 `host`（会话主机）：

| kind | payload |
|---|---|
| `ai/session_started` | Agent 名称、版本、审批模式 |
| `ai/prompt` | 用户发出的文本 |
| `ai/message`、`ai/thought` | 一条完整的回复或思考（流式片段合并后） |
| `ai/approval` | 请求：操作、风险、原因、是否 Agent 自带工具；回答：`answer`（`null` 为回合取消时撤回） |
| `ai/command`、`ai/file_read`、`ai/file_write`、`ai/list_dir`、`ai/terminal_read`、`ai/tool_call` | 操作、风险、是否经用户批准、结果（命令的退出码与输出、文件的字节数与 SHA-256、写入前后的哈希与行数、拒绝或失败原因） |
| `ai/turn_ended`、`ai/session_ended` | 回合的结束原因；会话结束 |

工具事件在 `payload.body_ref` 中引用本机密文。展开详情时使用设备密钥解密，其他设备只收到摘要。读取记录保存实际返回的行，写入记录保存最终文本，命令的两路输出分别保存。经批准读取敏感文件后，后续对话文本也仅在本机加密保存。删除对话一并移除密文。
