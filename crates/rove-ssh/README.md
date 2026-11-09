# rove-ssh

SSH Provider：认证、host key 校验，打开 PTY shell channel，以及 `FileSystem` seam 的 SFTP 实现。参考见 [docs/subsystems/ssh.md](../../docs/subsystems/ssh.md)。

## 契约

- `SshConnector::connect` 依次完成：TCP 连接（受 `connect_timeout_secs` 限制）→ host key 校验 → 认证。host key 等待用户确认的时间不计入连接超时。
- host key 由 `HostKeyVerifier` 决定是否接受。默认的 `VaultVerifier`：已记录且一致的密钥直接通过；已记录但不一致的密钥直接拒绝，不询问用户；未知密钥发出 `HostKeyPrompt` Live 事件等待用户回答，用户选择信任后写入 vault。
- `SshConnector::open_terminal` 复用同一 `user@host:port`（主机名不区分大小写）的连接，每个终端是一个独立的 channel；并发打开同一目标时只建立一条连接。复用时忽略传入的 `AuthSpec`。
- 连接上最后一个终端关闭后，连接再空闲 `idle_disconnect_secs` 秒才断开；期间有新终端打开则取消断开。
- 复用的连接打开 channel 失败时，丢弃这条连接并重连一次。
- `open_live` 只在已有存活连接时打开 channel，不需要凭据；没有存活连接时返回 `None`。应用在解析凭据之前先调用它，因此在已连接的主机上分屏不会再次索取密码。
- `SshConnector::connect` 建立不进入连接池的独立连接；`live` 返回存活的共享连接，不会新建。
- `SshConnection::open_sftp` 打开 SFTP 会话 `SftpFs`（实现 `rove_fs::FileSystem`）。会话和终端一样持有 channel 租约；服务器不提供 `sftp` 子系统时返回 `SshError::Sftp`。
- `AuthSpec` 中的秘密材料在 drop 时清零。密码认证失败后，会用同一个密码尝试 keyboard-interactive。
- `AuthSpec::resolve` 根据主机的认证方式、已存凭据和用户输入的密码确定凭据；需要先向用户索取密码时返回 `CredentialsRequired`。

- `parse_quick_connect` 解析快速连接字符串；`parse_ssh_config` 解析 OpenSSH 客户端配置；`import_ssh_config` 把配置中的主机写入 vault，跳过用户名、主机名、端口都相同的已有主机。

## 配置（`[plugin.ssh]`）

| 键 | 默认值 | 范围 | 含义 |
|---|---|---|---|
| `connect_timeout_secs` | 15 | 1–120 | TCP 连接超时 |
| `keepalive_secs` | 30 | 5–600 | keepalive 间隔 |
| `keepalive_max` | 3 | 1–10 | 连续多少次无应答后视为断开 |
| `hostkey_prompt_secs` | 120 | 10–600 | 等待用户确认 host key 的时间 |
| `idle_disconnect_secs` | 60 | 0–3600 | 最后一个终端关闭后连接保留的时间 |

## 局限

- 暂不支持 OpenSSH 证书认证、GSSAPI。
- 私钥支持 OpenSSH 和 PEM 格式，暂不支持 PuTTY 的 `.ppk` 格式。
- ssh_config 解析忽略 `Match` 块和 `Include` 指令；`ProxyJump` 会被解析出来，但还没有接入连接流程。
