# rove-tailscale

桌面 profile 通过固定版本的 [tailscale-rs](https://github.com/tailscale/tailscale-rs/tree/e88a60083ed2b9b2b8fd62733fe86ea887e72780) 启动用户态尾网设备。应用向 `TailnetService` 提供已解密的设备身份，读取连接状态和节点快照，并为 SSH 打开尾网 TCP 流。接口见 [tailscale 子系统](../../docs/subsystems/tailscale.md)。

`connect()` 启动后台设备并返回当前状态。调用者可订阅认证和节点变化。`disconnect()` 停止设备并清理快照。节点列表按名称（忽略大小写）和稳定 ID 排序，地址按 IPv4、IPv6 排列。设备名默认为 `RoveTerm`，旧配置中的空设备名沿用该默认值。

`ping_peer()` 在指定节点上执行一次 Disco 往返探测，结果包含可达状态、延迟和响应路径。调用需要正在运行的尾网连接。

## 插件配置

| `[plugin.tailscale]` 字段 | 默认值 | 范围 |
|---|---|---|
| `connect_timeout_ms` | 15000 | 100–120000 毫秒 |
| `peer_refresh_ms` | 2000 | 250–60000 毫秒 |

插件在加载时校验配置，服务注册随 `Disposer` 释放。
