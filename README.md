# RoveTerm

轻量、跨平台的智能终端，支持 Windows、macOS、Linux 和 Android。基于 Tauri 2 和 Rust 核心构建，集成 SSH 远程连接、本地终端、SFTP 文件管理、端口转发、多标签与分屏、主机管理和加密凭据存储，后续将加入多设备端到端加密同步，以及支持 ACP / MCP 的 AI Agent。

## 功能

- **终端**：本地 shell（Windows 上使用 ConPTY）；多标签、任意方向分屏、拖拽调整分屏大小；命令面板；可选本机字体、字号和行高，内置 Nerd Font 符号字体以正确显示 Powerline 主题。
- **SSH**：支持密码、私钥、私钥文件、ssh-agent 认证；首次连接时确认 host key，指纹变化时拒绝连接；同一主机的多个面板共享一条连接；可经跳板机（多级）或 HTTP / SOCKS5 代理连接，支持从 `ProxyJump` 导入。
- **SFTP**：文件面板与终端并排分屏，支持文件浏览与传输。文本文件可在内置编辑器中保存回服务器，桌面端也可用外部编辑器打开并在保存后自动回传。
- **端口转发**：按主机保存本地（-L）、远程（-R）、动态 SOCKS5（-D）转发规则；随终端自动启动；显示运行状态和连接数。
- **主机管理**：首页按最近连接排序；分组、搜索与右键菜单；从 `~/.ssh/config` 导入；`user@host:port` 快速连接；连接时显示过程，失败后可重试或改用密码。
- **密钥库**：生成或导入 SSH 密钥，查看与复制公钥、私钥，一键把公钥安装到主机；主机直接从库中选择密钥。
- **工作区**：右侧服务器状态（CPU、内存、磁盘、网速）与快捷指令面板，底部文件面板；重启后恢复上次的标签与分屏。
- **凭据**：设备主密钥保存在系统钥匙串中，凭据用 XChaCha20-Poly1305 加密；启动时静默解锁，只有需要凭据时才请求授权。
- **快捷键**：应用快捷键在 macOS 上用 ⌘，在其他平台上用 Ctrl+Shift，普通的 Ctrl 组合键始终交给 shell。

## 开发

```sh
pnpm install
pnpm tauri dev                                    # 启动桌面开发版
cargo run -p devsshd -- --addr 127.0.0.1:2222     # 本地测试 SSH 服务器（demo / demo）
cargo test -p <crate>                             # 运行某个 crate 的测试
```

环境搭建见 [docs/development.md](docs/development.md)，架构见 [docs/architecture.md](docs/architecture.md)，开发约定见 [AGENTS.md](AGENTS.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [architecture.md](docs/architecture.md) | 内核与插件、事件、数据流、能力 seam |
| [security.md](docs/security.md) | 威胁模型与密钥层级 |
| [testing.md](docs/testing.md) | 测试策略与性能基线 |
| [subsystems/](docs/subsystems/README.md) | 各子系统参考 |
| [decisions/](docs/decisions/README.md) | 设计决策记录 |
