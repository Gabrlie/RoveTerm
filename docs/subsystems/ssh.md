# ssh

`rove-ssh` 提供 `SshConnector` 服务：SSH 认证、host key 校验，以及在连接上打开 PTY shell 和 SFTP 会话。

## 类型

| 类型 | 说明 |
|---|---|
| `SshTarget` | `hostname`、`port`、`username`，以及可选的 `host_id` 和标题；`address()` 返回 `user@host:port`，IPv6 地址加方括号 |
| `AuthSpec` | `Password`、`PrivateKey { key, passphrase }`、`KeyFile { path, passphrase }`、`Agent` |
| `SshConnection` | 已认证的连接；`open_shell(size)` 打开 PTY shell channel，`open_sftp()` 打开 SFTP 会话 |
| `SshError` | `Connect`、`Timeout`、`HostKey(HostKeyRejection)`、`AuthFailed`、`CredentialsRequired`、`Key`、`Agent`、`Channel`、`Sftp`、`Protocol` |

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
- `live(target)` 返回存活连接，不需要凭据；没有时返回 `None`。`open_live(target, size)` 在它之上打开 shell。
- `open_terminal` 在共享连接上打开 channel；如果复用的连接打开 channel 失败，丢弃它并重连一次。
- `evict(target, connection)` 在该连接仍是共享连接时把它移出连接池，已打开的 channel 不受影响。
- 每个打开的 channel 持有一个租约。最后一个租约释放后，启动一个 `idle_disconnect_secs` 的计时器；期间连接被再次取出或打开新 channel，计时作废，否则断开连接。
- `live_connections()` 返回存活连接数；`disconnect_all()` 用于退出应用时断开全部连接。

<a id="sftp"></a>

## SFTP

`SshConnection::open_sftp` 在连接上开一个 session channel，请求 `sftp` 子系统并等待服务器确认，然后在 channel 流上初始化 `russh-sftp` 客户端，得到实现 [`FileSystem`](fs.md) 的 `SftpFs`。

- `SftpFs` 持有一个 channel 租约，和终端一样让连接保持打开；drop 时关闭 SFTP 会话并释放租约。
- 服务器拒绝子系统时返回 `SshError::Sftp`。
- 读写使用 `russh-sftp` 的流水线请求（默认各 16 个并发请求），单个请求超时 10 秒。
- SFTP 状态码映射为 `FsError`：`NoSuchFile` → `NotFound`，`PermissionDenied` → `PermissionDenied`，`OpUnsupported` → `Unsupported`，连接丢失 → `Closed`，其余 → `Io`。
- `list` 过滤掉服务器返回的 `.` 和 `..`；`label()` 是 `user@host:port`。

## 快速连接

`parse_quick_connect` 接受以下形式，返回 `QuickTarget { username, hostname, port }`，其中用户名和端口可以省略：

- `host`、`user@host`、`host:port`、`user@host:port`
- `[IPv6]:port`、不带端口的 IPv6 地址
- `ssh://user@host:port`
- `ssh [-p port] [-l user] user@host`（不支持其他选项）

## ssh_config 导入

`parse_ssh_config` 为 `Host` 行中每个不含通配符的别名生成一条 `SshConfigHost`。每个字段按 OpenSSH 的首次匹配规则取值：按文件顺序遍历所有与该别名匹配的块（包括 `Host *`、带 `!` 的排除模式，以及第一个 `Host` 行之前的全局设置），采用第一个出现的值。读取的字段有 `HostName`、`Port`、`User`、`IdentityFile`（取第一个）和 `ProxyJump`；`Match` 块和 `Include` 指令被忽略。

`import_ssh_config(vault, text, home, default_user)` 的转换规则：有 `IdentityFile` 时认证方式为 `KeyFile`，并把 `~` 展开为 `home`；否则为 `Agent`。缺少 `User` 时使用 `default_user`，导入的主机带 `ssh-config` 标签。用户名、主机名（不区分大小写）、端口都相同的已有主机会被跳过。
