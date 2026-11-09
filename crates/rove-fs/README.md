# rove-fs

文件系统 seam。定义 `FileSystem` trait，提供本地实现 `LocalFs`、打开中的文件系统表 `FsSessions` 和传输队列 `TransferQueue`。SFTP 实现在 `rove-ssh`。参考见 [docs/subsystems/fs.md](../../docs/subsystems/fs.md)。

## 契约

- 路径是 Provider 自己语法下的绝对路径字符串；拼接和取父目录只用 `FileSystem::join` / `parent`。远程路径按 POSIX 处理，本地路径按平台处理。
- `list` 不返回 `.` 和 `..`，不跟随符号链接；`stat` 跟随符号链接，`lstat` 不跟随。
- `remove_all` 递归删除目录；符号链接（包括指向目录的链接）只删除链接本身，从不进入其目标。
- `open_write` 创建或截断文件；调用方必须 `shutdown` 写入流，才算写完并关闭文件。
- `FsSessions` 只持有引用：移除一个条目后，正在运行的传输仍然持有它，传输结束后 Provider 才关闭。
- `TransferQueue` 同时运行的传输数不超过 `max_concurrent`，其余为 `queued`。每个传输把一个文件或目录复制到目标目录下：
  - 先扫描源目录，得到总字节数和总文件数，再按父目录在前的顺序创建目录、复制文件。
  - 源目录列出的名称必须是单个路径组成部分，且拼接后仍在目标目录之下，否则传输失败（防止恶意服务器返回 `../` 名称越界写入）。
  - 指向文件的符号链接按文件复制；指向目录的符号链接和特殊文件跳过，避免循环。
  - 每个文件先写入同目录下的临时文件 `.名称.传输id.part`，完整写完后再改名覆盖目标；失败或取消时只删除临时文件，原有目标文件不受影响，已完成的文件保留。
  - 顶层名称冲突按 `Conflict` 处理：`overwrite` 覆盖文件、合并目录；`rename` 改名为 `name (1).ext`；`skip` 不复制并标记 `skipped`。
- 每次状态变化都发出 Live 事件 `TransferUpdate`；进度事件同一传输最多每 `progress_interval_ms` 一次。

## 配置（`[plugin.fs]`）

| 键 | 默认值 | 范围 | 含义 |
|---|---|---|---|
| `max_concurrent` | 3 | 1–16 | 同时运行的传输数 |
| `buffer_kib` | 256 | 16–4096 | 每个传输的复制缓冲区 |
| `progress_interval_ms` | 200 | 50–5000 | 同一传输两次进度事件的最小间隔 |

## 局限

- 不支持断点续传，也不保留修改时间和权限。
- 取消正在运行的传输时，只删除当前文件的临时文件，已复制的文件不回滚。
- SFTP 版本 3 的 rename 不能覆盖已有文件，此时先删除目标再改名，两步之间有极短的窗口。
