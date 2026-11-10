# rove-session

定义 `TerminalSession` seam，并提供 `SessionManager`。参考见 [docs/subsystems/session.md](../../docs/subsystems/session.md)。

## 契约

- Provider 打开终端后产出 `OpenedTerminal`：控制句柄 `Arc<dyn TerminalSession>`、输出 channel 和元数据。输出 channel 在发送 `TerminalOutput::Exit` 后或出错时关闭。
- `SessionManager::attach` 必须在 Tokio 运行时内调用；它为每个会话启动一个输出泵任务，任务在输出 channel 关闭时结束。
- `OutputSink::send` 不能阻塞；返回 `false` 表示接收方已经消失，此后不再转发，但仍继续记录回放。
- `reattach` 在同一把锁内取回放快照并替换 sink，新 sink 收到的恰好是快照之后的输出。
- `snapshot` 只读取回放缓冲，不影响当前 sink（供 AI 读取终端内容）。
- 会话结束后仍保留在表中，直到调用 `close`；`TerminalSession::close` 是幂等的。

## 配置（`[plugin.session]`）

| 键 | 默认值 | 范围 | 含义 |
|---|---|---|---|
| `coalesce_ms` | 6 | 1–50 | 输出最多缓存多久再转发 |
| `max_chunk_bytes` | 65536 | 4096–1048576 | 缓存达到该字节数时立即转发 |
| `replay_bytes` | 262144 | 0–16777216 | 每个会话保留的回放字节数 |

## 局限

回放缓冲按字节截断，开头可能切断一个 UTF-8 字符或转义序列；终端模拟器能容忍开头的这一处损坏。
