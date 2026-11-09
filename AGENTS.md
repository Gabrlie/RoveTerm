# AGENTS.md

RoveTerm 是一款基于 Tauri 2 + Rust 的跨平台智能终端（Windows / macOS / Linux / Android）。改动 `crates/` 或 `src-tauri/` 之前先读 [docs/architecture.md](docs/architecture.md)；写文档遵循 [docs/AGENTS.md](docs/AGENTS.md)。

## 仓库布局

```
crates/      Rust 核心 crate，rove-<name>，每个 crate 负责一个 seam 的定义或实现
  rove-kernel/    Context、服务注册、事件总线、Disposer、Profile
  rove-session/   终端会话管理、输出合并、回放缓冲
  rove-pty/       本地 PTY Transport
  rove-ssh/       SSH Transport、连接复用、known_hosts、ssh_config、SFTP、端口转发
  rove-fs/        FileSystem seam、本地文件系统、传输队列
  rove-vault/     凭据加密存储
  rove-journal/   持久事件日志
  rove-sync/      端到端加密的多设备同步
src-tauri/   desktop / mobile profile 的应用外壳，只做 IPC 适配
ui/          Svelte 5 + TypeScript + xterm.js 前端
tools/       开发工具（devsshd 测试 SSH 服务器）
docs/        文档（docs/AGENTS.md）
scripts/     文档检查等脚本
```

## 命令

```sh
pnpm install                     # 前端依赖，node >= 24
pnpm tauri dev                   # 启动桌面开发版
pnpm tauri build                 # 打包
cargo test -p <crate> <filter>   # 只跑受影响 crate 的测试
cargo clippy -p <crate> --all-targets
cargo run -p devsshd -- --addr 127.0.0.1:2222   # 本地测试 SSH 服务器
node scripts/check-docs.mjs      # 文档字数上限与相对链接检查
```

## 约定

- **插件，不改内核**：新行为挂到 [architecture.md 的归属表](docs/architecture.md#新行为的归属位置) 列出的扩展点上；改动 `rove-kernel` 的语义需同步更新 architecture.md。
- **注册即副作用**：每一项服务注册或事件监听都返回 `Disposer`，插件卸载时 drop 即撤销（[kernel](docs/subsystems/kernel.md)）。
- **能力 seam 由 Definition / Provider / Consumer 三个角色组成**，新增能力时三者一起设计（[术语](docs/glossary.md#seam)）。
- **Waterfall 监听器必须调用 `next`** 才能把请求交给下一层；不调用即短路。
- **AI 可见即已记录**：AI 能看到的输入、执行过的命令、读写过的文件都必须能从 Journal 还原。
- **跨边界 ID 用 newtype**（`SessionId`、`HostId`），不用裸 `String` / `Uuid`。
- **显式优于隐式**：默认值在所属实现里通过 `resolve(request) -> Spec` 一步确定，不在执行路径里藏 `unwrap_or(default)`。
- **插件里不写死可调参数**：随部署变化的取值放进校验过的配置字段；协议常量和安全不变量保持固定。
- **配置错误立即失败**：能在加载时发现的错误在加载时报出，不静默跳过。
- **密钥不落明文**：密码、私钥、passphrase、token 只以密文写入 vault；不写日志、不写 JSON、不写测试夹具（[security.md](docs/security.md)）。
- **外部输入不可信**：IPC 参数、Agent 进程消息、远程服务器数据都在边界校验；同进程内的类型化调用信任类型系统。
- **错误用 `thiserror` 定义的类型化枚举**，库 crate 不 `unwrap` / `expect` 外部输入；空的错误分支要写明原因。
- **UI 文案走 i18n 字典**，不在组件里写死；应用快捷键只通过 actions 表注册（[ui](docs/subsystems/ui.md)）。
- **注释写契约，不写推理过程**；不复述代码。
- **文档跟随代码**：改动行为时在同一个提交里更新 crate README 和对应的 subsystems 页。

## 测试

- 只测逻辑，不写截图或像素类 UI 测试（[testing.md](docs/testing.md)）。
- 代码改动后只跑受影响 crate 的测试；纯文档改动不跑测试，只检查文档。
- 缺少相关测试时补上针对性测试，不用全量测试代替。

## 提交

- Conventional Commits：`type(scope): subject`，scope 用 crate 或模块名。
- 不添加任何署名行（包括 `Co-Authored-By`）。
- 一个提交只做一个原子任务；功能的文档和代码在同一个提交里。
- 提交 body 用 `- ` 无序列表，每点一行，使用真实换行。

## 编辑本文件

`CLAUDE.md` 是指向本文件的符号链接，编辑本文件即可。每条规则保持 1–3 行并链接到它的归宿文档。
