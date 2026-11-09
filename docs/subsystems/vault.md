# vault

`rove-vault` 存储主机、凭据、known_hosts 和端口转发规则，用设备主密钥按记录加密凭据，并定义 `KeyCache` seam。

## KeyCache seam

```rust
pub trait KeyCache: Send + Sync {
    fn load(&self, interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError>;
    fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError>;
}
```

| Provider | 平台 | 说明 |
|---|---|---|
| `OsKeyCache` | macOS / Windows / Linux | 钥匙串条目 `(keychain_service, "master/v1")` |
| `MemoryKeyCache` | 任意 | 不持久化，只在启用 `memory-key-cache` feature 时可用 |

`Interaction::Silent` 表示启动阶段，不应弹出授权；`Interaction::Allowed` 表示用户发起的操作。

## 解锁状态

| 状态 | 含义 | 出路 |
|---|---|---|
| `Locked { NotAttempted }` | 尚未尝试解锁 | `try_unlock()` |
| `Locked { StoreUnavailable }` | 无法访问系统钥匙串 | 修复环境后重试 |
| `Locked { Denied }` | 用户拒绝授权 | `unlock()` 重试 |
| `Locked { KeyLost }` | 已有凭据，但钥匙串中的密钥缺失或不匹配 | `reset_secrets()` 丢弃凭据 |
| `Unlocked` | 可以读写凭据 | — |

数据库的 `meta.key_check` 保存一段用当前密钥加密的固定明文，用于判断钥匙串中的密钥是否匹配。

## 加密

- 加密密钥 = HKDF-SHA256(设备主密钥, info = `rove/vault/enc/v1`)。
- 每条凭据序列化为 JSON 后用 XChaCha20-Poly1305 加密，存储格式为 24 字节随机 nonce 加密文。
- AAD 为 `rove/vault/host-secret/v1:<HostId>`，密文无法被挪到其他主机名下解密。

## 记录

| 类型 | 说明 |
|---|---|
| `Host` | `id`、`name`、`group`、`hostname`、`port`、`username`、`auth`、`route`、`tags`、时间戳与最近连接时间 `last_connected_at`；不含任何秘密 |
| `HostDraft` | 新建或修改主机的输入，由 `validate()` 去除空白并检查长度和字符 |
| `AuthMethod` | `Password`、`PrivateKey`（每台主机单独保存的私钥）、`Key { key }`（密钥库中的密钥）、`KeyFile { path }`、`Agent`、`Interactive` |
| `HostRoute` | `Direct`（默认）、`Jump { host }`（经另一台已保存主机跳转）、`Proxy { protocol, host, port }`（`http` 或 `socks5`，无认证） |
| `Secret` | `Password`、`PrivateKey { key, passphrase }`、`Passphrase`；字段为 `Zeroizing<String>`，`Debug` 不输出内容 |
| `HostKey` | 服务器公钥：算法、OpenSSH 单行格式、`SHA256:` 指纹 |
| `KnownHostStatus` | `Trusted`、`Unknown`、`Mismatch { expected_algorithm, expected_fingerprint }` |

每台主机最多一条凭据。known_hosts 按（小写主机名, 端口）记录一把受信任的公钥。

保存主机时校验路线：代理地址与端口由 `HostDraft::validate` 检查；跳板机必须是另一台已存在的主机，沿跳板链不能回到自身，链长不超过 `MAX_JUMP_DEPTH`（4）。删除主机时，以它为跳板的主机改为直连。

端口转发规则 `ForwardRule { id, host_id, label, spec, auto_start }` 属于一台主机，不含秘密，锁定时照常可读写。`ForwardSpec` 有三种：

| 类型 | 字段 | 校验 |
|---|---|---|
| `local`（`-L`） | `bind_host`、`bind_port`、`dest_host`、`dest_port` | `bind_host` 只能是 IP 地址或 `localhost`；端口 1–65535 |
| `remote`（`-R`） | 同上 | `bind_host` 由服务器解释；`bind_port` 为 0 时由服务器分配 |
| `dynamic`（`-D`） | `bind_host`、`bind_port` | 同 `local` |

`listens_publicly()` 判断本地监听地址是否对其他机器开放（非回环地址），UI 据此给出提示。

密钥库保存具名的 SSH 密钥 `SshKey { id, name, algorithm, public_key, fingerprint, has_private }`，名称不区分大小写唯一。名称和公钥在锁定时可读；私钥和 passphrase（`KeySecret`）按记录绑定加密，读写需要解锁。vault 不解析密钥，调用方从私钥推出算法、公钥和指纹后通过 `KeyDraft` 传入。主机以 `AuthMethod::Key` 引用密钥，保存时检查密钥存在；仍被主机使用的密钥不能删除（`KeyInUse` 列出这些主机）。重置凭据时密钥条目保留，私钥被丢弃，`has_private` 变为 `false`，需重新导入。`touch_host` 记录最近一次成功连接的时间。

偏好设置 `Preferences` 以一条记录保存，锁定时照常可读写；记录中缺少的字段取默认值，因此旧记录在新增字段后仍可读取。`set_preferences` 先经 `validate()` 校验：

| 字段 | 默认值 | 校验 |
|---|---|---|
| `terminal.font_family` | 空（使用内置字体栈） | 去除首尾空白，最多 100 个字符，不含引号、逗号、分号、反斜杠、花括号和控制字符 |
| `terminal.font_size` | 13 | 8–32 |
| `terminal.line_height` | 1.15 | 1.0–2.0 |

应用通过 `prefs_get` / `prefs_set` 命令读写偏好设置；`fonts_monospace` 用 fontdb 列出本机已安装的等宽字体族（去重、按名称排序），供字体选择使用，移动端返回空列表。

## 表结构

`meta`、`hosts`、`host_secrets`（外键级联删除）、`known_hosts`、`forwards`（外键级联删除）、`preferences`、`ssh_keys`。主机的路线以 JSON 存在 `hosts.route` 列。结构版本记录在 `PRAGMA user_version`，当前版本为 5；打开旧版本数据库时按版本依次迁移。
