# ssh

`rove-ssh` 提供 `SshConnector` 和 `Forwards` 服务：SSH 认证、host key 校验，在连接上打开 PTY shell 和 SFTP 会话，以及端口转发。

## 类型

| 类型 | 说明 |
|---|---|
| `SshTarget` | `hostname`、`port`、`username`，以及可选的 `host_id` 和标题；`address()` 返回 `user@host:port`，IPv6 地址加方括号 |
| `AuthSpec` | `Password`、`PrivateKey { key, passphrase }`、`KeyFile { path, passphrase }`、`Agent { passphrase }` |
| `SshConnection` | 已认证的连接；`open_shell(size)` 打开 PTY shell channel，`open_sftp()` 打开 SFTP 会话 |
| `Via` | 建立传输的方式：`Direct`、`Proxy { protocol, host, port }`、`Jump(Arc<SshConnection>)` |
| `ConnectStep` | 连接过程的步骤：`Connecting`、`Handshake`、`HostKeyAccepted`、`AgentUnavailable`、`Trying` / `Rejected { method, label }`、`Authenticated` |
| `ExecOutput` | `exec` 的结果：`stdout`、`stderr`、`code`、`truncated` |
| `SshError` | `Connect`、`Timeout`、`HostKey(HostKeyRejection)`、`AuthFailed`、`CredentialsRequired`、`Key`、`PassphraseRequired`、`Agent`、`Channel`、`ForwardBind`、`ForwardRejected`、`Proxy`、`Jump`、`Sftp`、`Protocol` |

## 凭据解析

`AuthSpec::resolve(host, stored_secret, typed)`。对基于密钥的方式，`typed` 是 passphrase，优先于已存的 passphrase：

| 认证方式 | 结果 |
|---|---|
| `Password` | 用户输入的密码 → 已存密码 → `CredentialsRequired` |
| `Interactive` | 用户输入的密码 → `CredentialsRequired` |
| `PrivateKey` | 已存私钥；没有时返回 `Key` 错误 |
| `KeyFile { path }` | 读取密钥文件，路径开头的 `~` 展开为用户主目录 |
| `Agent` | 先尝试 agent 中的公钥（Unix 用 `SSH_AUTH_SOCK`，Windows 先用 OpenSSH agent 命名管道，再用 Pageant），再依次尝试 `~/.ssh/id_ed25519`、`id_ecdsa`、`id_rsa` 中存在的文件 |

私钥已加密而没有 passphrase，或 passphrase 无法解密时返回 `PassphraseRequired`，界面据此询问 passphrase；文件读不到或格式不对时返回带原因的 `Key`。`Agent` 方式下 agent 不可用不算错误，只有默认密钥也都不可用时才返回 `Agent`。

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

- `shared(target, auth)` 返回该目标的存活连接；没有时用 `auth` 直连新建。`shared_via(target, auth, via)` 按 `via` 建立传输。每个目标有一把异步锁，并发请求只会建立一条连接。
- `live(target)` 返回存活连接，不需要凭据；没有时返回 `None`。`open_live(target, size)` 在它之上打开 shell。
- `open_terminal` 在共享连接上打开 channel；如果复用的连接打开 channel 失败，丢弃它并重连一次。
- `evict(target, connection)` 在该连接仍是共享连接时把它移出连接池，已打开的 channel 不受影响。
- 每个打开的 channel 持有一个租约。最后一个租约释放后，启动一个 `idle_disconnect_secs` 的计时器；期间连接被再次取出或打开新 channel，计时作废，否则断开连接。
- `live_connections()` 返回存活连接数；`disconnect_all()` 用于退出应用时断开全部连接。

## 跳板机与代理

`connect_timeout_secs` 覆盖建立传输的全过程，包括代理握手和跳板通道：

- `Proxy`：先连代理，再用 HTTP `CONNECT` 或 SOCKS5 `CONNECT`（不认证，目标名称交给代理解析）打开到目标的隧道。代理拒绝或协议不符时返回带原因的 `Proxy`。HTTP 响应头逐字节读取，不会吞掉随后的 SSH 版本行。
- `Jump`：在跳板连接上打开 `direct-tcpip` channel，在其上进行 SSH 握手。新连接持有跳板连接的一个租约，因此跳板连接在新连接存在期间（包括其空闲计时）不会断开。跳板机无法连到目标时返回 `Jump`。
- host key 按目标的主机名和端口校验，与是否经过跳板无关。

打开终端时，应用把连接步骤以 `{ "progress": ConnectStep }` 消息先于终端输出发到会话的输出 channel，跳板机的步骤也包含在内。用户输入的值默认按主机认证方式解释（密码或 passphrase）；`password_kind = "password"` 表示对密钥主机改用密码登录，这种密码不会被记住。成功连接已保存主机后调用 `touch_host`。

应用连接已保存主机时（`src-tauri/src/connect.rs`）先解析目标的凭据，再沿 `HostRoute` 逐级取得跳板机的共享连接（已有存活连接时直接复用），最后经最近一跳连接目标。跳板机只能用已保存的密码、未加密的密钥或 agent 认证，不会弹框询问；连不上跳板机时返回 `ssh.jump_host`，消息中注明是哪台跳板机及原因。

## 连接过程

`connect_via` / `shared_via` 接收可选的 `Progress` 回调，按发生顺序报告 `ConnectStep`。每次提交凭据前报告 `Trying`，被拒绝时报告 `Rejected`：`label` 是密钥文件路径，或密钥注释（为空时是算法和指纹）。认证全部失败时最后一步是 `Rejected`，界面据此显示停在哪一步。

## 执行命令与密钥

- `SshConnection::exec(command, limit)` 在不分配 PTY 的 session channel 上运行命令，分别收集 stdout 和 stderr，每路最多 `limit` 字节，超出部分丢弃并置 `truncated`；超时由调用方控制。
- `generate_key(kind, comment, passphrase)` 生成 Ed25519、ECDSA P-256 或 RSA 3072 密钥，返回 OpenSSH 格式文本，提供 passphrase 时加密。
- `describe_key(private, passphrase)` 解析私钥，返回算法、带注释的 OpenSSH 公钥行和 `SHA256:` 指纹。
- `authorize_command(public_key)` 生成把公钥追加到 `~/.ssh/authorized_keys` 的命令（已存在则不重复），只保留算法和密钥数据，注释不会进入 shell。

<a id="sftp"></a>

## SFTP

`SshConnection::open_sftp` 在连接上开一个 session channel，请求 `sftp` 子系统并等待服务器确认，然后在 channel 流上初始化 `russh-sftp` 客户端，得到实现 [`FileSystem`](fs.md) 的 `SftpFs`。

- `SftpFs` 持有一个 channel 租约，和终端一样让连接保持打开；drop 时关闭 SFTP 会话并释放租约。
- 服务器拒绝子系统时返回 `SshError::Sftp`。
- 读写使用 `russh-sftp` 的流水线请求（默认各 16 个并发请求），单个请求超时 10 秒。
- SFTP 状态码映射为 `FsError`：`NoSuchFile` → `NotFound`，`PermissionDenied` → `PermissionDenied`，`OpUnsupported` → `Unsupported`，连接丢失 → `Closed`，其余 → `Io`。
- `list` 过滤掉服务器返回的 `.` 和 `..`；`label()` 是 `user@host:port`。

## 端口转发

`Forwards` 按转发规则 ID（`ForwardId`）运行转发，规则的内容 `ForwardSpec` 来自 [vault](vault.md)。`start(id, host_id, connection, spec)` 在给定的共享连接上启动：

| 类型 | 做法 |
|---|---|
| `local` | 在本机 `bind_host:bind_port` 监听；每个连接打开一个 `direct-tcpip` channel 到 `dest_host:dest_port` |
| `dynamic` | 在本机监听 SOCKS5（无认证，只支持 `CONNECT`）；按客户端请求的目标打开 `direct-tcpip`，目标不可达时回复 host unreachable |
| `remote` | 发送 `tcpip-forward`，请求服务器监听；服务器打开的 `forwarded-tcpip` channel 按（地址, 端口）查路由，连接本机的 `dest_host:dest_port`；没有匹配路由的 channel 被拒绝 |

- 每个运行中的转发持有一个 channel 租约，每条转发中的连接再各持有一个，连接在转发停止前不会因空闲而断开。
- 本地端口被占用时返回 `ForwardBind`，服务器拒绝远程监听时返回 `ForwardRejected`，此时不登记转发。没有用户等待结果的调用方（自动启动）用 `record_failure` 登记一条 `failed` 状态，让 UI 能看到错误。
- `ForwardStatus { id, host_id, spec, state, bound_port, active, total, error }` 是 Live 事件。`bound_port` 是实际监听的端口（远程转发请求端口 0 时由服务器分配）。状态在启动、停止、失败时发出；连接计数每秒最多发出一次。
- 监督任务每秒检查一次连接；连接关闭时状态变为 `failed`，监听关闭，条目保留到再次 `start` 或 `stop`。
- `stop` 立即关闭监听和全部转发中的连接，远程转发会发送 `cancel-tcpip-forward`。重复 `start` 一个运行中的转发直接返回当前状态。

应用通过 `src-tauri/src/commands/forward.rs` 暴露转发：`forward_list` 返回规则及其状态，`forward_create` / `forward_update` / `forward_delete` 管理规则（修改或删除时先停止正在运行的转发），`forward_start` 用与打开终端相同的方式获取连接后启动，`forward_stop` 停止。`ForwardStatus` 以 `forward-update` 事件转发给 WebView。在已保存主机上打开终端后，该主机标记为 `auto_start` 且未在运行的规则会在同一连接上自动启动，失败时登记为 `failed`。退出应用时停止全部转发。

## 快速连接

`parse_quick_connect` 接受以下形式，返回 `QuickTarget { username, hostname, port }`，其中用户名和端口可以省略：

- `host`、`user@host`、`host:port`、`user@host:port`
- `[IPv6]:port`、不带端口的 IPv6 地址
- `ssh://user@host:port`
- `ssh [-p port] [-l user] user@host`（不支持其他选项）

## ssh_config 导入

`parse_ssh_config` 为 `Host` 行中每个不含通配符的别名生成一条 `SshConfigHost`。每个字段按 OpenSSH 的首次匹配规则取值：按文件顺序遍历所有与该别名匹配的块（包括 `Host *`、带 `!` 的排除模式，以及第一个 `Host` 行之前的全局设置），采用第一个出现的值。读取的字段有 `HostName`、`Port`、`User`、`IdentityFile`（取第一个）和 `ProxyJump`；`Match` 块和 `Include` 指令被忽略。

`import_ssh_config(vault, text, home, default_user)` 的转换规则：有 `IdentityFile` 时认证方式为 `KeyFile`，并把 `~` 展开为 `home`；否则为 `Agent`。`ProxyJump` 的最后一跳是本次导入的别名或已有主机的名称时，设为经该主机跳转；带 `user@`、端口的跳板写法不导入。缺少 `User` 时使用 `default_user`，导入的主机带 `ssh-config` 标签。用户名、主机名（不区分大小写）、端口都相同的已有主机会被跳过。
