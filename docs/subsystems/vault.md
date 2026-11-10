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
| `Host` | `id`、`name`、`group`、`hostname`、`port`、`username`、`auth`、`route`、`tags`、时间戳与最近连接时间 `last_connected_at`、检测到的系统 `os`；不含任何秘密 |
| `HostOs` | `id`（os-release 的 `ID`，或由 `uname` 得出的 `macos`、`freebsd`）、`like`（`ID_LIKE`）、`name`（`PRETTY_NAME`）；只用于显示，`set_host_os` 写入前校验 id 为不超过 32 字符的小写短名，名称截到 100 个可打印字符 |
| `HostDraft` | 新建或修改主机的输入，由 `validate()` 去除空白并检查长度和字符 |
| `AuthMethod` | `Password`、`PrivateKey`（每台主机单独保存的私钥）、`Key { key }`（密钥库中的密钥）、`KeyFile { path }`、`Agent`、`Interactive` |
| `HostRoute` | `Direct`（默认）、`Jump { host }`（经另一台已保存主机跳转）、`Proxy { protocol, host, port }`（`http` 或 `socks5`，无认证） |
| `Secret` | `Password`、`PrivateKey { key, passphrase }`、`Passphrase`；字段为 `Zeroizing<String>`，`Debug` 不输出内容 |
| `HostKey` | 服务器公钥：算法、OpenSSH 单行格式、`SHA256:` 指纹 |
| `KnownHostStatus` | `Trusted`、`Unknown`、`Mismatch { expected_algorithm, expected_fingerprint }` |

每台主机最多一条凭据。known_hosts 按（小写主机名, 端口, 路线）记录一把受信任的公钥，直连的路线为空字符串。

保存主机时校验路线：代理地址与端口由 `HostDraft::validate` 检查；跳板机必须是另一台已存在的主机，沿跳板链不能回到自身，链长不超过 `MAX_JUMP_DEPTH`（4）。删除主机时，以它为跳板的主机改为直连。

端口转发规则 `ForwardRule { id, host_id, label, spec, auto_start }` 属于一台主机，不含秘密，锁定时照常可读写。`ForwardSpec` 有三种：

| 类型 | 字段 | 校验 |
|---|---|---|
| `local`（`-L`） | `bind_host`、`bind_port`、`dest_host`、`dest_port` | `bind_host` 只能是 IP 地址或 `localhost`；端口 1–65535 |
| `remote`（`-R`） | 同上 | `bind_host` 由服务器解释；`bind_port` 为 0 时由服务器分配 |
| `dynamic`（`-D`） | `bind_host`、`bind_port` | 同 `local` |

`listens_publicly()` 判断本地监听地址是否对其他机器开放（非回环地址），UI 据此给出提示。

密钥库保存具名的 SSH 密钥 `SshKey { id, name, algorithm, public_key, fingerprint, has_private }`，名称不区分大小写唯一。名称和公钥在锁定时可读；私钥和 passphrase（`KeySecret`）按记录绑定加密，读写需要解锁。vault 不解析密钥，调用方从私钥推出算法、公钥和指纹后通过 `KeyDraft` 传入。主机以 `AuthMethod::Key` 引用密钥，保存时检查密钥存在；仍被主机使用的密钥不能删除（`KeyInUse` 列出这些主机）。重置凭据时密钥条目保留，私钥被丢弃，`has_private` 变为 `false`，需重新导入。`touch_host` 记录最近一次成功连接的时间。

应用通过 `src-tauri/src/commands/keys.rs` 暴露密钥库：`key_list`、`key_generate`（在阻塞线程上生成，RSA 需要数秒）、`key_import`（加密私钥缺少 passphrase 时返回 `ssh.passphrase_required`；可附带用户自己的公钥，经 `check_public_key` 确认与私钥配对后原样保存，否则使用从私钥推出的公钥，不匹配时返回 `key.public_invalid`）、`key_load_file`（读取用户选择的密钥文件，最大 64 KiB；按内容判断是私钥还是公钥——`-----BEGIN` 或 PuTTY 头为私钥，单行 `ssh-`、`ecdsa-sha2-`、`sk-` 开头为公钥——再读取旁边的另一半，都不是时返回 `key.unrecognized`）、`key_default_files`（桌面端列出 `~/.ssh` 中存在的默认密钥文件，按 ed25519、ecdsa、rsa 及其 `_sk` 变体的顺序；应用不在沙盒中可以直接读取，移动端返回空）、`key_update`（改名；附带公钥时校验后只替换公钥；附带私钥时整体替换）、`key_delete`、`key_reveal`（解锁后返回私钥与 passphrase 供查看或复制）、`key_install`（用与打开终端相同的连接方式，在主机上执行 `authorize_command`，20 秒超时，失败时返回带退出码和 stderr 的 `key.install_failed`）。

快捷指令 `Snippet { id, name, command }` 不含秘密，锁定时可读写；`SnippetDraft::validate` 要求命令非空且不超过 16 KiB，名称为空时取命令的第一行。
应用通过 `snippet_list` / `snippet_create` / `snippet_update` / `snippet_delete` 管理快捷指令；发送到终端由界面调用 `session_write` 完成。

偏好设置分两条记录保存在 `preferences` 表，锁定时照常可读写：`Preferences`（键 `app`）随用户走，以后参与同步；`DevicePrefs`（键 `device`）只属于本机，不离开设备。记录中缺少的字段取默认值，因此旧记录在新增字段后仍可读取。`set_preferences` 先经 `validate()` 校验：

| 字段 | 默认值 | 校验 |
|---|---|---|
| `terminal.font_family` | 空（使用内置字体栈） | 去除首尾空白，最多 100 个字符，不含引号、逗号、分号、反斜杠、花括号和控制字符 |
| `terminal.font_size` | 13 | 8–32 |
| `terminal.line_height` | 1.15 | 1.0–2.0 |
| `terminal.scheme_light` / `scheme_dark` | `rove-light` / `rove-dark` | 1–40 个小写字母、数字或 `-`；配色目录在界面侧，未知 id 退回默认 |
| `terminal.cursor_style` / `cursor_blink` | `block` / 否 | `block`、`bar`、`underline` |
| `terminal.scrollback` | 10000 | 1000–100000 |
| `terminal.copy_on_select` / `right_click` / `bell` | 否 / `menu` / `visual` | `menu` 或 `paste`；`none` 或 `visual` |
| `appearance.theme` / `accent` | `system` / `blue` | `system`、`light`、`dark`；系统色板中的 8 种 |
| `appearance.ui_scale` / `language` | 100 / `auto` | 90–125；`auto`、`zh-CN`、`en` |
| `files.default_conflict` | `ask` | `ask`、`rename`、`overwrite`、`skip` |
| `files.show_hidden` / `confirm_delete` | 否 / 是 | |
| `status.interval_secs` | 3 | 2–30 |
| `DevicePrefs.window_material` | 是 | |
| `DevicePrefs.download_dir` | 空（系统下载目录） | 去除首尾空白，空或绝对路径，最多 1024 字节 |
| `DevicePrefs.max_transfers` | 3 | 1–8 |
| `ai.mode` | `ask_by_risk` | `ask_by_risk` / `ask_all` / `auto`，新 AI 会话的默认审批模式 |
| `ai.read_only` / `ai.dangerous` | 空 | 去除首尾空白并丢弃空项，每类最多 200 项、每项最多 100 个字符、不含控制字符；只读项只能是程序名（字母、数字、`-_.+`） |
| `DevicePrefs.agent.command` / `args` | 空（自动查找 Codex 的 ACP 适配器） | 去除首尾空白；命令与每个参数最多 1024 字节、不含控制字符，最多 32 个参数 |
| `DevicePrefs.connection` | 连接超时 15 秒、保活 30 秒、3 次 | 连接超时 5–120 秒，保活间隔 5–600 秒，允许丢失的保活 1–10 次 |
| `DevicePrefs.agent.auth` | `auto` | `auto`（有 API key 时用 key，否则用 Agent 自己的登录）/ `api_key` / `account` |

应用通过 `prefs_get` / `prefs_set` 和 `device_prefs_get` / `device_prefs_set` 命令读写偏好设置，后者同时把 `max_transfers` 应用到传输队列（启动时也会应用）；`fonts_monospace` 用 fontdb 列出本机已安装的等宽字体族（去重、按名称排序），供字体选择使用，移动端返回空列表。


## 同步

`src/sync.rs` 是 vault 一侧的同步接口，不涉及同步协议（见 [sync](sync.md)）：

- `sync_records()` 列出全部可同步记录（种类 `prefs`、`host`、`key`、`host_secret`、`forward`、`snippet`，内容为 JSON）：主机不含连接时间，含检测到的系统；凭据与私钥为明文 JSON，交给同步引擎密封后才离开本机。需要已解锁，否则凭据会看似被删除并把删除扩散到其他设备。
- `apply_sync(changes)` 写入来自其他设备的变更：先按父到子的顺序写入新增和修改，再按子到父删除；内容像本地编辑一样校验，引用的主机不存在时跳过对应的凭据与转发，未知种类忽略。密钥缺少私钥时保留本机已有的私钥。
- AI 的 API key：`set_ai_key(Some(key))` 用设备主密钥密封后存在 `preferences` 表的 `ai_key` 记录中（需已解锁；去除首尾空白，不能为空、含空白或控制字符，最多 4096 字节），`None` 删除；`ai_key()` 需已解锁，`has_ai_key()` 锁定时也可读。它只属于本机，不参与同步。
- `sync_local()` / `set_sync_local()` 读写本机的同步设置（后端位置、同步状态、同步密钥、存储密码），存在 `preferences` 表的 `sync` 记录中，不参与同步；同步密钥和存储密码（如 WebDAV 密码）用设备主密钥分别密封，锁定时读出为空。

## 表结构

`meta`、`hosts`、`host_secrets`（外键级联删除）、`known_hosts`、`forwards`（外键级联删除）、`preferences`、`ssh_keys`、`snippets`。主机的路线和检测到的系统分别以 JSON 存在 `hosts.route`、`hosts.os` 列；`os` 无法解析时按未检测处理。编辑主机不会清除 `os`。结构版本记录在 `PRAGMA user_version`，当前版本为 8；打开旧版本数据库时按版本依次迁移。
