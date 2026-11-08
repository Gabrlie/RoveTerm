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
