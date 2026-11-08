# vault

`rove-vault` 存储主机、凭据和 known_hosts，用设备主密钥按记录加密凭据，并定义 `KeyCache` seam。

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
| `Host` | `id`、`name`、`group`、`hostname`、`port`、`username`、`auth`、`tags`、时间戳；不含任何秘密 |
| `HostDraft` | 新建或修改主机的输入，由 `validate()` 去除空白并检查长度和字符 |
| `AuthMethod` | `Password`、`PrivateKey`、`KeyFile { path }`、`Agent`、`Interactive` |
| `Secret` | `Password`、`PrivateKey { key, passphrase }`、`Passphrase`；字段为 `Zeroizing<String>`，`Debug` 不输出内容 |
| `HostKey` | 服务器公钥：算法、OpenSSH 单行格式、`SHA256:` 指纹 |
| `KnownHostStatus` | `Trusted`、`Unknown`、`Mismatch { expected_algorithm, expected_fingerprint }` |

每台主机最多一条凭据。known_hosts 按（小写主机名, 端口）记录一把受信任的公钥。

## 表结构

`meta`、`hosts`、`host_secrets`（外键级联删除）、`known_hosts`。结构版本记录在 `PRAGMA user_version`，当前版本为 1。
