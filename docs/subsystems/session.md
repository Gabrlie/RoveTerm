# session

`rove-session` 定义 `TerminalSession` seam，并提供 `SessionManager`：分配 `SessionId`、合并输出、维护回放缓冲。

## TerminalSession

```rust
#[async_trait]
pub trait TerminalSession: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<(), SessionError>;
    async fn resize(&self, size: TermSize) -> Result<(), SessionError>;
    async fn close(&self) -> Result<(), SessionError>;   // 幂等
}
```

输出不经过这个 trait：Provider 通过 `OpenedTerminal::output`（`mpsc::Receiver<TerminalOutput>`）交付 `Data(Bytes)` 和最终的 `Exit(ExitStatus)`。

## 值类型

| 类型 | 说明 |
|---|---|
| `SessionId` | UUIDv7 newtype；`FromStr` 用于校验 IPC 传入的字符串 |
| `TermSize` | `cols`、`rows` 都必须在 `1..=1000` 范围内，由 `TermSize::new` 校验 |
| `SessionInfo` | `SessionKind`（`Local { shell }` 或 `Ssh { host_id, address }`）加初始标题 |
| `ExitStatus` | 可选的退出码和信号名 |

## SessionManager

| 方法 | 语义 |
|---|---|
| `attach(opened, sink) -> SessionId` | 登记会话、启动输出泵、发出 `SessionOpened` |
| `write` / `resize` | 转发给对应的 `TerminalSession`；未知 ID 返回 `NotFound` |
| `close(id)` | 从表中移除、断开 sink、调用 `TerminalSession::close` |
| `reattach(id, sink) -> (Bytes, Option<ExitStatus>)` | 原子地取回放快照并替换 sink |
| `list()` | 列出会话及其退出状态 |
| `close_all()` | 关闭全部会话，用于退出应用时 |

## 输出合并

输出泵收到第一块数据后开启一个 `coalesce_ms` 的时间窗口，窗口内到达的数据合并成一块；缓存达到 `max_chunk_bytes` 时提前转发。每块数据先写入回放缓冲，再交给 sink。

## Live 事件

| 事件 | 时机 |
|---|---|
| `SessionOpened { id, info }` | `attach` 完成登记后 |
| `SessionExited { id, status }` | 输出 channel 结束后；没有收到 `Exit` 时 `status` 为默认值 |

[rove-journal](journal.md) 监听这两个事件并把它们写成 Journal 事件。
