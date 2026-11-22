# Changelog / 更新日志

本文件以中英双语记录 RoveTerm 的版本变更，版本号遵循 [SemVer 2.0](https://semver.org/) 规范。
This file documents notable changes to RoveTerm in both Simplified Chinese and English, following [SemVer 2.0](https://semver.org/).

## [Unreleased]

### 中文更新日志

#### 新增功能
- **应用内更新与签名校验**：桌面端接入 `tauri-plugin-updater`（minisign 签名校验 + 实时下载进度 + 退出草稿保护重启），并提供 GitHub Releases API 兜底检查与 Android `latest-android.json` 清单支持；启动后延迟静默检查，状态栏以轻量角标提醒。
- **内置 Tailscale 尾网引擎**：内置 `tailscale-rs` 桌面端尾网连接、设备列表弹窗、批量延迟测速与直连 / DERP 中继标识，支持将已有直连主机一键转换为 Tailscale 路由。
- **AI 助手与本地 ACP 交互登录**：重构侧边栏输入器与分标签 AI 设置页，支持本地 ACP 智能体交互式登录弹窗（协议登录 / 内置终端登录 / API 密钥），完善运维总管持久任务与定时任务调度。
- **内置编辑器工作区标签与 SFTP 增强**：双击支持的文本/代码文件在工作区标签页中以 CodeMirror 编辑器打开；新增目录切换加载动画；外部编辑改用操作系统原生默认打开方式，无需手填路径与参数。
- **服务器状态与终端底部状态条**：支持在远端终端底部显示可定制的迷你状态条（CPU / 内存 / 网络 / 延迟 / Tailscale 直连或中转标识），并新增 fnOS（飞牛 OS）与 Proxmox VE 系统识别及专属图标。
- **应用内日志查看器与密钥库重构**：在设置中提供可筛选、可自由拖选复制的日志查看器；优化密钥库界面，自动过滤已绑定当前密钥的主机并支持安装后一键切换认证方式。
- **手动构建与 `gh` 发布工具**：新增 `scripts/release.mjs`，支持同步递增版本号、多平台合并 `latest.json`、生成 `latest-android.json` 以及 FlClash 风格点击图标下载表与双语发布说明。

#### 修复与改进
- 修复冷启动时 Rustls `CryptoProvider` 未预初始化与 Vault 解锁竞态导致的 Tailscale 启动失败，修复非 Home DERP 重连风暴与控制流断线退出问题。
- 修复 OpenList / 阿里云盘 WebDAV `302` 重定向读取 `space.json` 触发 `403` 的兼容性问题，并支持在同步设置中查看与修改已保存凭据。
- 返回主页时自动收起右侧面板与底部文件面板（保留下载中心可用），切回会话标签时自动恢复；移除半透明窗口模糊以提升跨平台文本清晰度。

### English Changelog

#### Added
- **In-App Signed Updater**: Integrated `tauri-plugin-updater` on desktop with minisign signature verification, live download progress, and draft-safe restart, plus a GitHub Releases API fallback and `latest-android.json` manifest support. Runs a quiet delayed check at startup with a subtle status bar badge.
- **Embedded Tailscale Engine**: Built-in `tailscale-rs` desktop tailnet engine with a dedicated device dialog, latency probing, direct vs. DERP relay indicators, and one-click conversion of existing hosts to Tailscale routing.
- **AI Sidebar & Local ACP Sign-In**: Redesigned sidebar composer and tabbed AI settings, added interactive sign-in dialogs for local ACP agents (protocol auth, embedded terminal login, or API key), and expanded durable Fleet Ops tasks and scheduled runs.
- **Workspace Editor Tabs & SFTP Improvements**: Double-clicking supported text/code files opens them as workspace tabs beside terminals; added directory loading indicators and simplified external editing to launch through the OS default opener without manual path/arg setup.
- **Server Status & Terminal Mini Status Bar**: Added a customizable compact status bar at the bottom of remote terminals (CPU, memory, network, latency, Tailscale direct/relay badge) plus OS detection and icons for fnOS and Proxmox VE.
- **In-App Log Viewer & Redesigned Key Library**: Added a structured, freely selectable log viewer in Settings; refined the Key Library UI, filtered already-bound hosts from public-key installation, and added optional one-click auth switching.
- **Manual Build & `gh` Release Helper**: Added `scripts/release.mjs` for synchronized version bumps, multi-machine `latest.json` merging, `latest-android.json` generation, and FlClash-style clickable icon download tables in bilingual release notes.

#### Fixed & Improved
- Fixed Tailscale startup failures caused by late Rustls `CryptoProvider` initialization and vault unlock races, eliminated non-home DERP reconnect storms, and added automatic control-stream reconnection.
- Fixed OpenList / Aliyun Drive WebDAV `302 -> 403` read failures via cross-origin redirect handling and added credential reveal/edit support in Sync settings.
- Automatically collapsed right and bottom workspace panels when returning to Home (keeping Downloads available) and restored them on returning to a tab; removed translucent window blur for sharper cross-platform rendering.

## [0.1.0] - 2026-11-01

### 中文更新日志
- 初始版本：基于 Tauri 2 + Rust 插件内核 + Svelte 5 + xterm.js 的跨平台智能终端，包含本地 PTY、SSH（连接复用 / 跳板机 / 代理 / host key 校验）、SFTP 文件与传输队列、端口转发、XChaCha20-Poly1305 加密凭据库、端到端加密多设备同步、服务器状态监控与 AI 审批框架。

### English Changelog
- Initial foundation: cross-platform smart terminal built on Tauri 2, a Rust static-plugin kernel, Svelte 5, and xterm.js, featuring local PTY, multiplexed SSH (jump hosts, proxies, host key verification), SFTP browser and transfer queue, port forwarding, XChaCha20-Poly1305 encrypted credential vault, E2EE multi-device sync, server status monitoring, and risk-gated AI approvals.
