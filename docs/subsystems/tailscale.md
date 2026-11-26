# Tailscale

`rove-tailscale` 在桌面和移动端进程内运行用户态尾网设备，不依赖系统 VPN 接口或 TUN 权限。应用通过 `TailnetService` 读取状态和节点，通过 `Via::Custom` 为 [SSH](ssh.md) 打开尾网 TCP 流。设备生命周期和插件配置见 [crate README](../../crates/rove-tailscale/README.md)。

## 连接与授权

应用连接时在 [vault](vault.md) 中读取或生成设备身份。断开会保留身份和连接设置，注销会删除本机身份。启用自动连接时，应用解锁 vault 后恢复这台设备的连接，恢复的 SSH 标签页会等待解锁和尾网启动。控制流中断时运行时在后台重连。重连持续 6 秒后，状态从运行中切换为启动中。控制流恢复且授权有效后，设备刷新节点并恢复运行状态。休眠唤醒或控制流长时间中断会触发设备重建，并重新建立 WireGuard 会话。在无平台网络监听器的移动端，宿主通过 `update_network()` 推送网络接口快照以处理切网与局域网直连，并在回到前台时通过 `notify_foreground()` 探测控制连接或唤醒挂起的浏览器授权轮询。

`tailscale_status` 返回连接状态，`tailscale_snapshot` 同时返回节点列表。应用通过 `tailscale-status` 推送状态变化。状态包括未配置、已断开、启动中、需要登录、等待设备批准、正在运行和错误。需要浏览器登录时，`tailscale_open_auth_url` 打开当前设备提供的登录地址。等待管理员批准期间，设备保持轮询，批准后自动进入运行状态。

## SSH 路由

`HostRoute::Tailscale` 支持尾网 IP 和 MagicDNS。名称解析可以按已保存的稳定节点 ID 找回改名的设备。SSH 登录和 host key 校验继续由 SSH Provider 完成。节点的在线标志表示控制面在线。当前探测窗口内所有直连端点均无响应时，数据路径回退至 DERP 中继。`server_stats` 可为已保存的尾网主机返回 `tailscale_path`，从当前路由读取直连端点或 DERP 区域，无需发包测速。

## 节点延迟

`tailscale_ping_peer` 接收稳定节点 ID（`peerId`），在 4 秒内通过 Disco 探测直连和 DERP 中继。中继先响应时，最多再等待 450 毫秒以优先采用直连。结果包含本次探测的可达状态、往返延迟（毫秒）和路径。直连路径返回 UDP 端点，中继路径返回 DERP 区域。未收到响应时，路径为 `unknown`，延迟为 `null`。本机节点的 `home_derp` 在选定首选中继后提供区域信息。
