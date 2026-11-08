# rove-journal

持久的追加式事件日志。定义 `JournalStore` seam，提供 SQLite 实现，并把会话生命周期的 Live 事件记录为 Journal 事件。参考见 [docs/subsystems/journal.md](../../docs/subsystems/journal.md)。

## 契约

- `append` 返回单调递增的 `seq`；`read` 按 `seq` 升序返回结果，默认最多 1000 条。
- `Recorder` 在专用写线程上追加记录，事件监听器不会等待磁盘 I/O。写线程在全部 `Recorder` 克隆和订阅都被 drop、队列清空后结束。
- 追加失败时记录错误日志，不会让发出事件的一方失败。
- 结构版本记录在 `PRAGMA user_version`，只增不减；打开更新版本写入的数据库时返回 `FutureSchema`。

## 配置（`[plugin.journal]`）

| 键 | 默认值 | 含义 |
|---|---|---|
| `database` | 必填 | 数据库路径，由应用根据数据目录设置 |

## 局限

尚未实现记录保留期限和清理。
