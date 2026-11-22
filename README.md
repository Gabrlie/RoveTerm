<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="104" height="104" alt="RoveTerm Logo" />

# RoveTerm

**新一代跨平台智能终端 —— 让多主机 SSH、尾网直连、SFTP、实时监控与 AI 运维变得简单高效**

[![GitHub Stars](https://badgen.net/github/stars/Gabrlie/RoveTerm)](https://github.com/Gabrlie/RoveTerm/stargazers)
[![GitHub Release](https://badgen.net/github/release/Gabrlie/RoveTerm)](https://github.com/Gabrlie/RoveTerm/releases)
[![Rust](https://img.shields.io/badge/Rust-1.92%2B-F97316?logo=rust&logoColor=white)](Cargo.toml)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](src-tauri/tauri.conf.json)
[![Svelte 5](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)](package.json)
[![License: MIT](https://img.shields.io/badge/License-MIT-16A34A)](LICENSE)

[🇨🇳 简体中文](README.md) • [🇺🇸 English](README.en.md) • [📝 更新日志](CHANGELOG.md) • [📖 开发文档](docs/development.md) • [🏗️ 架构设计](docs/architecture.md) • [🐛 问题反馈](https://github.com/Gabrlie/RoveTerm/issues)

</div>

---

## 📋 项目简介

**RoveTerm** 是一款基于 **Rust 插件内核 + Tauri 2 + Svelte 5** 构建的轻量级跨平台智能终端，覆盖 Windows、macOS、Linux（桌面端）与 Android（持续适配中）。

它把日常运维最常来回切换的工具收进同一个原生窗口：**SSH 多路复用、内置 Tailscale 尾网、SFTP 文件浏览与工作区代码编辑器、端口转发、实时服务器监控、加密凭据库与多设备同步，以及受本地风险策略约束的 AI 助手与运维总管**。

---

## ✨ 核心特性

- 🚀 **轻量原生架构** - 基于 Rust 核心插件 + Tauri 2 + Svelte 5，无需重型 Electron 运行时，桌面端支持 WebGL 终端加速
- 🪟 **VSCode 风格工作区** - 多标签、任意方向分屏、拖拽跨组移动不中断会话、重启自动恢复布局；返回主页自动收起边栏并在切回会话时恢复
- 🔌 **高效 SSH 连接池** - 同一主机的终端、SFTP 与状态监控自动复用单条连接；支持多级跳板机（`ProxyJump` 导入）、HTTP / SOCKS5 代理与严格 host key 指纹保护
- 🌐 **内置 Tailscale 尾网引擎** - 桌面端内置 `tailscale-rs`，无需额外安装守护进程；提供独立设备窗口、批量测速、**直连 / DERP 中继区域**标识，以及已有主机一键转为尾网路由
- 📂 **SFTP 与工作区代码编辑器** - 支持目录加载态、拖拽传输队列与冲突处理；双击文本 / 配置文件直接以工作区标签页打开内置 CodeMirror 编辑器，外部编辑调用系统原生应用并在保存后自动回传
- 🔀 **端口转发管理** - 按主机持久化本地（`-L`）、远程（`-R`）与动态 SOCKS5（`-D`）转发规则，支持随终端自动启动并实时查看活跃连接数
- 📊 **实时服务器状态与底栏微监控** - 右侧状态面板展示每核 CPU 折线、内存 / Swap、多网卡速率与磁盘占用；远端终端底部可开启定制迷你状态条，并原生识别 **飞牛 OS（fnOS）** 与 **Proxmox VE（PVE）** 专属图标
- 🤖 **AI 助手与本机 ACP 智能体** - 支持 OpenAI / Anthropic / DeepSeek / OpenRouter / Ollama 等内置模型，以及本机 Codex / Claude Code 等 ACP 智能体（内置交互式登录弹窗），也可经 MCP 暴露给外部 Agent
- 🧭 **运维总管与定时巡检** - 单会话并发编排多台主机任务、持久化检查点与加密审计证据，支持定时任务巡检、失败重试与严格风险分级审批（危险操作必须人工确认）
- 🔐 **字段级加密与多设备同步** - 设备主密钥存于系统钥匙串，凭据采用 XChaCha20-Poly1305 加密；支持密钥库一键生成 / 过滤式安装公钥，以及端到端加密多设备同步（文件夹 / WebDAV，兼容 OpenList 与阿里云盘）
- 📜 **应用内日志查看器** - 直接在设置中按日志级别筛选、关键字搜索，结构化分行显示并支持自由拖选复制
- 🔄 **应用内签名自动更新** - 启动延迟静默检测、minisign 公钥验签、实时下载进度条与草稿保护安全重启

---

## 🗺️ 路线图 / TODO List

- [x] **更新检测与应用内签名自动更新** - 桌面端 `tauri-plugin-updater` 验签更新、GitHub Releases 兜底检测与 `latest-android.json` 清单
- [ ] 🐳 **Docker 管理** - 本机与远端容器启停、实时日志流、一键进入容器 Exec 终端、镜像 / 卷 / 网络管理与 Compose 编排视图
- [ ] 🧵 **进程与网络 / 端口线程查询** - 实时进程树、线程级 CPU / 内存占用排行、监听端口与 TCP / UDP 连接反查（快速定位端口占用进程）
- [ ] 📱 **Android 与 iOS 移动端深度适配** - 触控快捷键栏、手势切屏与分屏、Android Keystore / iOS Keychain 解锁、Android 应用内 APK 校验与原生安装器、移动端响应式体验
- [ ] ☁️ **更多同步后端与包生态** - S3 / Gist 同步界面接入，以及 Homebrew / Winget / AUR / Flatpak 分发渠道

---

## 🚀 本地开发与构建

### 前置条件

- **Rust stable**（版本见 [rust-toolchain.toml](rust-toolchain.toml)）
- **Node.js >= 24** 与 **pnpm**
- **Linux 桌面额外依赖**：`webkit2gtk-4.1`、`libsoup-3.0`、`javascriptcoregtk-4.1` 与可用的 Secret Service（如 GNOME Keyring）

### 桌面开发模式

```bash
# 安装前端依赖与 Git 规范钩子
pnpm install

# 启动桌面开发版
pnpm tauri dev

# 另开终端启动内置本地测试 SSH 服务器（账号 demo / 密码 demo）
cargo run -p devsshd -- --addr 127.0.0.1:2222
```

### 生产包与跨平台构建

```bash
# 当前平台构建安装包
pnpm tauri build

# macOS 构建临时签名的 Apple Silicon + Intel 通用包
rustup target add aarch64-apple-darwin x86_64-apple-darwin
APPLE_SIGNING_IDENTITY="-" pnpm tauri build --target universal-apple-darwin --config '{"bundle":{"createUpdaterArtifacts":false}}'

# Linux 交叉构建 Windows x64 NSIS 安装包
pnpm build:windows

# Android 调试 APK（需配置 ANDROID_HOME、NDK_HOME 与 JDK 17）
pnpm tauri android build --debug --apk --target aarch64
```

### 手动签名发布（`gh`）

```bash
# 1. 同步递增 Cargo.toml 与 package.json 版本号
pnpm release:bump 0.2.0

# 2. 携带本机 minisign 私钥构建带签名更新包（私钥绝不入库）
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/roveterm.key)" pnpm tauri build

# 3. 汇总产物、生成 latest.json / latest-android.json / SHA256SUMS.txt 与双语发布说明
pnpm release:prepare --tag v0.2.0
```

在 macOS 上手动发布时，给构建命令加上 `APPLE_SIGNING_IDENTITY="-"`。上例使用无密码的更新签名密钥。

> 更多构建细节、多机合并 `latest.json`（`--merge-latest`）与测试命令见 [docs/development.md](docs/development.md)。

---

## 🏗️ 技术架构

### 前端技术栈
- **Svelte 5 (Runes)** - 响应式组件框架，极低运行时开销
- **TypeScript** - 严格类型约束，覆盖全部 IPC 契约与状态机
- **Vite** - 极速前端开发与生产打包工具
- **xterm.js 6** - 高性能终端模拟器（集成 WebGL / Fit / Search / Unicode11 / Web Links）
- **CodeMirror 6** - 工作区标签内置代码编辑器（行号、多语言高亮、查找替换）
- **Marked** - 安全词法解析 Markdown（纯元素渲染，不注入原始 HTML）
- **Lucide & Simple Icons** - 统一图标体系与主流 Linux / NAS / 虚拟化系统品牌标志

### 后端与核心技术栈
- **Rust (2024 Edition) & Tauri 2** - 高性能、低内存占用的跨平台原生应用外壳
- **rove-kernel** - 静态链接插件内核：服务注册表、事件总线（Journal / Live / Waterfall 拦截器）与 `Disposer` 生命周期管理
- **Tokio & russh** - 异步运行时与纯 Rust SSH / SFTP / 端口转发连接复用引擎
- **SQLite (rusqlite) & XChaCha20-Poly1305 / Argon2id** - 本地事件日志、按字段加密凭据库与端到端多设备同步
- **rustls + ring** - 统一纯 Rust TLS 栈，桌面与移动端零 OpenSSL 依赖
- **tailscale-rs** - 嵌入式桌面端尾网运行时（WireGuard / Disco 探测 / DERP 中继）
- **tauri-plugin-updater (minisign)** - 签名校验更新清单与增量安装能力

### 系统与协议集成
- **PTY / ConPTY** - 跨平台本地交互式 Shell 终端
- **OS Keychain / Android Keystore** - 设备主密钥隔离托管，启动静默解锁
- **ACP & MCP** - 双向接入本地 AI Agent 与外部智能体工具桥接
- **WebDAV / 本地文件夹同步** - 端到端密文同步分段，支持 OpenList / 阿里云盘等网盘 WebDAV

---

## 📁 项目结构

```text
RoveTerm/
├── crates/                     # Rust 核心插件 crate（独立可测，不依赖 Tauri）
│   ├── rove-kernel/           # Context、服务注册表、事件总线、Disposer、Profile
│   ├── rove-session/          # 终端会话管理、输出合并、回放缓冲
│   ├── rove-pty/              # 本地 PTY 终端传输（Unix PTY / Windows ConPTY）
│   ├── rove-ssh/              # SSH 连接复用、known_hosts、ssh_config、SFTP、端口转发、OS 识别
│   ├── rove-fs/               # FileSystem seam、本地文件系统与传输队列
│   ├── rove-vault/            # 主机/密钥/偏好存储与 XChaCha20-Poly1305 字段加密
│   ├── rove-journal/          # SQLite 持久事件日志
│   ├── rove-sync/             # 端到端加密多设备同步（本地目录 / WebDAV）
│   ├── rove-http/             # 统一 HTTPS 客户端构造（rustls + ring）
│   ├── rove-tailscale/        # 桌面端内置 Tailscale 尾网服务与 TCP 拨号器
│   ├── rove-ai/               # AI 助手、命令风险分级、审批策略、ACP 与 MCP 桥接
│   └── rove-ops/              # 持久运维任务、主机授权范围与 worker 调度
├── src-tauri/                  # Tauri 2 应用外壳（desktop / mobile profile 与 IPC 命令）
│   ├── src/commands/          # 按领域拆分的 Tauri 命令（session / sftp / tailscale / ai / update 等）
│   ├── profiles/              # desktop / mobile 内置插件装配配置
│   └── tauri.conf.json        # 应用配置、CSP 与更新器公钥
├── ui/                         # Svelte 5 + TypeScript 前端应用
│   └── src/lib/               # terminal / layout / files / hosts / panels / ai / tailscale / update / i18n
├── tools/
│   └── devsshd/               # 本地集成测试用轻量 SSH 服务器
├── scripts/                    # 文档预算检查、Git 工作流钩子与本地手动发版脚本（release.mjs）
├── docs/                       # 架构、安全模型、开发指南、测试基线、子系统契约与 ADR 决策记录
├── third_party/                # 固定版本上游补丁（tailscale-rs 兼容修复）
├── CHANGELOG.md                # 中英双语完整更新日志
├── AGENTS.md                   # 仓库开发与 AI 协作规则
└── Cargo.toml                  # Rust Workspace 与单一版本号真源
```

---

## ⌨️ 常用快捷键

> `App` 在 macOS 上映射为 `⌘`，在 Windows / Linux 上映射为 `Ctrl+Shift`，普通 `Ctrl+字母` 始终交给终端内的 Shell。

| 快捷键 | 功能 | 快捷键 | 功能 |
|---|---|---|---|
| `App+T` | 新建本地终端 | `App+P` | 打开命令面板 |
| `App+K` | 快速连接（`user@host:port`） | `App+,` | 打开设置 |
| `App+D` / `App+E` | 向右 / 向下分屏 | `App+B` | 收起 / 展开左侧主机栏 |
| `App+]` / `App+[` | 切换聚焦分屏组 | `App+L` / `App+J` | 切换右侧面板 / 底部文件面板 |
| `App+O` | 在终端旁打开 SFTP 文件 | `App+I` | 打开 AI 助手面板 |
| `App+S` / `App+F` | 编辑器保存 / 查找替换 | `App+W` | 关闭当前标签或面板 |

---

## 📦 快速下载

> 点击下方平台图标直接前往最新版下载页；每版 GitHub Release 均附带**按平台与架构分类的一键直链**与 **SHA-256 校验和**，桌面端安装后支持**应用内 minisign 签名自动更新**。

<div align="center">

| 平台 | 点击图标获取安装包 | 自动更新支持 |
|---|---|---|
| **macOS** | [![macOS Apple Silicon](https://img.shields.io/badge/DMG-Apple%20Silicon-000000.svg?logo=apple&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) [![macOS Intel x64](https://img.shields.io/badge/DMG-Intel%20x64-374151.svg?logo=apple&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) [![macOS Universal](https://img.shields.io/badge/DMG-Universal-00A9E0.svg?logo=apple&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) | ✅ 应用内签名更新（`.app.tar.gz`） |
| **Windows** | [![Windows x64](https://img.shields.io/badge/Setup-x64-2d7d9a.svg?logo=windows&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) [![Windows ARM64](https://img.shields.io/badge/Setup-ARM64-67b7d1.svg?logo=windows&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) | ✅ 应用内签名更新（NSIS `.exe`） |
| **Linux** | [![Linux AppImage](https://img.shields.io/badge/AppImage-x64%20%2F%20ARM64-f84e29.svg?logo=linux&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) [![Linux Debian](https://img.shields.io/badge/DebPackage-x64%20%2F%20ARM64-FF9966.svg?logo=debian&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) [![Linux RPM](https://img.shields.io/badge/RpmPackage-x64%20%2F%20ARM64-F1B42F.svg?logo=redhat&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) | ✅ AppImage 应用内签名更新 |
| **Android** | [![Android APK](https://img.shields.io/badge/APK-ARMv8-168039.svg?logo=android&logoColor=white)](https://github.com/Gabrlie/RoveTerm/releases/latest) | 🚧 检测与校验清单已就绪（见路线图） |

</div>

> 完整中英双语版本记录请查看 [**CHANGELOG.md**](CHANGELOG.md)。

---

## 📄 开源协议

本项目基于 **MIT License** 开源，详见 [LICENSE](LICENSE)。

---

<div align="center">

**✨ 让多服务器运维与终端协作回归清爽高效！**

**RoveTerm — 轻量、安全的跨平台智能终端**

[📦 下载最新版](https://github.com/Gabrlie/RoveTerm/releases/latest) | [📝 更新日志](CHANGELOG.md) | [🐛 反馈问题](https://github.com/Gabrlie/RoveTerm/issues)

</div>
