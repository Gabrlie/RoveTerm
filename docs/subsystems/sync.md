# sync

`rove-sync` 在不可信的文件存储上实现端到端加密的多设备同步。设计理由见 [0005](../decisions/0005-sync.md)。

## 空间布局

| 路径 | 内容 |
|---|---|
| `space.json` | 格式版本、Argon2id 参数（`m` KiB、`t` 轮、`p` 线程）、16 字节盐、用同步密钥密封的校验串 |
| `devices/<设备 id>/<序号>.seg` | 一个分段：`{ device, seq, changes }` 经 XChaCha20-Poly1305 密封，AAD 为 `rove/sync/segment/v1:<设备>:<序号>`；序号 16 位补零，写后不改 |

打开空间时先校验 KDF 参数（内存 8 MiB–1 GiB、1–10 轮、1–8 线程），拒绝恶意清单让设备耗尽内存或时间；新空间默认 64 MiB、3 轮、1 线程。

## 类型

| 类型 | 说明 |
|---|---|
| `SyncKey` | 32 字节同步密钥，drop 时清零；`seal` / `open` 使用经 HKDF 派生的子密钥，`fingerprint` 用另一把子密钥做 HMAC-SHA256 |
| `Hlc` / `Clock` | 混合逻辑时钟：毫秒时间、计数器、设备 id，按此顺序比较；`tick` 总是晚于已发出或已见过的时间戳 |
| `Change` | 一条变更：记录类型、记录 id、`Hlc`、内容（`None` 表示删除） |
| `SyncState` | 本设备 id、时钟、下一个分段序号、每台其他设备已读到的序号、每条记录的已知状态（指纹、时间戳、是否已删除） |
| `SyncBackend` | `list` / `get` / `put` / `delete` 整个文件；`FolderBackend` 用本机目录实现，写入先写临时文件再改名 |

## 一轮同步

1. **检测本地变更**：对 `store.records()` 的每条记录计算指纹，与 `known` 不同（或新出现）的记为修改；`known` 中有而当前没有的记为删除。这些变更都用当前时刻打上时间戳。
2. **拉取**：列出 `devices/` 下其他设备的目录，读取序号大于已读位置的分段；打不开或设备、序号与位置不符时整轮失败（`Rejected`）。
3. **合并**：按时间戳顺序比较远程变更，时间戳晚于本地已知状态的胜出，同一记录的本地待推送变更随之作废；胜出的变更一次交给 `store.apply`，由存储按自身的引用关系排序写入。
4. **推送**：剩余的本地变更写成一个新分段，序号取 `next_seq` 与已有分段最大序号加一中较大者。

5. **压缩**：本设备的分段超过 32 个时，把它作为最后修改者的每条记录（含删除）的最新变更写成一个新分段，再删除更早的分段；被其他设备覆盖的旧变更直接丢弃。新分段序号更大，正在读取的设备不会漏掉内容。

失败的一轮不改变 `SyncState`；已经写入 `store` 的远程变更在下一轮会再次写入，结果相同。

## 应用中的同步

`src-tauri/src/commands/sync.rs` 暴露 `sync_status`、`sync_setup(target, password, create)`、`sync_now`、`sync_disable`。`SyncTarget` 目前只有 `Folder { path }`（绝对路径）。`sync_setup` 创建或加入空间（创建时密码至少 8 个字符），为本设备生成 UUIDv7 设备 id，跑第一轮后把目标、状态和同步密钥存入 vault（见 [vault](vault.md#同步)）。各轮由 `AppState.sync_lock` 串行执行，需要 vault 已解锁；本机没有同步密钥时 `sync_now` 返回 `sync.needs_password`。错误码为 `sync.*`（`wrong_password`、`no_space`、`space_exists`、`rejected`、`backend`、`format`、`crypto`、`store`、`not_configured`、`needs_password`、`weak_password`）。
