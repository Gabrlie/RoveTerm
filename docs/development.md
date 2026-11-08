# 开发指南

## 搭建教程

### 前置条件

- Rust stable（见 `rust-toolchain.toml`）
- Node.js 24+ 与 pnpm
- Linux 桌面：`webkit2gtk-4.1`、`libsoup-3.0`、`javascriptcoregtk-4.1` 开发包，以及可用的 Secret Service（如 GNOME Keyring）
- Android（可选）：Android SDK 与 NDK，设置 `ANDROID_HOME` 和 `NDK_HOME`，安装 `aarch64-linux-android` 等 Rust target

### 首次运行

1. 安装前端依赖：`pnpm install`
2. 启动桌面开发版：`pnpm tauri dev`
3. 另开一个终端，启动测试 SSH 服务器：`cargo run -p devsshd -- --addr 127.0.0.1:2222`
4. 在应用中快速连接 `demo@127.0.0.1:2222`，密码 `demo`，确认能进入 shell。

## 贡献者参考

### 目录与依赖方向

依赖只能从上往下：`src-tauri` → 插件 crate → `rove-kernel`。插件 crate 之间只依赖对方的 Definition（trait 和类型），不依赖对方的内部实现。

### 数据目录

应用数据存放在系统的应用数据目录下（Linux 为 `~/.local/share/dev.roveterm.app`）。设置环境变量 `ROVETERM_DATA_DIR` 可以改用其他目录，便于并行运行多个开发实例。

### 提交

遵循根目录 [AGENTS.md](../AGENTS.md#提交) 的提交规范。

### TODO 标记

按紧急程度分为 `FIXME`（错误行为，需要尽快修复）、`TODO`（计划中的工作）、`XXX`（可疑或脆弱的实现，需要复查）。
