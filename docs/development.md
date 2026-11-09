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

### 从 Linux 交叉构建 Windows 安装包

1. 安装工具：`sudo apt install nsis lld llvm clang`、`rustup target add x86_64-pc-windows-msvc`、`cargo install --locked cargo-xwin`。
2. 构建：`pnpm build:windows`。首次运行会下载 MSVC CRT 与 Windows SDK（需同意其许可，脚本已设置 `XWIN_ACCEPT_LICENSE=1`）。
3. 产物位于 `target/x86_64-pc-windows-msvc/release/bundle/nsis/RoveTerm_<版本>_x64-setup.exe`，未签名，首次运行时 SmartScreen 会提示。

交叉构建只产出 NSIS 安装包；MSI 和代码签名需要在 Windows 上构建。链接时 `LNK4099` 缺少 pdb 的警告可以忽略。

### 在 macOS 上构建

1. 安装 Xcode 命令行工具（`xcode-select --install`）、Rust、Node.js 24+ 与 pnpm。
2. 构建：`pnpm install && pnpm tauri build`，产物位于 `target/release/bundle/macos/RoveTerm.app` 与 `target/release/bundle/dmg/`。
3. 同时支持 Apple Silicon 与 Intel：先 `rustup target add aarch64-apple-darwin x86_64-apple-darwin`，再 `pnpm tauri build --target universal-apple-darwin`。

产物未签名：首次打开需在 Finder 中右键选择“打开”，或执行 `xattr -cr /Applications/RoveTerm.app`。钥匙串首次读取设备主密钥时会弹窗授权，选“始终允许”后之后启动即可静默解锁。

### Android

1. 确认已设置 `ANDROID_HOME`、`NDK_HOME`，并安装 JDK 17。
2. 构建调试 APK：`pnpm tauri android build --debug --apk --target aarch64`，产物位于 `src-tauri/gen/android/app/build/outputs/apk/`。
3. 连接设备调试：`pnpm tauri android dev`。

`mobile` profile 不启用本地终端；在接入 Android Keystore 之前，凭据库在 Android 上保持锁定，可以使用每次输入密码的方式连接主机。

## 贡献者参考

### 目录与依赖方向

依赖只能从上往下：`src-tauri` → 插件 crate → `rove-kernel`。插件 crate 之间只依赖对方的 Definition（trait 和类型），不依赖对方的内部实现。

### 数据目录

应用数据存放在系统的应用数据目录下（Linux 为 `~/.local/share/dev.roveterm.app`）。设置环境变量 `ROVETERM_DATA_DIR` 可以改用其他目录，便于并行运行多个开发实例。用户配置文件是应用配置目录下的 `roveterm.toml`（Linux 为 `~/.config/dev.roveterm.app/roveterm.toml`），可以覆盖任意插件的配置段。开发构建使用独立的钥匙串服务名 `dev.roveterm.app.dev`，不会读写正式版的设备主密钥。在没有系统钥匙串的环境（无头 X 服务器、CI）中，开发构建可以设置 `ROVETERM_DEV_MEMORY_KEYCACHE=1`，改用只存在于内存中的设备主密钥；重启后，之前保存的凭据将无法解密。release 构建忽略该变量。

### 提交

遵循根目录 [AGENTS.md](../AGENTS.md#提交) 的提交规范。

### TODO 标记

按紧急程度分为 `FIXME`（错误行为，需要尽快修复）、`TODO`（计划中的工作）、`XXX`（可疑或脆弱的实现，需要复查）。
