# rove-sync

端到端加密的多设备同步。每台设备只向共享空间中自己的目录追加密封分段，设备之间按记录用 HLC 合并（后写者胜）。设计理由见 [0005](../../docs/decisions/0005-sync.md)，参考见 [docs/subsystems/sync.md](../../docs/subsystems/sync.md)。

## 契约

- `create_space` / `join_space` 在后端创建或打开空间，返回同步密钥；密码错误时返回 `WrongPassword`。Argon2id 派生在阻塞线程上执行。
- `sync(backend, key, state, store)` 执行一轮：检测本地变更、拉取并合并其他设备的新分段、把剩余的本地变更写成一个新分段；本设备分段超过 `COMPACT_AFTER`（32）个时把历史压缩成一个。只有整轮成功时才更新 `state`。
- `RecordStore` 由调用方实现：`records()` 返回所有可同步记录，`apply()` 一次收到本轮合并胜出的全部远程变更（按时间戳排序，`body` 为 `None` 时删除），可以自行调整顺序以满足引用关系（先父后子）。`apply` 之后 `records()` 应返回相同的内容，否则下一轮会把它当成本地修改再推送一次。
- `SyncState` 不含秘密（指纹以同步密钥为键），由调用方在两轮之间保存。
- 后端路径由字母、数字、`.`、`_`、`-` 组成，不允许 `.`、`..`；单个文件最大 16 MiB。
- 后端：`FolderBackend`（本机目录）与 `WebDavBackend`（基本认证；必须是 https，回环地址可用 http；地址不能带凭据、查询或片段；凭据被拒绝时返回 `Unauthorized`）。

## 局限

- 后端可以删除或扣留分段，无法伪造或篡改。
- 本地变更以同步时刻为时间戳：两次同步之间的修改与同时段的远程修改按同步时间比较。
