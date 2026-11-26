# Changelog / 更新日志

本文件以中英双语记录 RoveTerm 的版本变更，版本号遵循 [SemVer 2.0](https://semver.org/) 规范。
This file documents notable changes to RoveTerm in both Simplified Chinese and English, following [SemVer 2.0](https://semver.org/).

## [Unreleased]

## [0.3.0-beta.3] - 2026-11-27

### 中文更新日志

- **Android 应用内更新下载与自动安装**：支持在应用内流式下载已签名发布 APK（实时显示进度与已下载大小）、完成 SHA-256 校验后通过 `FileProvider` 直接唤起系统安装器；Android 8+ 首次缺少「安装未知应用」权限时自动引导开启并在返回后继续安装，失败或重试直接复用本地已校验缓存包；保留真实指向 GitHub Release 页面的「前往 GitHub 下载」手动兜底按钮并移除不可点击的假按钮标签。
- **跨平台按资产更新过滤与纯净更新日志**：桌面端与移动端检查更新时按 Release 实际包含的平台安装包筛选，未改动平台不再收到无意义的更新推送；服务端与客户端统一剥离自动生成的测试版横幅、下载表格 HTML 与尾部校验区，杜绝首次打开更新弹窗出现原始 HTML 源码的问题；移动端启动检查发现未跳过的新版本时自动弹出更新窗口，并精简冗余提示文案。
- **移动端控件比例、页面排版与返回栈精修**：重塑移动端按钮、分段控件、下拉选择框、开关与分组标题比例（区分 36px 常规操作、30px 行内/工具栏与 28px 分组标题轻质感胶囊按钮），消除高按钮撑高标题栏与顶部异常留白；日志查看窗口自动撑满屏幕剩余高度；移除终端工具栏与 2×7 快捷键栏之间不对称的分割线并统一 4px 间距；服务器状态面板网卡行保持单行显示并提升移动端卡片边距、字号与核心图可读性；从运维总管或终端侧栏进入设置后，返回手势现直接回到来源页面。

### English Changelog

- **In-App Android APK Updates & Auto-Install**: Stream-download signed release APKs directly inside RoveTerm with live progress and incremental SHA-256 verification, then launch Android's package installer via `FileProvider`. On Android 8+, guide users through granting the "Install unknown apps" permission and automatically resume installation on return, reusing the verified cached APK on retries. Replaced the non-interactive mode pill with a working **Download on GitHub** fallback pointing to the GitHub Release page.
- **Per-Platform Update Filtering & Clean Release Notes**: Update checks now filter GitHub Releases by platform-specific assets so platforms without changes are no longer prompted unnecessarily. Sanitized generated beta banners, download badge HTML tables, and checksum footers across both manifest and API fallback paths so raw HTML never leaks into release notes. Mobile startup checks now automatically open the update dialog once per unskipped version.
- **Mobile Control Proportions, Layout & Back-Stack Polish**: Harmonized mobile button tiers (36px primary/dialog, 30px inline/toolbar, 28px soft-accent section-heading pills), 32px segmented tracks, 38px Select options, and 40×24px switches without altering desktop sizing. Expanded the mobile Logs viewer to fill remaining viewport height, removed the uneven terminal dock/keybar divider for uniform 4px spacing, kept Status panel network rows single-line with improved mobile padding/typography, and restored the caller pane (such as Fleet Lead) when navigating back from compact Settings.

## [0.3.0-beta.2] - 2026-11-26

### 中文更新日志

- **移动端原生界面与统一比例体系**：手机端采用「主机 / 工具 / 设置」三标签底部导航、独立设置页与全屏表单页面，统一顶栏、列表行高、图标盒、控件与字号令牌（`--m-*`）；冷启动不再恢复桌面侧栏或旧分屏布局，打开终端工具面板或状态页前自动收起软键盘。
- **沉浸式单会话终端与 2×7 快捷键栏**：重做手机端单会话终端底栏与工具面板；默认提供 Termux 风格的固定 2×7 圆角等宽快捷键网格（取消横向滚动及多余右侧按钮），`CTRL` / `ALT` 改为单次触发并以底部细横条指示，支持自定义页行容量提示与旧布局自动迁移；彻底隐藏终端横向滚动条并收窄触屏纵向滚动条。
- **弹窗防穿透、同步与图标修复**：修复主机更多菜单点击遮罩时穿透触发底层主机连接的问题并移除冗余取消按钮，拦截对话框遮罩穿透事件；修复 WebDAV 同步页密码显示为明文的问题；替换 Android 自适应图标与 iOS 启动图标为 RoveTerm 专属终端图标。
- **Android 后台保活同步与真实版本号**：会话从表中移除时立即发出 `SessionClosed` 事件并取消 SSH 读循环，通知栏保活服务随即清除前台通知与唤醒锁，不再滞留残留会话计数；Android 构建现自动从根目录 `Cargo.toml` 同步真实 `versionName` 与单调递增 `versionCode`。

### English Changelog

- **Native Mobile Shell & Unified Visual Scale**: Adopted 3-tab bottom navigation (**Hosts / Tools / Settings**), a standalone Settings page, full-screen mobile forms, and unified phone typography, row height, icon tile, and control tokens (`--m-*`). Compact cold starts skip restoring desktop panels/multi-tab layouts and dismiss the soft keyboard before opening terminal overlays.
- **Immersive Single-Session Terminal & 2×7 Key Bar**: Redesigned the compact terminal dock and tools sheet; switched the virtual key bar to a fixed non-scrolling 2×7 Termux-style rounded grid (removing right-side chrome) with one-shot `CTRL` / `ALT` bottom-bar indicators, row-capacity hints in customization, and automatic migration from prior layouts. Hid terminal horizontal scrollbars and slimmed vertical touch scrollbars.
- **Overlay Click-Through, Sync & App Icon Fixes**: Kept compact context-menu backdrops mounted through taps so dismissing never clicks through to rows underneath, removed the redundant bottom Cancel button, and blocked dialog backdrop pass-through; masked WebDAV password inputs properly; replaced default launcher icons with RoveTerm adaptive Android and opaque iOS icons.
- **Android Keep-Alive Sync & Real APK Versioning**: Added an immediate `SessionClosed` live event and SSH reader cancellation so closing a terminal removes the Android foreground keep-alive notification and wake lock right away; Android Gradle builds now derive `versionName` and a monotonic `versionCode` directly from workspace `Cargo.toml`.

## [0.3.0-beta.1] - 2026-11-25

### 中文更新日志

- **稳定版与测试版双通道发布与更新**：新增「稳定版 / 测试版」更新通道，可在「设置 → 关于 → 更新通道」按需切换；测试版安装包默认订阅测试通道，支持直接接收带签名校验的应用内预发布更新并在正式版发布后平滑升级。
- **移动端安全基础与系统桥接**：接入 `tauri-plugin-rove-mobile` 原生桥接，支持 Android Keystore 与 iOS Keychain 主密钥托管、应用内生物识别 / 屏幕锁保护切换，以及原生安全区域与输入法遮挡高度适配。
- **移动端工作区、后台保活与终端触控**：小于 768px 屏幕自动平铺标签栏并提供底部导航与侧栏抽屉，平板及以上保留自由分屏；新增 Android 前台保活服务（支持通知展示待审批任务与一键断开）与 iOS 后台任务保活；新增可自定义主行与附加行的 `TerminalKeyBar`、长按菜单、单指滑动滚动与双指捏合调整字号。
- **移动端 SFTP 文件、代码编辑器与系统分享导出**：`FileBrowser` 适配窄屏双行列表、轻点编辑/操作、长按菜单与多选模式，支持通过系统选择器暂存上传；`TextEditor` 默认在窄屏开启自动换行并新增符号输入栏；传输面板与日志页支持原生分享与保存到设备，顶部菜单支持从系统文件选择器导入 SSH 配置。
- **移动端 AI 助手、运维任务与设置重构**：`SettingsDialog`、`KeysDialog` 与 `AuditDialog` 在窄屏采用列表/详情两级推入导航并联动 Android 返回键；AI 对话与总管输入框在触控模式下支持 Enter 换行与按钮/快捷键发送，移动端自动隐藏桌面专用本地能力（本地 PTY、本地 ACP、定时任务与外部 MCP 桥接）。

### English Changelog

- **Dual Stable & Beta Release Channels**: Added separate Stable and Beta update channels selectable under **Settings → About → Release channel**. Beta builds follow the Beta channel by default, receive signed in-app pre-release updates via per-tag manifests, and transition smoothly to Stable once the final release ships.
- **Mobile Security Foundation & Native Bridge**: Added `tauri-plugin-rove-mobile` providing Android Keystore and iOS Keychain master-key storage, biometric/device-credential vault protection, and native safe-area plus IME keyboard inset tracking.
- **Mobile Workspace, Background Keep-Alive & Touch Terminal**: Compact viewports (`< 768px`) flatten workspace tabs with bottom navigation and a slide-in host drawer while tablets keep full split panes. Added Android foreground service keep-alive (including pending approval counts and quick disconnect) and iOS background task keep-alive, plus a customizable `TerminalKeyBar`, long-press context menus, touch scrolling, and pinch-to-zoom font sizing.
- **Mobile SFTP, Code Editor & Native Share/Export**: Adapted `FileBrowser` with compact two-line rows, touch actions, 450 ms long-press menus, multi-select mode, and staged system file uploads; added narrow-screen default word wrap and a symbol bar to `TextEditor`; enabled native share sheets and export-to-device for downloads and logs, plus system-picker SSH config import.
- **Mobile AI Assistant, Fleet Ops & Settings Overhaul**: Converted `SettingsDialog`, `KeysDialog`, and `AuditDialog` to Master-Detail navigation with Android Back support on compact screens; updated touch composers so Enter inserts newlines; gated desktop-only capabilities (local PTY, local CLI ACP agents, schedules, and external MCP bridge) cleanly on mobile.

## [0.2.1] - 2026-11-24

### 中文更新日志

- 修复内置 Tailscale 节点在系统休眠唤醒、网络切换或直连探测失败后的断连与卡死问题：自动重连控制面并回退至可用的 DERP 中继，重连超过短宽限期后在侧栏显示状态提示。
- 服务器状态面板与底部状态条不再在连接中或断开后残留旧数据；本地终端现可直接读取并展示本机 CPU、内存、网卡与磁盘实时状态。
- 运维总管标签页现在与主页一样自动收起右侧和底部面板，同时保留下载中心入口。
- 修复应用内日志查看器中偶现的 ANSI 转义字符乱码，并更新飞牛 OS（fnOS）为官方品牌图标。

### English Changelog

- Fixed embedded Tailscale recovery after system sleep/wake, network changes, and failed direct probes: control sessions and DERP relays automatically re-establish, failed direct paths fall back to DERP immediately, and prolonged reconnects surface an amber indicator in the sidebar.
- The Status panel and terminal status bar now clear stale remote metrics while connecting or after disconnecting, and local terminals now display live CPU, memory, network, and disk telemetry for the local host.
- The Fleet Lead tab now suspends right and bottom panels just like Home while keeping the Downloads Center accessible.
- Stripped stray ANSI escape codes from log files and the in-app log viewer, and updated fnOS hosts to use the official brand icon.

## [0.2.0] - 2026-11-23

### 中文更新日志

- 运维总管按主机显示任务进度、报告和审批。可停止或重试单台主机的任务，也可在独立子代理面板查看对话和执行记录。
- 主机任务完成后，运维总管会读取报告并汇总结果。任务运行期间仍可向总管发送消息。
- AI 历史支持搜索、角色筛选和日期分组，可在 Markdown 对话与审计时间线之间切换。展开操作即可查看命令输出、文件内容或彩色差异。
- 对话更名会同步到已打开的面板和运维总管主页的最近列表。开发构建使用独立的数据目录和钥匙串服务。

### English Changelog

- Fleet Lead shows progress, reports, and approvals per host. Stop or retry work on an individual host, or open the sub-agent panel to inspect its conversation and actions.
- Fleet Lead summarizes host reports as tasks complete. You can keep sending messages while tasks run.
- Search AI history, filter by role, and browse by date. Switch between the rendered conversation and the audit timeline, and expand actions to see command output, file contents, or color-coded diffs.
- Renames update open AI panels and Fleet Home's recent list. Development builds use a separate data directory and keychain service.

## [0.1.0] - 2026-11-22

### 中文更新日志

#### 核心能力
- **跨平台智能终端基础**：基于 Tauri 2 + Rust 静态插件内核 + Svelte 5 + xterm.js 6（桌面端支持 WebGL 加速），支持本地 PTY / ConPTY、多标签与任意方向分屏、标签拖拽跨组不断连、重启自动恢复会话布局，以及返回主页自动收起边栏并在切回会话时自动恢复。
- **SSH 多路复用与端口转发**：同一主机多面板复用单条 SSH 连接；支持密码、私钥、密钥库、SSH Agent 认证，严格 host key 指纹校验，多级跳板机（支持 `ProxyJump` 导入）、HTTP / SOCKS5 代理，以及按主机保存的本地（`-L`）/ 远程（`-R`）/ 动态 SOCKS5（`-D`）端口转发。
- **内置 Tailscale 尾网引擎**：桌面端内置 `tailscale-rs`，支持浏览器授权 / Auth Key 连接、独立设备窗口、批量延迟测速、**直连 / DERP 中继区域**标识，以及已有直连主机一键转换为尾网路由。
- **SFTP 文件面板与工作区代码编辑器**：支持目录加载态提示、拖拽上传/下载传输队列与同名冲突处理；双击文本 / 配置文件直接以工作区标签页打开内置 CodeMirror 6 编辑器（行号、语法高亮、查找替换、远端冲突检测）；外部编辑直接调用操作系统原生默认打开方式并在保存后自动回传。
- **服务器实时监控与终端底栏微监控**：右侧状态面板展示系统信息、每核 CPU 曲线、内存 / Swap、网卡速率、磁盘占用与 Tailscale 路径标识；远端终端底部支持可定制迷你状态条；原生识别 **飞牛 OS（fnOS）** 与 **Proxmox VE（PVE）** 并显示专属品牌图标。
- **AI 助手、本机 ACP 智能体与运维总管**：支持 OpenAI / Anthropic / DeepSeek / OpenRouter / Ollama 等内置模型服务商与 Codex / Claude Code 等本机 ACP 智能体（含交互式登录弹窗），并可经 MCP 桥接开放给外部 Agent；支持多主机并发编排的运维总管、加密审计证据与定时巡检任务，全部操作遵循本地风险分级审批策略。
- **密钥库、端到端加密与同步**：设备主密钥托管于系统钥匙串，凭据以 XChaCha20-Poly1305 按字段加密；密钥库支持生成 / 导入与过滤式一键安装公钥到主机；支持本地文件夹与 WebDAV（兼容 OpenList / 阿里云盘 302 策略）端到端加密多设备同步。
- **应用内日志查看器与签名自动更新**：在设置中提供结构化、可按级别筛选并自由拖选复制的日志查看器；桌面端接入 `tauri-plugin-updater`（minisign 公钥验签 + 实时下载进度 + 草稿保护重启），并提供 GitHub Releases API 兜底检测与 Android `latest-android.json` 清单支持。

### English Changelog

#### Highlights
- **Cross-Platform Smart Terminal**: Built on Tauri 2, a Rust static-plugin kernel, Svelte 5, and xterm.js 6 (with WebGL acceleration on desktop). Includes local PTY / ConPTY, multi-tab and arbitrary split layouts, drag-and-drop tab transfers without dropping sessions, workspace restore, and auto-collapsing side/bottom panels on Home.
- **Multiplexed SSH & Port Forwarding**: Shares one SSH connection per host across terminals, SFTP, and telemetry; supports password, private key, Key Library, and SSH Agent auth, strict host-key verification, multi-hop jump hosts (`ProxyJump` import), HTTP/SOCKS5 proxies, and per-host local (`-L`), remote (`-R`), and dynamic SOCKS5 (`-D`) port forwards.
- **Embedded Tailscale Engine**: Built-in `tailscale-rs` on desktop with browser / auth-key login, dedicated peer dialog, batch latency probes, **Direct vs. DERP relay region** indicators, and one-click conversion of existing hosts to tailnet routing.
- **SFTP Browser & Workspace Code Editor**: Directory loading indicators, drag-and-drop transfer queue, and conflict prompts; double-click supported text/config files to open built-in CodeMirror 6 tabs alongside terminals, or edit externally via the OS native opener with conflict-checked auto-upload.
- **Live Server Telemetry & Terminal Mini Bar**: Per-core CPU charts, memory/swap, interface throughput, disk usage, and Tailscale path badges in the Status panel; customizable bottom status bar on remote terminals; native OS detection and brand icons for **fnOS** and **Proxmox VE**.
- **AI Assistant, Local ACP Agents & Fleet Ops**: Supports built-in model providers (OpenAI, Anthropic, DeepSeek, OpenRouter, Ollama, etc.) and local ACP agents (Codex, Claude Code, custom commands with interactive sign-in dialogs), plus an opt-in MCP server bridge. Fleet Lead orchestrates concurrent multi-host tasks with encrypted audit evidence, scheduled checks, and local risk-gated approvals.
- **Key Library, Per-Field Encryption & E2EE Sync**: OS Keychain master keys with XChaCha20-Poly1305 per-field encryption, redesigned Key Library with filtered one-click public-key installation, and encrypted multi-device sync via local folder or WebDAV (including OpenList / Aliyun Drive 302 compatibility).
- **In-App Log Viewer & Signed Updater**: Structured, filterable, freely selectable log viewer in Settings; desktop `tauri-plugin-updater` with minisign verification, live progress bars, and draft-safe restarts, backed by a GitHub Releases API fallback and `latest-android.json` manifest support.
