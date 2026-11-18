# sync

`rove-sync` 在不可信的文件存储上实现端到端加密的多设备同步。设计理由见 [0005](../decisions/0005-sync.md)。

## 空间布局

| 路径 | 内容 |
|---|---|
| `space.json` | 格式版本、Argon2id 参数（`m` KiB、`t` 轮、`p` 线程）、16 字节盐、用同步密钥密封的校验串 |
| `devices/<设备 id>/<序号>.seg` | 一个分段：`{ device, seq, changes }` 经 XChaCha20-Poly1305 密封，AAD 为 `rove/sync/segment/v1:<设备>:<序号>`；序号 16 位补零，写后不改 |

创建空间时 `space.json` 只在不存在时写入（`SyncBackend::put_new`：文件夹先写临时文件再硬链接到位，不支持硬链接时独占创建；WebDAV 用 `If-None-Match: *`，已存在时服务器返回 412），写后再读回比对，不是自己写的就视为已存在；`create_or_join_space` 此时改为用同一密码加入，两台设备同时创建也只会得到一个空间。每轮开始前 `verify_space` 确认清单仍能用本机的同步密钥打开，否则返回 `SpaceChanged`，应用忘掉密钥并要求重新输入密码，不会写入用别的密钥密封的数据。

打开空间时先校验 KDF 参数（内存 8 MiB–1 GiB、1–10 轮、1–8 线程），拒绝恶意清单让设备耗尽内存或时间；新空间默认 64 MiB、3 轮、1 线程。

## 类型

| 类型 | 说明 |
|---|---|
| `SyncKey` | 32 字节同步密钥，drop 时清零；`seal` / `open` 使用经 HKDF 派生的子密钥，`fingerprint` 用另一把子密钥做 HMAC-SHA256 |
| `Hlc` / `Clock` | 混合逻辑时钟：毫秒时间、计数器、设备 id，按此顺序比较；`tick` 总是晚于已发出或已见过的时间戳 |
| `Change` | 一条变更：记录类型、记录 id、`Hlc`、内容（`None` 表示删除） |
| `SyncState` | 本设备 id、时钟、下一个分段序号、每台其他设备已读到的序号、每条记录的已知状态（指纹、时间戳、是否已删除） |
| `SyncBackend` | `list` / `get` / `put` / `delete` 整个文件；`FolderBackend` 用本机目录实现，写入先写临时文件再改名 |
| `WebDavBackend` | WebDAV 服务器上的目录：`PROPFIND`（Depth 1）列目录，`GET` / `PUT` / `DELETE` 读写删除，写入前用 `MKCOL` 逐级创建目录（405 视为已存在）；配置的目录本身先用 `PROPFIND`（Depth 0）确认，不存在时才创建，避免对服务器根目录发出被拒绝的 `MKCOL`；基本认证；要求 https（回环地址可用 http），HTTP 客户端由 `rove-http` 构造（rustls + ring 与系统证书，Android 上用内置根证书） |

## 一轮同步

1. **检测本地变更**：先从 Vault 和 Journal 各自已提交的导入记录恢复远端版本，再对 `store.records()` 的每条记录计算指纹，与 `known` 不同（或新出现）的记为修改；`known` 中有而当前没有的记为删除。这些变更都用当前时刻打上时间戳。
2. **拉取**：列出 `devices/` 下其他设备的目录，读取序号大于已读位置的分段；打不开或设备、序号与位置不符时整轮失败（`Rejected`）。
3. **合并**：按时间戳顺序比较远程变更，时间戳晚于本地已知状态的胜出，同一记录的本地待推送变更随之作废。设备在空间中的第一轮（从未读写过）对双方都有的记录让空间的版本胜出，因为本地副本只是刚被打上时间戳，并非刚被编辑（例如新设备的默认偏好）；只在本地存在的记录照常推送。胜出的变更一次交给 `store.apply`，由存储按自身的引用关系排序写入。
4. **推送**：剩余的本地变更写成一个新分段，序号取 `next_seq` 与已有分段最大序号加一中较大者。

5. **压缩**：本设备的分段超过 32 个时，把它作为最后修改者的每条记录（含删除）的最新变更写成一个新分段，再删除更早的分段；被其他设备覆盖的旧变更直接丢弃。新分段序号更大，正在读取的设备不会漏掉内容。

一轮在写入远程变更之后失败（推送分段出错）时，`SyncState` 仍记下已读位置、时钟和已写入的记录，但不记未发出的本地变更；调用方失败时也要保存状态。否则下一轮会把刚写入的远程内容当成本地修改、以新的时间戳重新推送，压过其他设备在此期间的更新。分段已发出后压缩失败，状态按已发出保存。

`SyncState.defer` 列出本机不可信的记录类型（`defer_to_space(kinds)` 设置，同时忘掉这些类型的已知状态）：下一轮重新读取所有分段（包括本设备自己的），这些类型的记录由空间的版本胜出，本地缺少的不会被当作删除推送；成功后清空。凭据库重置后用它恢复密码与私钥（见下文）。

## 应用中的同步

`src-tauri/src/commands/sync.rs` 分两步设置同步：

- `sync_connect(target, storage_password)` 连接存储：检查地址与 WebDAV 凭据（错误立即返回），把目标和密封后的 WebDAV 密码存入 vault（见 [vault](vault.md#同步)），返回该位置是否已有空间 `{ exists }`。重新连接本机已在同步的同一位置时沿用原有的设备 id、同步状态与密钥，避免本地记录被当成新变更覆盖其他设备更新的修改；换位置则生成新的 UUIDv7 设备 id。
- `sync_probe()` 对已连接的存储再次查看是否已有空间。
- `sync_setup(password)` 在已连接的存储上加入已有空间，没有空间时创建（密码至少 8 个字符，新空间从新的同步状态开始；其他设备恰好同时创建时改为加入），跑第一轮后存入同步密钥。
- 每轮（`sync_setup` 与 `sync_now`）先 `verify_space`，结束后无论成败都保存同步状态；空间被替换（`sync.space_changed`）时清除本机的同步密钥。
- 偏好不再作为一条整体记录同步（"后写者胜"时，一台设备升级后重新序列化的偏好会覆盖另一台的服务商列表），而是按节（`pref`）与按服务商（`ai_provider`）分条同步，见 [vault](vault.md#同步)。
- 同步的记录除 vault 的各类外，还有 AI 对话（`ai_conversation`，`SyncStore` 组合 `VaultStore` 与 Journal 上的 `ConversationStore`）：最近开始的 100 段、90 天内的对话，内容为该会话的 `ai/*` 记录（`rove_ai::audit::conversation_body`：按时间排序，单个文本截到 2 KiB，整段不超过 64 KiB，超出时较早的记录让位，开始与标题记录始终保留）。收到时按时间与类型计数，把本机没有的记录以原时间导入 Journal（`missing_records`、`JournalStore::import`），因此两台设备都继续过的对话会合并。对话离开同步范围（超出 100 段或 90 天）表现为删除，不会删除其他设备上的对话；用户删除对话则以墓碑传递：内容为 `{"deleted": true}` 的同 id 记录（90 天内的 `ai/conversation_deleted` 记录生成），收到时删除本机该对话的记录并留下同样的标记，此后其他设备送来的旧内容不再导入。会话 id、记录类型和大小在边界校验。
- `vault_reset_secrets` 重置凭据后调用 `after_secrets_reset`：同步状态对 `rove_vault::SYNC_SECRET_KINDS`（`host_secret`、`key`、`ai_key`）`defer_to_space`，并清除随设备密钥失效的同步密钥与存储密码；重新输入同步密码后的第一轮从空间恢复这些凭据，而不是把它们的缺失同步给其他设备。

`SyncTarget` 为 `Folder { path }`（绝对路径）或 `WebDav { url, username }`，WebDAV 密码不进入目标的 JSON。`sync_status` 报告 `configured`（已选存储）、`connected`（文件夹，或本机有 WebDAV 密码）和 `ready`（还有同步密钥）。`sync_now` 运行一轮，`sync_disable` 清除本机的同步设置。各轮由 `AppState.sync_lock` 串行执行，需要 vault 已解锁；本机没有同步密钥时 `sync_now` 返回 `sync.needs_password`。错误码为 `sync.*`（`wrong_password`、`no_space`、`space_exists`、`space_changed`、`rejected`、`backend`、`format`、`crypto`、`unauthorized`、`store`、`not_configured`、`needs_password`、`weak_password`）。
