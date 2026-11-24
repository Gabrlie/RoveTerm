# RoveTerm 架构

改动 `crates/` 或 `src-tauri/` 之前先读本文。术语见[术语表](glossary.md)，安全边界见 [security.md](security.md)。

## 分层

```
ui/            Svelte 5 + xterm.js，在 WebView 里渲染；只持有 SessionId / HostId
  │ invoke（控制面）            │ ipc::Channel（终端字节，数据面）
src-tauri/     应用外壳：按 profile 组装插件，把 Tauri 命令翻译成服务调用
  │
rove-kernel    Context：服务注册表 + 事件总线 + Disposer + Profile
  │
插件 crate     rove-session / rove-pty / rove-ssh / rove-fs / rove-vault / rove-journal ……
```

Rust 侧不依赖 UI：所有核心 crate 都能脱离 Tauri 单独测试，headless profile（`rove` CLI，用于 MCP / ACP 模式）复用同一套插件。

## 内核与插件

[`rove-kernel`](subsystems/kernel.md) 是唯一的公共底座。插件向共享的 `Context` 提供服务、监听事件、注册拦截器；**每一项注册都返回 `Disposer`**，插件卸载时按注册的逆序 drop，从而撤销它的全部影响。内核本身不含业务逻辑，没有需要打补丁的特权路径：扩展 RoveTerm 的方式是把插件挂在其他插件旁边。

插件在编译期静态链接，由 profile 选择启用哪些（[决策 0004](decisions/0004-static-plugin-kernel.md)）。进程外扩展通过 MCP / ACP 接入，不在进程内加载动态代码。

## Profile

Profile 是一组启用的插件加一份 TOML 配置。随应用交付的 profile：

| Profile | 载体 | 用途 |
|---|---|---|
| `desktop` | `src-tauri` | Windows / macOS / Linux 桌面应用 |
| `mobile` | `src-tauri`（Android / iOS） | 移动端，不启用本地 PTY 或定时任务 |
| `headless` | `rove` CLI | 以 MCP server / ACP 模式运行，无 UI |

配置按层合并：内置 profile（`src-tauri/profiles/*.toml`）→ 用户配置文件（应用配置目录下的 `roveterm.toml`）→ 应用运行时生成的一层（数据库路径、钥匙串服务名）；后一层按表深度合并覆盖前一层。每个插件在 `apply` 时校验自己的配置段，配置错误让启动失败，而不是被静默忽略。

## 核心 crate

| crate | 职责 | 提供的服务 |
|---|---|---|
| [rove-kernel](subsystems/kernel.md) | 服务注册、事件总线、Disposer、Profile、插件运行时 | `Context` |
| [rove-session](subsystems/session.md) | `TerminalSession` seam 的定义；会话表、输出合并、回放缓冲 | `SessionManager` |
| [rove-pty](subsystems/pty.md) | 本地 PTY 会话 | `LocalTerminals`、`LocalMonitor` |
| [rove-ssh](subsystems/ssh.md) | SSH 认证、host key 校验、连接复用、ssh_config、SFTP、端口转发 | `SshConnector`、`Forwards` |
| [rove-fs](subsystems/fs.md) | `FileSystem` seam 的定义；本地文件系统、传输队列 | `FsSessions`、`TransferQueue`、`LocalFs` |
| [rove-vault](subsystems/vault.md) | 主机与凭据存储、字段级加密、设备主密钥 | `Vault`、`KeyCache` |
| [rove-journal](subsystems/journal.md) | 持久事件日志 | `JournalStore` |
| [rove-sync](subsystems/sync.md) | 端到端加密的多设备同步 | `SyncBackend`、`RecordStore`、`sync` |
| rove-http | 共享的 HTTPS 客户端构造（rustls + ring，Android 用内置根证书） | `client` |
| [rove-ai](subsystems/ai.md) | AI Agent：命令风险分级、审批策略 | `Policy` |
| [rove-tailscale](subsystems/tailscale.md) | 内置尾网连接、设备发现与 TCP 流 | `TailnetService` |
| [rove-ops](subsystems/ops.md) | 持久运维任务、主机范围与共享 worker 预算 | `TaskStore`、`OpsSupervisor` |

<a id="events"></a>

## 事件

事件分三类，用途不同：

- **Journal 事件**是追加到 `JournalStore` 的持久事实：会话打开与关闭、连接记录、布局变化、AI 的轮次、工具调用和审批结果。一个事实需要在重启后仍然存在（会话恢复、审计）时，用它。
- **Live 事件**是只在进程内广播的瞬时通知：终端输出、传输进度、host key 确认请求、AI 流式分片。观察进行中的工作时，用它。
- **能力事件**是挂在某个能力前面的 waterfall 拦截点，例如 `exec/pre`、`fs/pre`、`tool/pre`。权限策略、审计和脱敏挂在这里，不需要修改提供能力的插件。

Waterfall 拦截器按优先级串成链，**拦截器必须调用 `next` 才会把请求交给下一层**；不调用就是短路，并直接返回自己的结果（例如拒绝）。

## 终端数据流

```
Provider（rove-pty / rove-ssh）
  └─ open → OpenedTerminal { session: Arc<dyn TerminalSession>, output: Receiver<TerminalOutput> }
SessionManager::attach
  ├─ 分配 SessionId，发出 Live 事件 SessionOpened（journal 插件监听后写入 Journal）
  └─ 输出泵：按时间窗口或字节上限合并输出 → 回放环形缓冲 → OutputSink
src-tauri 的 OutputSink → ipc::Channel → TermView.write(Uint8Array)
输入：TermView.onData → invoke("session_write") → SessionManager → TerminalSession::write
```

会话活在 Rust 侧，UI 只持有 `SessionId`。WebView 重载或面板重新挂载时，通过回放缓冲恢复屏幕内容；这也是会话恢复和"AI 操作终端"的前提。

## SSH 连接复用

`SshConnector` 按 (主机, 端口, 用户) 维护一个连接池。同一主机的多个面板复用同一条 SSH 连接，每个面板开一个 channel；最后一个 channel 关闭后，连接再空闲一段可配置的时长才断开。host key 首次出现时发出 Live 事件，等待 UI 确认；已记录的指纹发生变化时拒绝连接。

## 能力 seam

一个 **seam** 是一项可替换的能力，由三个角色组成：声明接口的 **Definition**（trait）、实现它的 **Provider**、使用它的 **Consumer**。单个角色不构成 seam；新增一项能力时三者一起设计。

| Seam | Definition 所在 | Provider | Consumer |
|---|---|---|---|
| `TerminalSession` | rove-session | rove-pty、rove-ssh | `SessionManager`、AI 终端工具 |
| `KeyCache` | rove-vault | 系统钥匙串（`keyring`）、Android Keystore、iOS Keychain | `Vault` 解锁 |
| `JournalStore` | rove-journal | SQLite | `SessionManager`、会话恢复、审计 |
| `FileSystem` | rove-fs | 本地、SFTP | 传输队列、SFTP 面板、ACP `fs/*`、MCP 文件工具 |
| `SyncBackend` | rove-sync | WebDAV、S3、Gist、自建服务 | 同步引擎 |
| `Policy` | rove-ai | 规则策略、模型审阅 | `exec/pre`、`fs/pre`、`tool/pre` 拦截器 |
| `TailnetService` | rove-tailscale | 内置 tailscale-rs 设备 | 桌面设备列表、SSH 自定义路由 |
| `TaskStore` | rove-ops | 本机 Journal 任务事件 | `OpsSupervisor`、总管和定时调度 |

终端和文件系统绑定在同一个**主机上下文**上：AI 的 `terminal/*` 和 `fs/*` 指向哪台主机，就整体在哪台主机上执行，Provider 不需要为此分叉实现。

## 安全不变量

运维任务先将正文写入本机加密审计，再持久化目标和状态后派发。总管通过任务工具异步编排，单主机 `Assistant` 仍通过 `Policy` 执行。任务范围、全局 worker 许可、状态迁移和恢复规则由 `rove-ops` 提供；应用层适配主机解析、凭据、模型和界面事件。详见 [ops](subsystems/ops.md)。

- 凭据只以密文存储，设备主密钥由本机 `KeyCache` 托管（[vault](subsystems/vault.md)）。
- 启动时静默解锁；解锁失败时应用照常可用，只有需要凭据的操作才请求授权。
- **AI 可见即已记录**：AI 能看到的输入、执行的命令、读写的文件都能从 Journal 还原；唯一的例外是用户主动删除的对话（[ai](subsystems/ai.md) 的 `ai_delete`）。
- Agent 进程发来的任何"始终允许"或模式请求都不参与决策；只有本地 `Policy` 决定是否放行。

威胁模型和密钥层级见 [security.md](security.md)。

## 新行为的归属位置

新行为挂到已有的扩展点上。修改内核语义时，同步更新本表。

| 目标 | 做法 |
|---|---|
| 新增一种终端来源（serial、telnet……） | 新插件实现 `TerminalSession`，并提供自己的打开服务；产物交给 `SessionManager::attach` |
| 新增 UI 可调用的操作 | 在 `src-tauri/src/commands/` 新增命令，只做参数校验和服务调用（[cookbook](cookbook/adding-a-tauri-command.md)） |
| 新增快捷键或菜单项 | 在 `ui/src/lib/actions.ts` 的 actions 表登记（[ui](subsystems/ui.md)） |
| 新增需要持久化的事实 | 定义新的 Journal 事件类型，从日志回放 |
| 观察进行中的工作 | 监听对应的 Live 事件 |
| 限制或审计某项能力 | 在该能力的 waterfall 事件上注册拦截器 |
| 新增一种文件来源 | 实现 `FileSystem`，打开后放入 `FsSessions`；传输和文件面板无需改动（[fs](subsystems/fs.md)） |
| 新增凭据缓存方式 | 实现 `KeyCache` 并在 profile 中替换 |
| 新增一个插件 | 实现 `Plugin`，在 profile 中启用（[cookbook](cookbook/adding-a-plugin.md)） |
| 发现尾网设备 | 从 `TailnetService` 获取节点快照，使用 SSH 自定义传输建立连接 |
| 持续运维任务 | 经 `OpsSupervisor` 持久化并取得 worker 许可，执行仍复用主机工具及审批链 |
