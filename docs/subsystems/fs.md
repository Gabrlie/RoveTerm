# fs

`rove-fs` 定义 `FileSystem` seam，以及在任意两个文件系统之间复制文件的传输队列。契约和配置见 [crate README](../../crates/rove-fs/README.md)。

## 角色

| 角色 | 实现 |
|---|---|
| Definition | `FileSystem` trait、`DirEntry` / `Metadata` / `FsError` |
| Provider | `LocalFs`（rove-fs）、`SftpFs`（[rove-ssh](ssh.md#sftp)） |
| Consumer | `TransferQueue`、`src-tauri` 的 `sftp_*` 命令、之后的 ACP `fs/*` 和 MCP 文件工具 |

## 服务

`FsPlugin`（插件名 `fs`）注册三项服务：

- `FsSessions`：用 `FsId` 寻址的已打开文件系统。IPC 只传 `FsId`，Rust 侧持有 Provider。
- `TransferQueue`：传输队列，进度通过 Live 事件 `TransferUpdate` 广播。
- `LocalFs`：本机文件系统，作为上传的源和下载的目标。

## 传输

```
enqueue(TransferRequest) → TransferId，状态 queued
  └─ 取得并发许可 → running
       ├─ 决定目标路径（按 Conflict 处理顶层重名）
       ├─ 扫描源目录 → 目录与文件步骤、总字节数
       └─ 逐个复制，每块检查取消 → done / failed / cancelled
```

传输持有源和目标的 `Arc<dyn FileSystem>`，所以关闭 SFTP 面板不会中断正在进行的传输；SFTP 会话在最后一个持有者释放后才关闭它的 channel。

## 路径

远程路径一律按 POSIX 处理（`posix_join`、`posix_parent`）。本地路径由 `LocalFs` 按平台语法拼接。从 IPC 传入的单个文件名先经 `validate_name` 校验，拒绝空名、`.`、`..` 和包含分隔符的名称。
