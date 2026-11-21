# 0007：通过系统 Tailscale 客户端发现设备

## 背景

RoveTerm 的 SSH、SFTP 和主机工具已经使用操作系统的网络连接。桌面端需要发现用户尾网中的设备，并将它们带入同一套主机、凭据和 host key 管理流程。

## 决定

- `rove-tailscale` 定义 `TailnetDiscovery`，CLI Provider 固定执行已安装客户端的 `status --json`，不经过 shell。超时和输出大小在配置中校验。
- 设备发现身份使用 `PeerStatus.ID`，不使用可变化的公钥或不唯一的显示名称。`Online` 只用于控制面状态展示。
- 设备选择复用普通主机表单，用户名和认证由用户确认。连接地址优先采用尾网 IP，MagicDNS 名称可选；主机记录与连接协议保持统一。
- Windows/macOS/Linux 复用客户端的 daemon 适配；macOS 从应用可执行文件启动 CLI 时设置 `TAILSCALE_BE_CLI=1`。移动 profile 不加载该插件。

## 取舍

LocalAPI 的 Status 已有稳定接口，但直接在 Rust 中适配三平台的 socket、管道和本机授权会增加维护成本。CLI 已提供这些连接适配，JSON 输入在 Provider 边界解析并校验。

Cloud API 是设备管理面，不提供本机尾网路由；tsnet 是 Go 嵌入式节点；Tailscale SSH 另有身份认证和浏览器再认证流程。这些能力不属于设备发现服务。

## 验证

伪 CLI 覆盖定位、路径空格、运行状态、读取权限、超时、输出限制、取消和 JSON 变化；设备逻辑覆盖 IPv4/IPv6、重名、ID 与公钥变化和刷新顺序。真实客户端验收核对发现结果、SSH 认证和 host key 行为。
