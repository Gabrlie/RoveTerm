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
| `src/lib/forwards/` | 端口转发对话框 `ForwardsDialog`、转发状态 `forwards.svelte.ts` |
| `src/lib/hosts/` | 主机列表与搜索、主机编辑、快速连接、密码输入、host key 确认 |
| `src/lib/vault/vault.svelte.ts` | vault 锁定状态，以及由用户操作触发的解锁和重置 |
| `src/lib/home/HomePage.svelte` | 首页：主机卡片按最近连接排序，快速连接、新建主机、本地终端入口 |
| `src/lib/panels/` | 工作区面板：右侧 `RightPanel`（`StatusPanel` 服务器状态、`SnippetsPanel` 快捷指令），底部 `BottomPanel`（文件） |
| `src/lib/keys/` | 密钥库对话框 `KeysDialog` 与密钥列表 `keys.svelte.ts` |
| `src/lib/settings/` | 偏好设置 `prefs.svelte.ts`（启动时在挂载前加载）与设置对话框 `SettingsDialog` |
| `src/lib/i18n/` | 文案字典与 `t()`、`errorText()` |
| `src/styles/tokens.css` | 设计 token：表面与材质、细线、阴影、强调色派生、状态色、间距（4px 网格）、圆角、字号、字体、动效 |
| `src/styles/controls.css` | 共享控件样式：按钮（`primary` / `secondary` / `plain`）、文本框、滚动条、分组表单（`form-group` / `form-row`） |
| `src/lib/terminal/schemes.ts`、`SCHEMES.md` | 终端配色目录与第三方配色来源 |
| `src/styles/fonts.css`、`src/assets/fonts/` | 内置的 Symbols Nerd Font Mono 及其许可说明 |

## TermView

`TermView` 接收一个 `open(size, onOutput) => Promise<SessionId>` 函数：挂载后先按容器尺寸计算终端尺寸，再调用 `open` 启动会话，因此 shell 一开始就拿到正确的尺寸。

- 输出通过 IPC Channel 到达：`ArrayBuffer` 直接写入 xterm；`{ progress }` 消息由 `connectLog.ts` 格式化为一行连接日志（连接、握手、host key、逐个凭据的尝试与拒绝、认证成功）写入终端，连接成功后在第一段终端输出到达时清空，连接失败时保留以便查看停在哪一步；`{ exit }` 消息在终端中显示退出提示，并调用 `onexit`。
- 输入通过 `onData` 调用 `sessionWrite`；会话打开之前的输入会被丢弃。
- 面板在连接完成前被关闭时，稍后返回的会话立即由 `sessionClose` 关闭，之后到达的输出和错误都被忽略，不会留下后台会话。
- 容器尺寸变化经 `ResizeObserver` 和 `requestAnimationFrame` 合并后调用 `fit`，行列数变化时才调用 `sessionResize`。
- macOS、Windows 和 Android 上按需动态加载 WebGL 渲染器；Linux 桌面（WebKitGTK）在同时存在多个 GL 上下文时，较早的 WebGL 终端不再重绘，因此使用 DOM 渲染器。WebGL 上下文丢失时退回 DOM 渲染器并立即重绘。
- 每次 `fit` 之后重绘可见区域，避免画布尺寸变化后留白。
- 注册 `attachCustomKeyEventHandler`，应用快捷键不会再发给 shell。
- 终端配色按界面亮暗分别取偏好中的 `scheme_light` / `scheme_dark`（`theme.ts`），两个槽位都可以选任意方案，未知 id 退回同亮度的 RoveTerm 配色；终端周围的 `--term-bg` / `--term-fg` 跟随所选方案。修改偏好后已打开的终端立即换色。
- 字体、字号、行高取自偏好设置，修改后对已打开的终端立即生效。字体栈依次为用户选择的字体、`--font-mono` 内置字体栈（以 `monospace` 结尾）、内置的 Symbols Nerd Font Mono；符号字体放在最后，因为第一个可用字体决定行高度量。创建终端和切换字体前先等待字体加载，避免字形在加载前被测量或缓存。
- 组件提供 `focus()`、`selection()`、`paste(text)` 方法。

## 标签与分屏

- 每个标签的图标右下角有状态点，取自当前聚焦的面板：绿色为已连接，黄色闪烁为连接中，红色为失败，灰色为已退出。关闭按钮只在悬停和当前标签上显示。
- 标签栏最左侧固定一个"主页"标签（`HOME_TAB`），启动时显示它；关闭最后一个标签后回到主页；"+"按钮也打开主页，`App+T` 直接新建本地终端。`Ctrl+Tab` 循环时主页排在最前。
- 首页列出已保存主机，按 `last_connected_at` 降序排列，从未连接的按名称排在后面；卡片显示经由图标、运行中的转发数和"x 分钟前连接"，点击连接，悬停显示浏览文件和编辑。终端连上已保存主机后刷新主机列表，排序随之更新。

- 每个标签页有一棵分屏树：叶子是面板，内部节点是 `split { dir, ratio, a, b }`。`row` 表示左右分屏，`column` 表示上下分屏，`ratio` 被限制在 0.1–0.9。
- `PaneGrid` 把分屏树换算成每个面板的矩形，用绝对定位渲染。面板按 ID 作为 key，拆分或调整大小时终端组件不会重新挂载，会话也不会重启；非活动标签页用 `visibility: hidden` 隐藏，终端保持原有尺寸。
- 拆分面板时，新面板打开与原面板相同的目标；对 SSH 面板，后端会复用已有连接，不需要再次输入凭据。
- 本地 shell 以退出码 0 退出时自动关闭面板；其他情况保留面板并显示退出提示。

## 文件面板

文件面板是一种面板类型（`PaneTarget` 的 `sftp`），和终端一样参与标签与分屏。

- 打开方式：在主机栏中点主机的"浏览文件"按钮，新开一个标签；或在 SSH 终端中按 `App+O`，在右侧分屏打开同一主机。对快速连接的终端同样有效。
- `FileBrowser` 挂载时调用 `sftp_open`，卸载时调用 `sftp_close`。需要密码时走与终端相同的 `ssh.credentials_required` → 密码框 → `workspace.reopen` 流程。
- 文件图标按类型取色：目录用强调色，压缩包、图片、脚本与源码、配置、文本各用一种系统色，其他为灰色。列表获得焦点时选中行为强调色实底，失去焦点时为灰底。
- 双击目录进入，双击文件下载到系统下载目录；路径栏可以直接输入路径。单击选择，Ctrl/⌘ 单击多选，Shift 单击连选。
- 列表获得焦点时：方向键移动选择，Enter 打开，Backspace 返回上级，Delete 删除，F2 重命名，Ctrl/⌘+A 全选。
- 上传通过系统文件选择框（`tauri-plugin-dialog`，只授予 `dialog:allow-open`），或把文件从系统拖到面板上。上传时若当前目录已有同名项、下载时若本地目标目录已有同名项（由 `transfer_local_clashes` 检查），都询问替换、保留两者还是跳过。
- 面板底部列出与该主机相关的传输（`transfer-update` 事件），显示进度并可取消；上传到当前目录的传输完成后自动刷新列表。
- 是否显示隐藏文件取自偏好 `files.show_hidden`，工具栏按钮切换时写回偏好。

## 端口转发

- 在主机栏中点主机的"端口转发"按钮，或在命令面板中运行"管理当前主机的端口转发"，打开该主机的转发对话框。
- 对话框列出规则：类型（L / R / D）、地址、状态（运行中及连接数、失败原因、已停止），可以启动、停止、编辑、删除。修改规则会停止正在运行的转发。
- 编辑器按类型显示字段和说明；本地或动态转发的监听地址不是回环地址时显示警告。勾选"自动启动"的规则在打开该主机的终端时启动。
- 启动时需要密码（`ssh.credentials_required`）就在该规则下显示密码框，可以勾选记住密码。
- `forwards.svelte.ts` 监听 `forward-update` 事件；主机栏在有运行中的转发时，在主机名旁显示转发数。

## 会话恢复

标签与分屏变化后 1.5 秒，把 `workspace.snapshot()` 经 `workspace_save` 记入 journal。快照（`src/lib/workspace/snapshot.ts`）记录每个标签的分屏树、各面板的目标（本地、已保存主机 id、快速连接地址、文件面板）、聚焦面板和当前标签，不含任何凭据。启动时 `workspace_load` 读取最新快照，经 `parseSnapshot` 校验后恢复；恢复的面板先处于 `deferred` 状态，只显示标题，所在标签第一次显示时才连接（无论通过点击、快捷键切换，还是关闭相邻标签后自动切换），因此不会同时发起多条连接或弹出多个密码框；快照中的当前标签立即激活。

## 工作区面板

终端区域右侧和下方各有一个可开关、可拖动边缘调整大小的面板，开关状态和尺寸按设备记在 localStorage。它们都跟随当前聚焦面板所在的远程主机（`remoteSourceOf`），主机变化时重新挂载；聚焦本地终端或首页时显示提示。

- 右侧面板（`App+L` 或标签栏右侧按钮）：
  - 状态：每 3 秒调用 `server_stats` 采样，用两次采样的差值计算使用率和速率，分卡片显示：系统（主机名、操作系统、响应时间、运行时间、内核；经跳板机时响应时间按段显示为 `12+85 ms`，10 ms 以下保留一位小数，标签后用小字注明各段，如"（跳板 + 目标）"）、CPU（平均使用率、负载，以及每核最近 2 分钟使用率的折线图 `CoreChart`：每行放尽量多的图（每图至少约 110px，超过 8 核时改为只显示数值的紧凑图），再均分各行，如窄面板里 4 核排成 2 × 2）、内存（物理内存与交换分区）、网络（各网卡上下行速率，按流量排序，超过 6 个时可展开）、磁盘（每个挂载分区的占用）。只使用已有连接，没有连接或非 Linux 服务器时显示说明。第一次采样返回前显示与正式卡片同结构的骨架屏（通用类 `.skeleton`，带扫光动画），数据到达后直接替换。每台主机最近一次采样、CPU 历史和速率按 `sourceKey` 缓存在模块级（随窗口存活），切换面板或重新打开右侧面板时立即显示缓存内容，再继续刷新。
  - 快捷指令：搜索、新建、编辑、删除；"插入"以 bracketed paste 写入聚焦终端（不执行），"运行"把命令（多行按行）连同回车一次写入会话，保证回车不会先于命令到达。
- 底部面板（`App+J` 或标签栏右侧按钮）：聚焦终端连接成功后，用 `FileBrowser` 显示其所在主机的文件，复用同一条共享连接，不会自行发起连接或询问密码。聚焦的是文件面板时不重复显示。面板没有单独的标题行，关闭按钮在文件工具栏最右侧（`FileBrowser` 的 `onclose`）。最近显示过的 4 台主机各保留一个 `FileBrowser` 实例，不在当前主机的隐藏而不卸载，切换回来时目录、列表和选择保持原样，超出时关闭最早的一个；底部面板首次打开后一直挂载，关闭只是隐藏。

## 密钥库

主机栏顶部的钥匙按钮或命令面板的"密钥库"打开 `KeysDialog`：左侧是密钥列表和"生成密钥""导入密钥"入口，右侧显示选中的密钥。

- 生成：名称、类型（Ed25519、ECDSA P-256、RSA 3072）、注释、可选的私钥密码。
- 导入：粘贴私钥或从文件读取（`key_read_file`），从文件读取时自动读取同名 `.pub` 文件作为公钥；公钥可留空（使用从私钥推出的公钥，保留私钥中的注释）或自行填写；加密私钥缺少密码时高亮密码框。
- 编辑公钥：在详情中修改公钥（例如改注释），保存前校验与私钥配对。
- 查看：算法、指纹、公钥（可复制）；"显示私钥"解锁后调用 `key_reveal` 显示私钥，可复制。
- 改名、删除（仍被主机使用时提示失败原因）。
- 安装公钥到主机：选择已保存主机后调用 `key_install`；主机还不能登录时显示密码框，以 `password_kind = "password"` 连接。
- 主机编辑的认证方式"密钥库中的密钥"用下拉框选择密钥，旁边的"管理密钥库…"在编辑框之上打开密钥库，关闭后列表自动更新。每台主机单独保存私钥的旧方式只对已在使用它的主机显示。

确认操作统一使用 `ask()`（`src/lib/ui/confirm.svelte.ts`），由 `ConfirmHost` 渲染为主题化的对话框，不使用浏览器的 `confirm()`。

## 设置

主机栏底部的齿轮按钮或 `App+,` 打开设置对话框。左侧是分区导航，右侧是分组表单（`form-group` / `form-row`），每个分区一个组件（`src/lib/settings/sections/`）。修改立即生效：`editPrefs()` / `editDevice()`（`edit.svelte.ts`）修改一份副本后交给后端校验保存，被拒绝时在底部显示原因，不需要统一保存。

| 分区 | 内容 |
|---|---|
| 外观 | 主题（自动 / 浅色 / 深色缩略图）、强调色色板、界面文字大小、语言、半透明窗口（本机） |
| 终端 | 字体（从 `fonts_monospace` 列出的本机等宽字体中选，已保存但不再安装的仍保留）、字号、行高与预览；浅色与深色模式配色（带色卡）；光标样式与闪烁、滚动缓冲、选中即复制、右键行为、响铃 |
| 文件 | 显示隐藏文件、删除前确认、同名文件的默认处理、下载位置（本机）、同时传输数（本机） |
| 服务器状态 | 刷新间隔 |
| 快捷键 | 只读列出 actions 表，可按名称或按键搜索 |
| 关于 | 版本与许可说明 |

各项在使用处读取 `prefs`：终端的光标、滚动缓冲、选中即复制、右键（菜单：复制、粘贴、全选、清屏；或直接粘贴）、响铃（短暂的强调色闪烁）由 `TermView` 应用，修改后对已打开的终端立即生效；文件面板的同名处理不为"每次询问"时直接按设定执行，关闭删除确认后直接删除，下载默认存到设定的下载位置；状态面板按刷新间隔采样。

## 主机与凭据

- 主机前的头像（`src/lib/ui/Avatar.svelte`）取名称的首字母，颜色由名称稳定地选自系统色板 `--sys-*`（`src/lib/ui/avatar.ts`），侧栏和主页共用。
- 侧栏按分组列出主机，搜索时匹配名称、地址、用户名、分组和标签；双击或按 Enter 连接。悬停时只显示"连接"和"更多"两个按钮，右键、"更多"或 ContextMenu 键打开菜单：连接、浏览文件、端口转发、编辑、复制主机（不复制已保存的凭据）、删除。
- 经跳板机或代理连接的主机在名称后显示小图标，悬停提示经由的跳板机或代理，不占用地址那一行。
- 标题栏右侧的布局按钮或 `App+B` 收起和展开侧栏；是否展开按设备记在 localStorage。侧栏顶部直接是搜索框，快速连接、新建主机、导入和密钥库都在应用菜单里。
- 连接前，如果主机使用"每次询问"认证，或者使用密码认证但没有保存密码，先弹出密码框；用户可以勾选"记住密码"，连接成功后由后端写入 vault。
- 终端打开失败时记录到面板的 `failure`，在面板底部显示失败原因和操作：重试、输入私钥密码（缺少 passphrase 时）、使用密码登录（对密钥主机以 `password_kind = "password"` 重连，这种密码不能记住）、编辑主机、关闭。
- 打开面板时如果后端返回 `ssh.credentials_required` 或 `ssh.passphrase_required`，弹出密码或私钥密码框，再用新面板替换原面板重新连接（`workspace.reopen`）。密钥主机可以勾选"记住私钥密码"，成功后存为 passphrase。
- 一次性密码只保存在面板状态中，会话打开后立即清除。
- 主机编辑表单在保存失败（例如用户拒绝解锁）时保持打开，已填写的内容不会丢失。留空的秘密字段表示保留已保存的值。
- 主机编辑表单的"连接方式"可选直连、经跳板机（从其他已保存主机中选择）、HTTP 代理或 SOCKS5 代理（填写地址和端口）。经跳板机连接时，跳板机和目标的 host key 依次确认。
- `hostkey-prompt` 事件按到达顺序排队，逐个弹出确认框，显示算法和 `SHA256:` 指纹。关闭对话框视为拒绝。
- 侧栏底部显示 vault 状态。锁定时点击即请求解锁；设备密钥丢失时，确认后可以重置并丢弃已保存的凭据。
- 快速连接接受 `user@host:port` 等形式；不填密码时使用 SSH agent 和默认密钥文件，都不可用时询问密码。
- 读取私钥、agent、网络和 SFTP 类错误会在提示文字后附上后端给出的原因（如文件路径）。

## actions 表

`actions.ts` 是快捷键、命令面板、应用菜单和设置页快捷键列表的唯一来源。菜单结构在 `src/lib/menu.ts`（文件、编辑、视图、主机、帮助），每项只引用动作 id，文字和快捷键都取自动作表。新增应用快捷键时只在这里登记，不要在组件里单独监听按键。

- 绑定中的 `App` 在 macOS 上展开为 `Meta`（⌘），在其他平台展开为 `Ctrl+Shift`，因此普通的 Ctrl+字母总是交给 shell。
- 按键匹配使用物理按键（`event.code`），不受键盘布局和 Shift 的影响。
- 窗口在捕获阶段分发动作；终端通过 `isAppShortcut` 把这些按键让给应用。
- 标记了 `yieldsToTextField` 的动作（复制、粘贴）在终端以外的输入框获得焦点时不触发，交给浏览器的原生编辑行为。
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
| 设置 | `App+,` |
| 状态与快捷指令面板 | `App+L` |
| 文件面板 | `App+J` |
| 快速连接 | `App+K` |
| 新建主机 | `App+N` |
| 显示或隐藏主机栏 | `App+B` |
| 在终端旁打开文件面板 | `App+O` |
| 管理当前主机的端口转发 | 无（命令面板） |
| 从 ssh_config 导入、快捷键、关于 | 无（菜单与命令面板；后两者打开设置对话框的对应分区） |

## 图标

界面图标只使用 Lucide（`lucide-static` 1.61.0，ISC 许可），在 `src/lib/ui/icons.ts` 中按含义命名后通过 `?raw` 导入；`Icon` 统一以 1.75 线宽、圆头圆角描边绘制，颜色取 `currentColor`。只有图标的按钮统一使用 `IconButton`，`label` 同时作为提示文字和无障碍名称。

右键菜单统一使用 `ContextMenu`：在指定坐标弹出并移到 `<body>`，自动避开窗口边缘，支持方向键和 Enter，选中、Escape、点击别处或窗口缩放时关闭。

对话框统一使用 `Dialog`：打开时聚焦第一个输入控件，关闭时把焦点还给打开前的元素（通常是终端）；Escape 在窗口上监听，只关闭最内层的对话框，因此对话框内的焦点元素消失后仍然有效。

下拉选择统一使用 `src/lib/ui/Select.svelte`，不直接写原生 `<select>`：它按主题 token 绘制，列表以固定定位弹出，不会被对话框的滚动区域裁剪；支持方向键、Home/End、Enter 选择，Escape 只关闭列表。

## i18n

- `en.ts` 是源字典，定义 `MessageKey`；其他语言实现 `Messages` 类型，缺键会在类型检查时报错。
- `t(key, params)` 替换 `{name}` 占位符；`errorText(error)` 按 `CommandError.code` 查找 `error.<code>` 文案。
- 根据 `navigator.languages` 选择 `zh-CN` 或 `en`。

## 设计 token

组件只读取 `tokens.css` 中的 CSS 变量，不写死颜色、间距、圆角、阴影和动效时长。暗色主题跟随系统，也可以通过 `<html data-theme="light|dark">` 强制指定。

- 表面分层：`--window`（窗口底色）之上是半透明材质 `--material-sidebar` / `--material-bar` / `--material-popover`，配合 `--material-blur` 做毛玻璃；内容区用不透明的 `--surface` / `--surface-raised`。WebKitGTK 接受但不绘制 `backdrop-filter`，所以 Linux 上 `<html data-blur="off">`，弹层材质改为不透明。
- 窗口材质：启动时调用 `window_set_material`（`src/lib/ui/window.ts`），macOS 26 及以上用 Liquid Glass，更早的 macOS 用侧栏毛玻璃（vibrancy），Windows 11 用 Mica；Windows 10、Linux 和移动端返回 `false`，保持不透明。生效时 `<html data-material="on">`，`--window` 变为透明，侧栏和工具栏只做淡色叠加，内容区和终端仍不透明。窗口为此在 `tauri.macos.conf.json` / `tauri.windows.conf.json` 中设为 `transparent`。
- 标题栏以下，侧栏、工作区（标签栏、终端区与底部面板）、右侧面板是浮动的圆角卡片（`--radius-lg`），彼此及与窗口边缘相隔 `--gap-panel`，轮廓为 `--panel-shadow`；间隙露出窗口底色或系统材质。工作区与右侧面板之间的间隙本身就是拖动调整宽度的把手，悬停时显示强调色细线。
- 窗口顶部是 `TitleBar`（`src/lib/layout/TitleBar.svelte`，高 38px），空白处带 `data-tauri-drag-region`，可拖动、双击最大化；右侧是侧栏、底部面板、右侧面板三个布局开关。
  - Windows 与 Linux 不用系统标题栏（`decorations: false`，见 `tauri.windows.conf.json` / `tauri.linux.conf.json`）：左侧是 `menu.ts` 定义的文字菜单，点击展开、展开时指向其他菜单名即切换、左右方向键在菜单间移动；最右侧是自绘的最小化、最大化/还原、关闭按钮，最大化状态随窗口缩放更新。Windows 11 悬停最大化按钮时的贴靠布局弹窗因此不可用。
  - macOS 使用覆盖式标题栏（`titleBarStyle: Overlay`，隐藏标题），红绿灯画在标题栏左侧（`trafficLightPosition` 为 16, 19），标题栏左侧留出 80px，不显示文字菜单。菜单进入系统菜单栏：界面启动和切换语言时调用 `menu_set`，传入 `nativeMenus()`（`menu.ts` 中除编辑外的菜单，文字为当前语言）和系统项文字；应用菜单含关于、设置、隐藏、退出，编辑菜单使用系统预定义项（保证输入框原生复制粘贴），另有窗口菜单。选中动作项时后端发出 `menu-action` 事件，界面核对是动作表中的 id 后执行。原生菜单项不设快捷键，按键仍由动作表处理，避免与自定义绑定冲突或重复触发。
- 分隔一律用半透明细线 `--separator`（常以 0.5px 的 `box-shadow` 绘制），`--border` 只给输入类控件描边。
- 强调色只设 `--accent`，`--accent-hover`、`--accent-soft`、`--focus-ring` 等由 `color-mix()` 派生；状态色有 `--success` / `--warning` / `--danger` 及其 soft 版本。
- `--shadow-sm|md|lg` 是多层柔和阴影，`--glass-edge` 是玻璃边缘高光；动效用 `--ease` 与 `--dur-fast|dur|dur-slow`，系统要求减少动效时时长为 0。
- 外观偏好由 `applyAppearance()`（`src/lib/settings/appearance.ts`）在偏好加载和保存后应用：主题写入 `<html data-theme>`（跟随系统时移除），并通过 `setTheme` 让原生窗口外观跟随（Mica 与 macOS 材质取的是窗口外观，否则深色界面后面会透出浅色材质），非蓝色的强调色把 `--accent` 指向 `--sys-*`，界面缩放写入 `--ui-scale`（文字与控件高度按它缩放，终端字号不受影响），语言交给 `setLocale()`。
- 文案目录放在响应式状态 `locale`（`src/lib/i18n/locale.svelte.ts`）里，`t()` 读取它，切换语言后已渲染的界面随之更新，不需要重新挂载。
- 旧名 `--bg`、`--bg-elevated`、`--bg-sunken` 暂作别名保留。
- `controls.css` 的选择器都包在 `:where()` 里，没有优先级，组件自己的规则总能覆盖；自带外框的输入框（搜索框、路径栏）加 `class="bare"` 去掉文本框外观。开关用 `src/lib/ui/Switch.svelte`。
