# ssh

`rove-ssh` 提供 `SshConnector` 服务：SSH 认证、host key 校验，以及在连接上打开 PTY shell。

## 类型

| 类型 | 说明 |
|---|---|
| `SshTarget` | `hostname`、`port`、`username`，以及可选的 `host_id` 和标题；`address()` 返回 `user@host:port`，IPv6 地址加方括号 |
| `AuthSpec` | `Password`、`PrivateKey { key, passphrase }`、`KeyFile { path, passphrase }`、`Agent` |
| `SshConnection` | 已认证的连接；`open_shell(size)` 打开 PTY shell channel |
| `SshError` | `Connect`、`Timeout`、`HostKey(HostKeyRejection)`、`AuthFailed`、`CredentialsRequired`、`Key`、`Agent`、`Channel`、`Protocol` |

## 凭据解析

`AuthSpec::resolve(host, stored_secret, typed_password)`：

| 认证方式 | 结果 |
|---|---|
| `Password` | 用户输入的密码 → 已存密码 → `CredentialsRequired` |
| `Interactive` | 用户输入的密码 → `CredentialsRequired` |
| `PrivateKey` | 已存私钥；没有时返回 `Key` 错误 |
| `KeyFile { path }` | 读取密钥文件；passphrase 依次取已存的 → 用户输入的 |
| `Agent` | 依次尝试 agent 中的公钥：Unix 用 `SSH_AUTH_SOCK`，Windows 先用 OpenSSH agent 命名管道，再用 Pageant |

## host key 校验

`HostKeyVerifier` 是可替换的校验策略。`VaultVerifier` 的行为：

| vault 中的记录 | 结果 |
|---|---|
| 一致 | 通过 |
| 不一致 | 返回 `HostKeyRejection::Mismatch`，不询问用户 |
| 没有记录 | 通过 `HostKeyPrompts::ask` 发出 `HostKeyPrompt`，等待回答 |

`HostKeyPrompt { request_id, hostname, port, key }` 是 Live 事件，UI 通过 `HostKeyPrompts::respond(request_id, Trust | Reject)` 回答。超时未回答时返回 `PromptTimeout`；选择 `Trust` 时密钥写入 vault。

## PTY shell

`open_shell` 请求 `xterm-256color` PTY 和 shell，然后把 channel 拆成读写两半：读半部分由一个任务转发 `Data` / `ExtendedData`，并在 channel 关闭时交付 `Exit`，其中包含退出码或信号；写半部分实现 `TerminalSession` 的 `write`、`resize`（window-change）和 `close`。

## 连接池

`SshConnector` 按 (小写主机名, 端口, 用户名) 维护连接池：

- `shared(target, auth)` 返回该目标的存活连接；没有时用 `auth` 新建。每个目标有一把异步锁，并发请求只会建立一条连接。
- `open_terminal` 在共享连接上打开 channel；如果复用的连接打开 channel 失败，丢弃它并重连一次。
- 每个打开的 channel 持有一个租约。最后一个租约释放后，启动一个 `idle_disconnect_secs` 的计时器；期间连接被再次取出或打开新 channel，计时作废，否则断开连接。
- `live_connections()` 返回存活连接数；`disconnect_all()` 用于退出应用时断开全部连接。
