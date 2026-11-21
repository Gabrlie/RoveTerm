# Tailscale

`rove-tailscale` 在桌面进程内运行用户态尾网设备。应用通过 `TailnetService` 读取状态和节点，通过 `Via::Custom` 为 [SSH](ssh.md) 打开尾网 TCP 流。设备生命周期和插件配置见 [crate README](../../crates/rove-tailscale/README.md)。

## 连接与授权

应用连接时在 [vault](vault.md) 中读取或生成设备身份。断开会保留身份和连接设置，注销会删除本机身份。启用自动连接时，应用解锁 vault 后恢复这台设备的连接。

`tailscale_status` 返回连接状态，`tailscale_snapshot` 同时返回节点列表。应用通过 `tailscale-status` 推送状态变化。状态包括未配置、已断开、启动中、需要登录、等待设备批准、正在运行和错误。需要浏览器登录时，`tailscale_open_auth_url` 打开当前设备提供的登录地址。

## SSH 路由

`HostRoute::Tailscale` 支持尾网 IP 和 MagicDNS。名称解析可以按已保存的稳定节点 ID 找回改名的设备。SSH 登录和 host key 校验继续由 SSH Provider 完成。节点的在线标志表示控制面在线。
