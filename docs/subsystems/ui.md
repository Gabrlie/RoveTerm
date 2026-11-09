# ui

`ui/` 是 Svelte 5 + TypeScript 前端：终端视图、文件面板、标签与分屏、主机管理、actions 表、i18n 字典和设计 token。前端只持有 `SessionId` / `HostId` / `FsId`，会话本身活在 Rust 侧。

## 目录

| 路径 | 职责 |
|---|---|
| `src/lib/ipc.ts` | 所有 Tauri 命令和事件的类型化封装；组件不直接调用 `invoke` |
| `src/lib/terminal/TermView.svelte` | 一个 xterm.js 视图绑定一个会话 |
| `src/lib/terminal/theme.ts` | 亮、暗两套终端配色 |
| `src/lib/terminal/registry.ts` | 按面板登记的终端句柄，供复制、粘贴等动作找到聚焦的终端 |
| `src/lib/layout/layout.ts` | 分屏树的纯函数：拆分、移除、调整比例、计算矩形 |
| `src/lib/layout/PaneGrid.svelte` | 渲染一个标签页的全部面板和分隔条 |
| `src/lib/workspace/workspace.svelte.ts` | 标签、分屏树和每个面板的连接目标 |
| `src/lib/tabs/TabBar.svelte` | 标签栏 |
| `src/lib/actions.ts` | actions 表：快捷键、命令面板的唯一来源 |
| `src/lib/ui/` | 通用组件：`Icon`、`IconButton`、`Dialog`、`CommandPalette`；图标映射在 `icons.ts` |
| `src/lib/files/` | SFTP 文件面板 `FileBrowser`、传输状态 `transfers.svelte.ts`、路径与格式化的纯函数 `format.ts` |
| `src/lib/hosts/` | 主机列表与搜索、主机编辑、快速连接、密码输入、host key 确认 |
| `src/lib/vault/vault.svelte.ts` | vault 锁定状态，以及由用户操作触发的解锁和重置 |
| `src/lib/i18n/` | 文案字典与 `t()`、`errorText()` |
| `src/styles/tokens.css` | 设计 token：颜色、间距（4px 网格）、圆角、字号、字体 |

## TermView

`TermView` 接收一个 `open(size, onOutput) => Promise<SessionId>` 函数：挂载后先按容器尺寸计算终端尺寸，再调用 `open` 启动会话，因此 shell 一开始就拿到正确的尺寸。

- 输出通过 IPC Channel 到达：`ArrayBuffer` 直接写入 xterm；`{ exit }` 消息在终端中显示退出提示，并调用 `onexit`。
- 输入通过 `onData` 调用 `sessionWrite`；会话打开之前的输入会被丢弃。
- 容器尺寸变化经 `ResizeObserver` 和 `requestAnimationFrame` 合并后调用 `fit`，行列数变化时才调用 `sessionResize`。
- macOS、Windows 和 Android 上按需动态加载 WebGL 渲染器；Linux 桌面（WebKitGTK）在同时存在多个 GL 上下文时，较早的 WebGL 终端不再重绘，因此使用 DOM 渲染器。WebGL 上下文丢失时退回 DOM 渲染器并立即重绘。
- 每次 `fit` 之后重绘可见区域，避免画布尺寸变化后留白。
- 注册 `attachCustomKeyEventHandler`，应用快捷键不会再发给 shell。
- 跟随系统亮暗主题切换配色。
- 组件提供 `focus()`、`selection()`、`paste(text)` 方法。

## 标签与分屏

- 每个标签页有一棵分屏树：叶子是面板，内部节点是 `split { dir, ratio, a, b }`。`row` 表示左右分屏，`column` 表示上下分屏，`ratio` 被限制在 0.1–0.9。
- `PaneGrid` 把分屏树换算成每个面板的矩形，用绝对定位渲染。面板按 ID 作为 key，拆分或调整大小时终端组件不会重新挂载，会话也不会重启；非活动标签页用 `visibility: hidden` 隐藏，终端保持原有尺寸。
- 拆分面板时，新面板打开与原面板相同的目标；对 SSH 面板，后端会复用已有连接，不需要再次输入凭据。
- 本地 shell 以退出码 0 退出时自动关闭面板；其他情况保留面板并显示退出提示。

## 文件面板

文件面板是一种面板类型（`PaneTarget` 的 `sftp`），和终端一样参与标签与分屏。

- 打开方式：在主机栏中点主机的"浏览文件"按钮，新开一个标签；或在 SSH 终端中按 `App+O`，在右侧分屏打开同一主机。对快速连接的终端同样有效。
- `FileBrowser` 挂载时调用 `sftp_open`，卸载时调用 `sftp_close`。需要密码时走与终端相同的 `ssh.credentials_required` → 密码框 → `workspace.reopen` 流程。
- 双击目录进入，双击文件下载到系统下载目录；路径栏可以直接输入路径。单击选择，Ctrl/⌘ 单击多选，Shift 单击连选。
- 列表获得焦点时：方向键移动选择，Enter 打开，Backspace 返回上级，Delete 删除，F2 重命名，Ctrl/⌘+A 全选。
- 上传通过系统文件选择框（`tauri-plugin-dialog`，只授予 `dialog:allow-open`），或把文件从系统拖到面板上。上传时若当前目录已有同名项，询问替换、保留两者还是跳过；下载总是自动改名，不覆盖本地文件。
- 面板底部列出与该主机相关的传输（`transfer-update` 事件），显示进度并可取消；上传到当前目录的传输完成后自动刷新列表。
- 是否显示隐藏文件保存在 `localStorage`，读写失败时按不显示处理。

## 主机与凭据

- 侧栏按分组列出主机，搜索时匹配名称、地址、用户名、分组和标签；双击或按 Enter 连接，悬停时显示编辑和删除按钮。
- 连接前，如果主机使用"每次询问"认证，或者使用密码认证但没有保存密码，先弹出密码框；用户可以勾选"记住密码"，连接成功后由后端写入 vault。
- 打开面板时如果后端返回 `ssh.credentials_required`，弹出密码框，再用新面板替换原面板重新连接（`workspace.reopen`）。
- 一次性密码只保存在面板状态中，会话打开后立即清除。
- 主机编辑表单在保存失败（例如用户拒绝解锁）时保持打开，已填写的内容不会丢失。留空的秘密字段表示保留已保存的值。
- `hostkey-prompt` 事件按到达顺序排队，逐个弹出确认框，显示算法和 `SHA256:` 指纹。关闭对话框视为拒绝。
- 侧栏底部显示 vault 状态。锁定时点击即请求解锁；设备密钥丢失时，确认后可以重置并丢弃已保存的凭据。
- 快速连接接受 `user@host:port` 等形式；不填密码时使用 SSH agent。

## actions 表

`actions.ts` 是快捷键、命令面板（以及之后的菜单和快捷键设置页）的唯一来源。新增应用快捷键时只在这里登记，不要在组件里单独监听按键。

- 绑定中的 `App` 在 macOS 上展开为 `Meta`（⌘），在其他平台展开为 `Ctrl+Shift`，因此普通的 Ctrl+字母总是交给 shell。
- 按键匹配使用物理按键（`event.code`），不受键盘布局和 Shift 的影响。
- 窗口在捕获阶段分发动作；终端通过 `isAppShortcut` 把这些按键让给应用。
- `setOverrides` 用来替换用户自定义的绑定。

| 动作 | 默认绑定 |
|---|---|
| 新建本地终端 | `App+T` |
| 关闭面板 | `App+W` |
| 向右分屏 / 向下分屏 | `App+D` / `App+E` |
| 聚焦下一个 / 上一个面板 | `App+]` / `App+[` |
| 下一个 / 上一个标签 | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| 切换到第 N 个标签 | `App+1` … `App+9` |
| 复制 / 粘贴 | `App+C` / `App+V` |
| 命令面板 | `App+P` |
| 快速连接 | `App+K` |
| 新建主机 | `App+N` |
| 显示或隐藏主机栏 | `App+B` |
| 在终端旁打开文件面板 | `App+O` |

## 图标

界面图标只使用 Remix Icon 4.8.0（Apache-2.0）的 `-line` 样式，在 `src/lib/ui/icons.ts` 中按含义命名后通过 `?raw` 导入。4.9 及以后的版本改用了另一种许可证，升级前需要先核对许可证。只有图标的按钮统一使用 `IconButton`，`label` 同时作为提示文字和无障碍名称。

## i18n

- `en.ts` 是源字典，定义 `MessageKey`；其他语言实现 `Messages` 类型，缺键会在类型检查时报错。
- `t(key, params)` 替换 `{name}` 占位符；`errorText(error)` 按 `CommandError.code` 查找 `error.<code>` 文案。
- 根据 `navigator.languages` 选择 `zh-CN` 或 `en`。

## 设计 token

组件只读取 `tokens.css` 中的 CSS 变量，不写死颜色、间距和圆角。暗色主题跟随系统，也可以通过 `<html data-theme="light|dark">` 强制指定。
