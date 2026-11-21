# tailscale

`rove-tailscale` 将本机 Tailscale 客户端作为只读设备发现 Provider。设备选择后的终端、文件和 AI 操作继续使用 [SSH](ssh.md)、[文件系统](fs.md)及其审批与审计路径。

## 角色与服务

| 角色 | 实现 |
|---|---|
| Definition | `TailnetDiscovery`、`TailnetSnapshot`、`TailnetPeer`、`TailscalePeerId` |
| Provider | `CliDiscovery`，调用本机客户端的 `status --json` |
| Consumer | 桌面端设备列表、现有主机创建与连接表单 |

`TailscalePlugin`（插件名 `tailscale`）注册 `dyn TailnetDiscovery`，返回可撤销的服务注册。可执行文件定位、进程与解析契约、配置范围见 [crate README](../../crates/rove-tailscale/README.md)。

## 快照

快照包含客户端状态、设备数组、捕获时间和可选的静态诊断。设备摘要只包含稳定 ID、名称、MagicDNS、Tailscale IP、系统及控制面在线标志；账户与认证数据留在系统客户端。

客户端状态分别表示未安装、无读取权限、服务不可用、尚无状态、需登录、待设备批准、已停止、启动中、运行中与未知状态。状态读取是一次性操作，页面负责显式刷新、请求顺序及过期结果标记。
