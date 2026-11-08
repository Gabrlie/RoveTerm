# rove-ssh

SSH 终端 Provider：认证、host key 校验，以及打开 PTY shell channel。参考见 [docs/subsystems/ssh.md](../../docs/subsystems/ssh.md)。

## 契约

- `SshConnector::connect` 依次完成：TCP 连接（受 `connect_timeout_secs` 限制）→ host key 校验 → 认证。host key 等待用户确认的时间不计入连接超时。
- host key 由 `HostKeyVerifier` 决定是否接受。默认的 `VaultVerifier`：已记录且一致的密钥直接通过；已记录但不一致的密钥直接拒绝，不询问用户；未知密钥发出 `HostKeyPrompt` Live 事件等待用户回答，用户选择信任后写入 vault。
- `SshConnection::open_shell` 返回的终端持有连接的引用；只要还有终端打开，连接就保持。
- `AuthSpec` 中的秘密材料在 drop 时清零。密码认证失败后，会用同一个密码尝试 keyboard-interactive。
- `AuthSpec::resolve` 根据主机的认证方式、已存凭据和用户输入的密码确定凭据；需要先向用户索取密码时返回 `CredentialsRequired`。

## 配置（`[plugin.ssh]`）

| 键 | 默认值 | 范围 | 含义 |
|---|---|---|---|
| `connect_timeout_secs` | 15 | 1–120 | TCP 连接超时 |
| `keepalive_secs` | 30 | 5–600 | keepalive 间隔 |
| `keepalive_max` | 3 | 1–10 | 连续多少次无应答后视为断开 |
| `hostkey_prompt_secs` | 120 | 10–600 | 等待用户确认 host key 的时间 |

## 局限

- 暂不支持 OpenSSH 证书认证、GSSAPI。
- 私钥支持 OpenSSH 和 PEM 格式，暂不支持 PuTTY 的 `.ppk` 格式。
