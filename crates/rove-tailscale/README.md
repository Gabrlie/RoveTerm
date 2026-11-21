# rove-tailscale

通过已安装的 Tailscale 桌面客户端读取本机尾网状态与设备摘要。`TailnetDiscovery` 是 Definition，`CliDiscovery` 是 Provider，应用设备浏览器是 Consumer。参考见 [tailscale 子系统](../../docs/subsystems/tailscale.md)。

## 契约

- 每次 `snapshot()` 只运行固定的 `status --json`，不经 shell；程序退出、超时、超量输出或调用取消后释放本次进程。
- 自动查找当前 PATH 中的客户端；macOS 另外支持官方应用中的 CLI。`program` 可指定绝对路径。子进程只继承系统路径与用户目录等环境，设置 `TAILSCALE_BE_CLI=1`，不继承模型或云服务凭据变量。
- 未安装、无读取权限、服务不可用以 `TailnetState` 返回，客户端缺失不影响插件注册。超时、输出超限、无效 JSON 和无效必需字段返回 `TailscaleError`，错误不带原始 stdout、stderr、账户列表或登录 URL。
- `TailscalePeerId` 来自状态字段 `ID`，公钥 map key 变化不改变身份；同名设备按各自 ID 区分。IP 去重，IPv4 排在 IPv6 前；MagicDNS 域名移除末尾的点。
- `online` 只表示 Tailscale 控制面在线。Consumer 使用选定地址进入现有 SSH/SFTP 流程，仍须核对主机密钥并进行 SSH 认证。
- 接受未知 JSON 字段；设备缺少有效 ID、地址，或具有重复 ID 时拒绝整个新快照，调用方可继续显示明确标记过期的旧结果。

## 配置（`[plugin.tailscale]`）

| 字段 | 默认值 | 范围 |
|---|---|---|
| `program` | 自动查找 | 绝对路径，无控制字符；文件存在与可执行性在读取时检查 |
| `timeout_ms` | 5000 | 100–60000 毫秒 |
| `max_output_bytes` | 8388608 | 1024–67108864 字节，stdout 与 stderr 共用 |

配置在插件加载时校验。注册返回的 `Disposer` 在插件卸载时撤销服务。
