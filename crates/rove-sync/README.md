# rove-sync

端到端加密的多设备同步。每台设备只向共享空间中自己的目录追加密封分段，设备之间按记录用 HLC 合并（后写者胜）。设计理由见 [0005](../../docs/decisions/0005-sync.md)，参考见 [docs/subsystems/sync.md](../../docs/subsystems/sync.md)。

## 契约

- 应用端的 `RecordStore` 在每个数据库的导入事务中保存远端版本和实际入库数据的指纹。重试首先恢复这些版本，两库都写完才推进读取位置。

- `create_space` / `join_space` 在后端创建或打开空间，返回同步密钥；密码错误时返回 `WrongPassword`。Argon2id 派生在阻塞线程上执行。创建只在清单不存在时写入（`put_new`）并读回比对，已存在（包括同时创建）返回 `SpaceExists`；`create_or_join_space` 此时改为加入。`verify_space` 确认清单仍能用给定密钥打开，否则返回 `SpaceChanged`。
- `sync(backend, key, state, store)` 执行一轮：检测本地变更、拉取并合并其他设备的新分段、把剩余的本地变更写成一个新分段（设备的第一轮里，双方都有的记录以空间为准）；本设备分段超过 `COMPACT_AFTER`（32）个时把历史压缩成一个。在写入远程变更之后失败时，`state` 只记下已读、已写入的部分（不含未发出的本地变更），调用方失败时也要保存它。
- `RecordStore` 由调用方实现：`records()` 返回所有可同步记录，`apply()` 一次收到本轮合并胜出的全部远程变更（按时间戳排序，`body` 为 `None` 时删除），可以自行调整顺序以满足引用关系（先父后子）。`apply` 之后 `records()` 应返回相同的内容，否则下一轮会把它当成本地修改再推送一次。
- `SyncState` 不含秘密（指纹以同步密钥为键），由调用方在两轮之间保存。`defer_to_space(kinds)` 让下一轮从所有分段（含本设备的）恢复这些类型的记录，空间的版本胜出，本地缺少的不当作删除。
- 后端路径由字母、数字、`.`、`_`、`-` 组成，不允许 `.`、`..`；单个文件最大 16 MiB。
- 后端的 `put_new` 只在文件不存在时写入：`FolderBackend` 用硬链接（不支持时独占创建），`WebDavBackend` 发送 `If-None-Match: *`。
- 后端：`FolderBackend`（本机目录）与 `WebDavBackend`（基本认证；必须是 https，回环地址可用 http；地址不能带凭据、查询或片段；HTTP 401 返回 `Unauthorized`，HTTP 403 返回 `Forbidden`，其余网络/TLS/HTTP 异常展开原因并附带方法与状态码返回 `Backend`；HTTP 客户端来自 `rove-http`）。

## 局限

- 后端可以删除或扣留分段，无法伪造或篡改。
- 本地变更以同步时刻为时间戳：两次同步之间的修改与同时段的远程修改按同步时间比较。
