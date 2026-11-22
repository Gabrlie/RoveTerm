# vault

`rove-vault` 存储主机、凭据、known_hosts 和端口转发规则，用设备主密钥按记录加密凭据，并定义 `KeyCache` seam。

设备的安装 ID 单独存储在本机 Vault 中，切换同步空间后仍可识别定时任务归属。

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
| `HostRoute` | `Direct`（默认）、`Jump { host }`（经另一台已保存主机跳转）、`Proxy { protocol, host, port }`（`http` 或 `socks5`，无认证）、`Tailscale { peer_id }`（可选稳定节点 ID） |
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
| `DevicePrefs.download_dir` | 空（系统下载目录） | 去除首尾空白，空或绝对路径，最多 1024 字节 |
| `DevicePrefs.max_transfers` | 3 | 1–8 |
| `DevicePrefs.external_editor` | `program`、`args` 均空（禁用） | 只属于本机；见[本机编辑器偏好](#本机编辑器偏好) |
| `ai.mode` | `ask_by_risk` | `ask_by_risk` / `ask_all` / `auto`，新 AI 会话的默认审批模式 |
| `ai.max_steps` | 40 | 1–200，内置助手一个回合内最多的模型请求次数 |
| `ai.request_timeout_secs` | 180 | 30–1800，内置助手等待回复开始及两段之间的最长时间 |
| `ai.reviewer` | `model` | `model` / `chosen` / `jev`，自动模式下普通修改的判断者；旧值 `off` 读作 `model` |
| `ai.schedules` | 空 | 总管的定时任务，最多 50 个：id 1–40 个小写字母、数字或 `-` 且不重复；名称 1–60 个字符；指令 1–8000 字节；`repeat` 为 `once {at}`、`hourly {minute}`、`daily {hour, minute}`、`weekly {weekday 0–6 自周一, hour, minute}`；`allow_changes`、`catch_up`（默认是）、`enabled`（默认是）、`created_at` |
| `ai.review_model` | 空 | `chosen` 判断者所用的 `{provider, model}`，须为已添加服务商的模型，选 `chosen` 时必填 |
| `ai.review_confidence` | 80 | 50–99（百分比），审阅者放行所需的置信度 |
| `ai.read_only` / `ai.dangerous` | 空 | 去除首尾空白并丢弃空项，每类最多 200 项、每项最多 100 个字符、不含控制字符；只读项只能是程序名（字母、数字、`-_.+`） |
| `ai.providers` | 空 | 内置助手的模型服务商，最多 20 个：id 为 1–40 个小写字母、数字或 `-` 且不重复；名称 1–60 个字符；格式 `chat` / `responses` / `anthropic`；地址去除首尾空白与末尾 `/`，须为 http(s)、不含凭据和查询，最多 500 字节；模型 1–200 字节（新对话的默认模型）；`models` 为对话中可切换的其他模型，最多 32 个，去除首尾空白、空项、重复项和与默认相同的项；`max_tokens` 为空或 1–1000000；`contexts` 为各模型的上下文窗口（来自模型列表，只保留该服务商的模型），`context_window` 为列表未给出时的窗口，均为 1000–100000000 token；`effort` 为 `minimal` / `low` / `medium` / `high` / `xhigh` / `max`，默认 `medium`，旧版本留空的读作 `medium`。随用户同步，不含 key |
| `ai.default_agent` | 空 | 新对话默认的 Agent：`builtin:<服务商 id>` 或 `acp:<智能体 id>` |
| `DevicePrefs.agents` | 空 | `disabled`：关闭的已检测智能体 id（最多 50）；`custom`：按命令添加的智能体（最多 20，id 为 `custom-` 加小写 slug，名称 1–60 个字符，命令非空，命令与参数最多 1024 字节、不含控制字符，最多 32 个参数）；`auth`：`auto` / `api_key` / `account`；`expose`：是否允许用户自己运行的智能体经 MCP 使用已保存的主机（默认关闭）。旧记录中的单个 `agent` 命令在读取时移入 `custom`（id `custom-1`），之后不再写出 |

应用通过 `prefs_get` / `prefs_set` 和 `device_prefs_get` / `device_prefs_set` 命令读写偏好设置，后者同时把 `max_transfers` 应用到传输队列（启动时也会应用）；`fonts_monospace` 用 fontdb 列出本机已安装的等宽字体族（去重、按名称排序），供字体选择使用，移动端返回空列表。

### 本机编辑器偏好

`ExternalEditorPrefs { program, args }` 存在 `DevicePrefs.external_editor` 中，不参与同步。旧记录缺少对象或对象中的字段时用空默认值。`validate()` 去除 `program` 首尾空白，要求非空时为当前平台的绝对本机路径（最多 4096 字节），只检查语法，不查询文件是否存在或可执行。Windows 只接受 `.exe`（不区分大小写）；macOS 的 `.app` 路径只在 `args` 为空时接受，应用通过 `/usr/bin/open -a` 打开它。清空配置须同时清空程序与参数；空程序配非空参数返回 `VaultError::Invalid`。

`args` 最多 32 项，每项最多 4096 字节，总共最多 32768 字节；路径和参数都拒绝控制字符。参数保留空字符串和首尾空格，应用逐项作为 argv 传入，不拼接 shell、不展开 shell 语法；`{file}` 的替换由启动编辑器的应用层完成，Vault 原样保存。

## 同步

主机备注（`notes.rs`，表 `host_notes`，结构版本 10）：每台主机一段，最多 16 KiB、除换行与制表符外不含控制字符，空文本即删除，随主机删除；以单独的 `host_notes` 记录同步（不随 `host` 记录），AI 更新备注不会覆盖用户对主机的修改。

AI key 被删除时写日志（只记 id，不记 key）：用户在设置中移除（`AI key removed by the user`）、其他设备经同步删除（`AI key removed by another device through sync`），以及重置凭据库时（`resetting the vault`），便于追查 key 的丢失来源。

`src/sync.rs` 是 vault 一侧的同步接口，不涉及同步协议（见 [sync](sync.md)）：

- `sync_records()` 列出全部可同步记录（种类 `host_notes`（主机备注）、`pref`（偏好按节：appearance、terminal、files、status、ai，ai 节不含服务商）、`ai_provider`（每个服务商一条）、`host`、`key`、`host_secret`、`forward`、`snippet`、`ai_key`；偏好按节同步，一台设备改了某一节不会覆盖其他节，也不会带走另一台的服务商；旧版本的整体记录 `prefs` 及其删除被忽略，旧版本也忽略新的种类；内容为 JSON）：主机不含连接时间，含检测到的系统；凭据与私钥为明文 JSON，交给同步引擎密封后才离开本机。需要已解锁，否则凭据会看似被删除并把删除扩散到其他设备。
- `apply_sync(changes)` 写入来自其他设备的变更：先按父到子的顺序写入新增和修改，再按子到父删除；内容像本地编辑一样校验，引用的主机不存在时跳过对应的凭据与转发，未知种类忽略。密钥缺少私钥时保留本机已有的私钥。
- AI 的 API key 按 Agent 分别保存：`set_ai_key(id, Some(key))` 用设备主密钥密封后存在 `preferences` 表的 `ai_key:<id>` 记录中，`id` 为 `builtin:<服务商 id>` 或 `acp:<智能体 id>`，AAD 绑定该 id，挪到别的记录下无法打开（需已解锁；key 去除首尾空白，不能为空、含空白或控制字符，最多 4096 字节），`None` 删除；`ai_key(id)` 需已解锁，`ai_key_ids()` 锁定时也可读。schema 9 把旧的单个 key 改名为 `ai_key:acp:codex`，它仍以旧的 AAD 打开，且只有这条记录可以。key 作为 `ai_key` 记录随端到端加密的同步分段同步（每台设备再用自己的设备密钥密封）；`reset_secrets()` 会一并删除这些记录。应用在重置后让同步从空间恢复密码与私钥（见 [sync](sync.md#应用中的同步)）。
- `sync_local()` / `set_sync_local()` 读写本机的同步设置（后端位置、同步状态、同步密钥、存储密码），存在 `preferences` 表的 `sync` 记录中，不参与同步；同步密钥和存储密码（如 WebDAV 密码）用设备主密钥分别密封，锁定时读出为空。

## 表结构

`meta`、`hosts`、`host_secrets`（外键级联删除）、`known_hosts`、`forwards`（外键级联删除）、`preferences`、`ssh_keys`、`snippets`。主机的路线和检测到的系统分别以 JSON 存在 `hosts.route`、`hosts.os` 列；`os` 无法解析时按未检测处理。编辑主机不会清除 `os`。结构版本记录在 `PRAGMA user_version`，当前版本为 8；打开旧版本数据库时按版本依次迁移。

Tailscale 设备身份密封在本机 `preferences` 的 `tailscale_state` 记录，AAD 为 `rove/vault/tailscale-state/v1`。记录存在性在锁定时可读，读写密钥需要解锁。重置凭据会清除身份并将当前设备标记为未配置。
