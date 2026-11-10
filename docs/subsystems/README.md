# 子系统参考

每个子系统一页，描述类型、语义和不变量。行为叙述和整体数据流见 [architecture.md](../architecture.md)。

| 页面 | 范围 |
|---|---|
| [kernel](kernel.md) | `Context`、服务注册、事件总线、`Disposer`、`Plugin`、Profile |
| [session](session.md) | `TerminalSession` seam、`SessionManager`、输出合并与回放 |
| [pty](pty.md) | 本地 PTY 会话 |
| [ssh](ssh.md) | SSH 认证、host key、连接池、ssh_config 与快速连接、SFTP |
| [fs](fs.md) | `FileSystem` seam、本地文件系统、传输队列 |
| [vault](vault.md) | 主机与凭据存储、设备主密钥、解锁 |
| [journal](journal.md) | 持久事件日志 |
| [sync](sync.md) | 端到端加密的多设备同步：空间、分段、HLC 合并 |
| [ai](ai.md) | AI Agent：风险分级、审批策略 |
| [ui](ui.md) | 前端结构、actions 表、i18n、设计 token |
