# journal

`rove-journal` 定义 `JournalStore` seam，并提供基于 SQLite 的追加式事件日志。Journal 事件是重启后仍然存在的事实，用于会话恢复和审计。

## JournalStore seam

```rust
pub trait JournalStore: Send + Sync {
    fn append(&self, record: NewRecord) -> Result<i64, JournalError>;
    fn read(&self, query: &JournalQuery) -> Result<Vec<JournalRecord>, JournalError>;
}
```

| 字段 | 说明 |
|---|---|
| `seq` | 单调递增的序号 |
| `ts` | 追加时间，Unix 毫秒 |
| `kind` | `领域/名称` 形式，例如 `session/opened` |
| `session_id` | 所属会话，可为空 |
| `payload` | JSON 负载 |

`JournalQuery` 可以按 `after_seq`、`session_id`、`kind` 过滤，并用 `limit` 限制条数（默认 1000）。

## 记录的事件

| kind | 来源 Live 事件 | payload |
|---|---|---|
| `session/opened` | `SessionOpened` | `{ "info": SessionInfo }` |
| `session/exited` | `SessionExited` | `{ "status": ExitStatus }` |

新增需要持久化的事实时，定义新的 `kind`，并在 `Recorder` 或所属插件中追加记录。

## 写入路径

`Recorder` 订阅 Live 事件，把记录放进队列，由 `journal-writer` 线程依次追加。表结构为单表 `events`，并对 `(session_id, seq)` 建了索引。
