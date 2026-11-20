# rove-vault

存储主机、主机凭据、known_hosts 和端口转发规则；凭据用设备主密钥派生的密钥加密。参考见 [docs/subsystems/vault.md](../../docs/subsystems/vault.md)，密钥层级见 [docs/security.md](../../docs/security.md)。

## 契约

- `installation_id` 独立于同步空间，标识定时任务的执行设备。审计正文用设备密钥密封，并绑定会话 ID 与正文引用。

- `Vault` 打开后处于锁定状态。主机元数据和 known_hosts 在锁定时照常可读写；读写凭据需要先解锁，否则返回 `VaultError::Locked`。
- `try_unlock()` 用于启动阶段，失败时只改变锁定状态，不返回错误。`unlock()` 用于用户发起的操作，允许系统弹出授权。
- 首次解锁且没有任何密钥时，生成新的设备主密钥并写入 `KeyCache`。
- vault 中已有凭据、但 `KeyCache` 中的密钥缺失或不匹配时，状态为 `KeyLost`；只能通过 `reset_secrets()` 丢弃全部凭据后重新解锁，主机和 known_hosts 保留。
- 删除主机时级联删除它的凭据；修改主机的认证方式类别时删除原有凭据。
- 主机的跳板机必须是另一台已存在的主机，跳板链无环且不超过 `MAX_JUMP_DEPTH`；删除跳板机后依赖它的主机改为直连。
- 端口转发规则随主机级联删除；`ForwardDraft::validate` 拒绝不是 IP 地址或 `localhost` 的本地监听地址。
- 密钥库：名称与公钥锁定时可读，私钥加密保存；被主机使用的密钥不能删除；重置凭据后保留条目、丢弃私钥。
- 快捷指令（snippets）不含秘密，名称为空时取命令第一行。
- `preferences()` 在没有记录时返回默认值；`set_preferences()` 校验后整体替换。`device_preferences()` / `set_device_preferences()` 读写只属于本机的 `DevicePrefs`，单独存放，不随同步离开设备。偏好设置不含秘密，锁定时可读写。
- `DevicePrefs.external_editor` 是本机编辑器配置，旧记录缺字段时默认禁用；只校验绝对路径语法和参数边界，不查询磁盘。完整限制及参数传递契约见 [本机编辑器偏好](../../docs/subsystems/vault.md#本机编辑器偏好)。
- 数据库结构版本记录在 `PRAGMA user_version` 中，只增不减，旧版本按顺序迁移；打开更新版本写入的数据库时返回 `FutureSchema`。

## 配置（`[plugin.vault]`）

| 键 | 默认值 | 含义 |
|---|---|---|
| `database` | 必填 | 数据库路径，由应用根据数据目录设置 |
| `keychain_service` | `dev.roveterm.app` | 系统钥匙串中的服务名；开发构建使用不同的名字 |

宿主可以在应用插件之前向 `Context` 提供自己的 `dyn KeyCache`；否则插件使用 `OsKeyCache`。

## Feature

- `memory-key-cache`：启用 `MemoryKeyCache`，一个不持久化的 `KeyCache`，只用于测试和开发构建。

## 局限

- Linux 需要可用的 Secret Service；`keyring` 无法保证"不弹窗"，Secret Service 集合被锁定时可能弹出解锁对话框。
- Android 和 iOS 上没有内置的 `KeyCache`，宿主必须自行提供。
