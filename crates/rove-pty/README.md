# rove-pty

提供本地 PTY 终端服务 `LocalTerminals` 和桌面主机状态服务 `LocalMonitor`。参考见 [docs/subsystems/pty.md](../../docs/subsystems/pty.md)。

## 契约

- `LocalTerminals::open(LocalRequest)` 先通过 `LocalRequest::resolve` 确定全部取值（请求 → 插件配置 → 平台默认），再启动 shell；显式提供 `args` 时替换平台默认 shell 参数，`env` 合并覆盖默认终端变量。
- 每个终端使用三个线程：reader 转发输出，writer 按顺序写入输入，waiter 收集退出状态。shell 退出或会话关闭时三个线程都会结束。
- 输出 channel 先交付全部输出，最后交付一个 `TerminalOutput::Exit`，随后关闭。
- `close` 终止 shell 并释放 PTY，可以重复调用；关闭后 `write` 和 `resize` 返回 `SessionError::Closed`。

## 配置（`[plugin.pty]`）

| 键 | 默认值 | 含义 |
|---|---|---|
| `shell` | 平台默认 | 请求未指定 shell 时使用的程序，不能为空字符串 |
| `args` | `[]` | 配合 `shell` 使用的参数；未配置 `shell` 时忽略 |

## 局限

- Windows 上依赖 ConPTY（Windows 10 1809 及以上）。
- Android 上能编译，但 `mobile` profile 不启用本插件。
