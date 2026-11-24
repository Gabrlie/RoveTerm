# pty

`rove-pty` 提供 `LocalTerminals` 服务，在本机 PTY 中启动 shell 并产出 `TerminalSession`。

## 请求解析

`LocalRequest { shell, cwd, size }` 中每个字段都可省略，由 `resolve(&PtyConfig) -> LaunchSpec` 一次性确定：

| 字段 | 取值顺序 |
|---|---|
| 程序 | 请求的 `shell` → 配置的 `shell`（带配置的 `args`）→ 平台默认 |
| 参数 | macOS 上的 POSIX shell 加 `-l`（登录 shell），其他情况为空 |
| 工作目录 | 请求的 `cwd` → 用户主目录；目录不存在时继承父进程的目录 |
| 尺寸 | 请求的 `size` → 80x24 |

平台默认 shell：Unix 用 `$SHELL`，没有时用 `/bin/sh`；Windows 依次查找 `PATH` 中的 `pwsh.exe`、`powershell.exe`，都没有时用 `%COMSPEC%`，最后用 `cmd.exe`。

shell 的环境变量额外设置 `TERM=xterm-256color`、`COLORTERM=truecolor`、`TERM_PROGRAM=RoveTerm`。

## 线程与结束顺序

1. reader 线程每次最多读 16 KiB，通过有界 channel 交给 `SessionManager`。
2. writer 线程从输入队列取数据并按顺序写入 PTY。
3. waiter 线程等待 shell 退出；在 Windows 上它随后释放伪控制台，让 reader 读到文件结束。
4. reader 读到文件结束后，取得 waiter 的退出状态并发送 `Exit`。

父进程在启动 shell 后立即关闭 slave 端，否则 reader 永远读不到文件结束。

## 本机状态

`LocalMonitor::sample` 在 macOS、Linux 和 Windows 上采集 CPU、内存、系统、磁盘和网卡数据，并返回与 SSH 主机共用的 `ServerStats`。首次采样建立 CPU 基线，后续采样用累加计数表示 CPU 使用率。完成采样后的 250 毫秒内复用该样本。本机样本的链路延迟列表为空。
