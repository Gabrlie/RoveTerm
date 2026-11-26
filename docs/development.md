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

1. 安装工具：`sudo apt install nsis lld llvm clang`、`rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc`、`cargo install --locked cargo-xwin`。
2. 构建：`pnpm build:windows`（x64）或 `pnpm build:windows:arm64`（ARM64）。首次运行会下载 MSVC CRT 与 Windows SDK（需同意其许可，脚本已设置 `XWIN_ACCEPT_LICENSE=1`）。
3. 产物位于 `target/<target>/release/bundle/nsis/RoveTerm_<版本>_<arch>-setup.exe`，未签名，首次运行时 SmartScreen 会提示。

交叉构建只产出 NSIS 安装包；MSI 和代码签名需要在 Windows 上构建。链接时 `LNK4099` 缺少 pdb 的警告可以忽略。

### 在 macOS 上构建

1. 安装 Xcode 命令行工具（`xcode-select --install`）、Rust、Node.js 24+ 与 pnpm。
2. 运行 `pnpm install`，再用 `APPLE_SIGNING_IDENTITY="-" pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'` 为本机架构构建临时签名（ad hoc）的安装包。产物位于 `target/release/bundle/`。
3. 运行 `rustup target add aarch64-apple-darwin x86_64-apple-darwin`。构建时用 `--target` 选择 `aarch64-apple-darwin`（Apple Silicon）、`x86_64-apple-darwin`（Intel x64）或 `universal-apple-darwin`（通用包）。产物位于 `target/<target>/release/bundle/`。

临时签名的开源安装包未经 Apple 公证。将 RoveTerm 移到「应用程序」后，若 macOS 提示「已损坏，无法打开」，运行 `xattr -cr /Applications/RoveTerm.app` 后重新打开。钥匙串首次读取设备主密钥时会弹窗授权，选“始终允许”后后续启动可自动解锁。

### 版本号与手动签名发布（`gh`）

版本号遵循 SemVer 2.0（`MAJOR.MINOR.PATCH[-prerelease]`），以根目录 `Cargo.toml` 的 `[workspace.package].version` 为单一真源（`src-tauri/tauri.conf.json` 自动继承）：Pre-1.0 阶段仅在新增独立功能模块、跨平台里程碑或配置/同步格式变更时递增 `MINOR`（如 `0.2.0`、`0.3.0`）；缺陷修复、UI 交互打磨与既有子系统的体验重构递增 `PATCH`（如 `0.2.1`）。发版分为**稳定版**（`stable`，无预发布后缀，如 `0.3.0`）与**测试版**（`beta`，带 `-beta.N` 预发布后缀，如 `0.3.0-beta.1`）两个通道：稳定版发布为 GitHub `Latest`，测试版发布为 GitHub `Pre-release`（不覆盖 `Latest`），应用内可在「设置 → 关于 → 更新通道」切换订阅，测试版安装包默认跟踪测试通道并在升级到正式版时平滑过渡。

本地构建带签名更新包并经 GitHub CLI（`gh`）发布：

1. 同步递增版本号：显式指定版本 `pnpm release:bump 0.3.0-beta.1`，或按通道自动递增 `pnpm release:bump:beta`（`0.2.1` → `0.3.0-beta.1` → `0.3.0-beta.2`）/ `pnpm release:bump:stable`（`0.3.0-beta.2` → `0.3.0`，或递增 `PATCH`），并在 [CHANGELOG.md](../CHANGELOG.md) 记录中英双语更新日志。
2. 签名密钥保存在构建机本机（如 `~/.tauri/roveterm.key`，权限 `0600`），私钥绝不入库。公钥写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。构建时传入 `TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/roveterm.key)"` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，无密码时传空字符串。打包开源 macOS 版本时同时设置 `APPLE_SIGNING_IDENTITY="-"`。
3. 汇总产物、生成 `latest.json`（若含已签名发布 `.apk` 则同步生成带通道标记的 `latest-android.json`）、`SHA256SUMS.txt` 与带平台图标下载表的双语 `RELEASE_NOTES.md`：稳定版运行 `pnpm release:prepare:stable --tag v0.3.0`（对比基准默认取上一个稳定 tag，发布命令附带 `--latest`），测试版运行 `pnpm release:prepare:beta --tag v0.3.0-beta.1`（自动附带双语测试版说明与 `--prerelease`）。客户端按 Release 实际包含的平台资产筛选更新，未改动且未上传产物的平台不会收到更新提示，因此单平台迭代可只构建并发布受影响平台。多台机器分批补建其他平台时，传入 `--merge-latest <已有 latest.json 路径或 URL>` 或 `--merge-android` 合并平台条目，再使用 `gh release upload --clobber` 更新。
4. 审阅并按需微调生成的 `RELEASE_NOTES.md`，执行脚本输出的 `gh release create` 命令（或附加 `--publish --draft` 先创建草稿发布）。

发布三个 macOS 安装包时，用多个 `--from` 指定各架构的 `bundle/dmg` 和通用包的 `bundle/macos`。通用包的更新归档及签名供两种 macOS 架构共用。

### Android

1. 确认已设置 `ANDROID_HOME`、`NDK_HOME`，并安装 JDK 17+。
2. 构建调试 APK：`pnpm tauri android build --debug --apk --target aarch64`，产物位于 `src-tauri/gen/android/app/build/outputs/apk/`。
3. 构建已签名发布 APK：通过环境变量 `ROVETERM_ANDROID_KEYSTORE_PATH`、`ROVETERM_ANDROID_KEYSTORE_PASSWORD`、`ROVETERM_ANDROID_KEY_ALIAS`、`ROVETERM_ANDROID_KEY_PASSWORD` 或本地未入库的 `src-tauri/gen/android/keystore.properties` 提供本机签名密钥库，再运行 `pnpm tauri android build --apk --target aarch64`。`scripts/release.mjs` 仅收录已签名的发布 APK，自动跳过 `debug` 与 `unsigned` 包。
4. 连接设备调试：`pnpm tauri android dev`。

`build.gradle.kts` 默认读取根目录 `Cargo.toml` 的 `[workspace.package].version` 作为 `versionName`，并派生随发行版本递增的 `versionCode`。例如 `0.3.0-beta.1`、`0.3.0-beta.2` 和 `0.3.0` 分别对应 `300301`、`300302` 和 `300999`。`tauri.properties` 中的 `tauri.android.versionName` 和 `tauri.android.versionCode` 各自优先。

派生编码要求主版本号在 `0..=209` 范围内，次版本号和修订号在 `0..=99` 范围内。超过各自编码上限的预发布序号共用该上限，其他预发布标签共用偏移值 `1`。

Android 使用 Keystore 保护凭据库的设备主密钥，可在应用的安全设置中启用生物识别或屏幕锁验证。也可在装有 Android SDK、NDK、JDK 17、Rust 与 Node.js / pnpm 的 Docker 容器中挂载工作区，使用相同命令构建 APK。

`src-tauri/gen/android/app/src/main/AndroidManifest.xml` 声明 `SessionKeepAliveService` 为 `specialUse` 类型的前台服务，登记用途及 `FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_SPECIAL_USE`、`POST_NOTIFICATIONS`、`WAKE_LOCK` 与用于应用内 APK 更新安装的 `REQUEST_INSTALL_PACKAGES` 权限。有活动会话等工作时，服务通过带「断开全部」操作的通知展示保活状态。

`file_paths.xml` 通过 `cache-path`、`files-path`、`external-files-path` 和 `external-cache-path` 为 `FileProvider` 注册应用缓存、文件及外部缓存目录，供应用内 APK 更新安装器与系统分享面板安全读取本地文件。应用内更新下载已签名发布 APK 至应用缓存并完成 SHA-256 校验后自动拉起系统安装界面，首次缺少未知来源安装权限时引导授权并在返回后自动续装。

移动端启动图标以 [RoveTerm 图标](../src-tauri/icons/icon.png) 为源。Android 提供自适应图标与各密度的回退资源，iOS 的 AppIcon 使用不透明 PNG。

### iOS / iPadOS（用户自签名）

iOS / iPadOS 由用户自签名安装，App Store 和 `pnpm release:prepare` 的官方发布包均不提供安装包。在装有 Xcode 的 macOS 上运行 `pnpm tauri ios init` 生成 Xcode 工程。工作区插件 [`tauri-plugin-rove-mobile`](../src-tauri/plugins/tauri-plugin-rove-mobile/README.md) 在构建时接入 Swift Keychain 和 Insets 代码，并补齐 `Info.plist` 的 Face ID 和本地网络用途说明。使用 Xcode 构建并签名，或通过侧载工具签名和安装。

应用带活动工作进入后台时，`KeepAliveController` 通过 `beginBackgroundTask` 向系统申请有限的后台运行时间。时限到达、活动结束或应用回到前台时，控制器调用 `endBackgroundTask`。

`tauri-plugin-rove-mobile` 通过 `UIActivityViewController` 分享应用沙盒中的下载文件，通过 `UIDocumentPickerViewController` 将副本导出到用户选择的位置。

## 开发交付教程

1. 在本任务的实际 worktree 中运行 `git status --short --branch`，确认目录、分支和已有差异，记录本任务的改动归属。
2. 按[分支规范](#分支规范)创建或选择任务分支。使用 dsh 的 `enter_worktree` 时显式传入合规的 `name`，如 `feat/text-editor`；若工具已生成临时分支，在首次修改前用 `git branch -m <name>` 更正实际 Git 分支，worktree 目录可保持原名。
3. 将任务拆为可独立验证、回退的行为单元，完成一个单元所需的代码、测试和文档，并运行受影响部分的检查。验证受阻时说明失败原因和剩余改动。
4. 只暂存本单元的文件或差异块，审阅 `git diff --cached`，按[提交规范](../AGENTS.md#提交)完成本地提交，再开始下一单元。暂存区含其他任务的改动时，先将本任务放到独立 worktree 处理。
5. 交付前在同一 worktree 核对 `git status --short --branch` 和本任务的提交记录，确认本任务改动均已提交、开工前已有的其他差异保留，报告分支、提交 hash 和验证结果。

开发请求默认包含上述本地提交流程。只读研究任务没有开发提交；用户明确要求暂不提交时，报告验证结果和保留的改动。

## 贡献者参考

### 目录与依赖方向

依赖只能从上往下：`src-tauri` → 插件 crate → `rove-kernel`。插件 crate 之间只依赖对方的 Definition（trait 和类型），不依赖对方的内部实现。

### 数据目录

应用数据存放在系统的应用数据目录下（Linux 为 `~/.local/share/dev.roveterm.app`）。开发构建默认使用独立的 `.dev` 数据目录（桌面端如 `~/.local/share/dev.roveterm.app.dev`，macOS 为 `~/Library/Application Support/dev.roveterm.app.dev`，移动端为应用数据目录下的 `dev/`）和独立的钥匙串服务名 `dev.roveterm.app.dev`，与正式版的 `vault.db`、日志及设备主密钥完全隔离，不会互相覆盖。设置环境变量 `ROVETERM_DATA_DIR` 可以改用其他目录，便于并行运行多个开发实例。用户配置文件是应用配置目录下的 `roveterm.toml`（Linux 为 `~/.config/dev.roveterm.app/roveterm.toml`），可以覆盖任意插件的配置段。在没有系统钥匙串的环境（无头 X 服务器、CI）中，开发构建可以设置 `ROVETERM_DEV_MEMORY_KEYCACHE=1`，改用只存在于内存中的设备主密钥；重启后，之前保存的凭据将无法解密。release 构建忽略该变量。

### 分支规范

分支采用 [Conventional Branch 1.1.0](https://conventionalbranch.org/v1.1.0/spec.json) 的名称语法，项目任务分支使用下表的类型；现有主分支 `main` 保留。

| 前缀 | 用途 | 示例 |
|---|---|---|
| `feat/` | 新功能 | `feat/text-editor` |
| `fix/` | 常规错误修复 | `fix/sftp-save` |
| `hotfix/` | 紧急修复 | `hotfix/credential-leak` |
| `release/` | 发布准备 | `release/v0.1.0` |
| `chore/` | 文档、工具、依赖维护及不改变行为的重构 | `chore/git-workflow` |

任务分支名称为 `<type>/<description>`，只含一个 `/`。描述由小写英文字母或数字组成的片段构成，片段之间使用单个 `-` 或 `.`，首尾为字母或数字；例如 `feat/new--editor`、`fix/save_file`、`release/v0.1.0.` 均不合法。项目允许的前缀以此表为准。

`pnpm check:branch` 检查当前 worktree 的 Git 分支；创建分支前可用 `pnpm check:branch <name>` 检查拟用名称。

### 提交格式

遵循根目录 [AGENTS.md](../AGENTS.md#提交) 的提交规范。

提交消息的 `type` 使用小写标识，`scope` 为小写 crate 或模块标识，可含数字、点、下划线、连字符和斜杠，允许在 `)` 后添加 Conventional Commits 的 `!` 标记。body 存在时与标题空一行，每个非空行以 `- ` 开头并写在一行内。`pnpm check:commit <message-file>` 清除注释并截去 Git 剪切标记及其后的内容，只读校验剩余正文，并拒绝常见的署名 trailers；注释前缀遵循 `core.commentString` 或 `core.commentChar`。

### Git 检查

`pnpm install` 自动安装仓库内的 `.githooks`；已有依赖时可运行 `pnpm hooks:install`。安装器可重复执行，只配置当前仓库的 `core.hooksPath`；发现已有自定义 hooks 配置或有效钩子时报告冲突，保留原配置和钩子。

| 钩子 | 检查 |
|---|---|
| `pre-commit` | 当前 worktree 的分支名称、暂存差异中的空白错误 |
| `commit-msg` | 清理并校验提交消息，通过后写回已验证正文，使实际提交消息与校验结果一致 |
| `pre-push` | 实际推送的分支引用名称，跳过标签和删除分支操作 |

钩子执行快速 Git 规范检查；受影响模块的测试按[开发交付流程](#开发交付教程)运行。及时提交及提交的行为边界在该流程中核对。Git 检查脚本和安装器的逻辑测试通过 `pnpm test:workflow` 运行。

### TODO 标记

按紧急程度分为 `FIXME`（错误行为，需要尽快修复）、`TODO`（计划中的工作）、`XXX`（可疑或脆弱的实现，需要复查）。
