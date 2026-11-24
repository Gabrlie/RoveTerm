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

移动端上传先把系统文件选择器提供的文件复制到 `<app_cache_dir>/rove-staged`，再将暂存路径交给 `TransferQueue`。最终 `TransferUpdate` 触发暂存清理，入队前取消可显式丢弃暂存文件。应用启动和退出时清理暂存目录。

## 路径

远程路径一律按 POSIX 处理（`posix_join`、`posix_parent`）。本地路径由 `LocalFs` 按平台语法拼接。从 IPC 传入的单个文件名先经 `validate_name` 校验，拒绝空名、`.`、`..` 和包含分隔符的名称。传输时源目录列出的每个名称同样校验，并确认拼接后的路径恰好位于目标目录之下（例如拒绝 Windows 的 `C:x`），不符合时整个传输失败，不会写到目标目录之外。

## 应用命令

`src-tauri/src/commands/sftp.rs` 把 seam 暴露给 WebView。所有命令只接受 `FsId` 和路径字符串，Provider 留在 Rust 侧。

| 命令 | 作用 |
|---|---|
| `sftp_open(host_id \| address, password, remember)` | 获取连接的方式和打开终端相同，返回 `{ id, label, home }` |
| `transfer_local_clashes` | 检查待下载的名称是否已在目标目录中存在，界面据此询问替换、保留两者或跳过 |
| `sftp_list` / `sftp_mkdir` / `sftp_rename` / `sftp_remove` | 浏览和修改远程目录；`sftp_remove` 递归删除，拒绝删除根目录 |
| `sftp_close` | 从 `FsSessions` 移除；进行中的传输不受影响 |
| `transfer_prepare_upload` | 暂存移动端选择的文件，返回可用于上传的本地路径 |
| `transfer_discard_staged` | 丢弃尚未入队的暂存文件 |
| `transfer_upload` / `transfer_download` | 为每个路径排一个传输 |
| `transfer_list` / `transfer_cancel` / `transfer_clear` | 查询、取消、清除已结束的传输 |
| `transfer_reveal(id)` | 显示已完成的下载。移动端打开系统分享面板。macOS 和 Windows 在文件管理器中选中文件，Linux 打开所在目录 |
| `transfer_export(id)` | 在移动端通过系统文件选择器将已完成的下载保存到设备，保存成功返回 `true`，取消返回 `false` |

下载与重名检查共用默认目录。桌面端使用系统下载目录，系统未配置时退回已存在的 `~/Downloads`。移动端在系统下载目录不可用时使用应用专用下载目录。分享和导出按传输 ID 取得下载路径。

`TransferUpdate` 以 `transfer-update` 事件转发给 WebView。远程路径必须是绝对 POSIX 路径，本地路径必须是绝对路径，单次最多 1000 个路径；新文件名经 `validate_name` 校验。

## 文本编辑

`TextFile` 携带原始文本与内容版本；`read_text_file` / `save_text_file` 通过 `FileSystem` 完成快照读取和版本校验替换，读取限制与暂存写入规则见 [crate 契约](../../crates/rove-fs/README.md#契约)。

`sftp_read_text` / `sftp_save_text` 将 [文本快照与版本校验写入](../../crates/rove-fs/README.md#契约) 暴露给内置编辑器；`sftp_retain` 为草稿取得同一 Provider 的独立句柄，关闭文件面板不影响草稿。

桌面端 `external_edit_start` 创建本机副本并启动编辑器；每 500 ms 检查一次，内容变更稳定至少 1 秒才回传。同一会话的写回、重试和结束串行执行，状态以 `external-edit-update` 发出。冲突或失败暂停自动回传；重新打开原 SSH 目标（包括路线）的文件面板后可重试。选择覆盖时，保存只对确认前读取的远端版本有效。退出保留副本和版本，下次启动显示为待恢复，不会自动上传。
