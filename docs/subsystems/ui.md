# ui

`ui/` 是 Svelte 5 + TypeScript 前端：终端视图、标签与分屏、主机管理、actions 表、i18n 字典和设计 token。前端只持有 `SessionId` / `HostId`，会话本身活在 Rust 侧。

## 目录

| 路径 | 职责 |
|---|---|
| `src/lib/ipc.ts` | 所有 Tauri 命令和事件的类型化封装；组件不直接调用 `invoke` |
| `src/lib/terminal/TermView.svelte` | 一个 xterm.js 视图绑定一个会话 |
| `src/lib/terminal/theme.ts` | 亮、暗两套终端配色 |
| `src/lib/i18n/` | 文案字典与 `t()`、`errorText()` |
| `src/styles/tokens.css` | 设计 token：颜色、间距（4px 网格）、圆角、字号、字体 |

## TermView

`TermView` 接收一个 `open(size, onOutput) => Promise<SessionId>` 函数：挂载后先按容器尺寸计算终端尺寸，再调用 `open` 启动会话，因此 shell 一开始就拿到正确的尺寸。

- 输出通过 IPC Channel 到达：`ArrayBuffer` 直接写入 xterm；`{ exit }` 消息在终端中显示退出提示，并调用 `onexit`。
- 输入通过 `onData` 调用 `sessionWrite`；会话打开之前的输入会被丢弃。
- 容器尺寸变化经 `ResizeObserver` 和 `requestAnimationFrame` 合并后调用 `fit`，行列数变化时才调用 `sessionResize`。
- WebGL 渲染器按需动态加载，不可用时 xterm 使用 DOM 渲染器。
- 跟随系统亮暗主题切换配色。
- 组件提供 `focus()`、`selection()`、`paste(text)` 方法。

## i18n

- `en.ts` 是源字典，定义 `MessageKey`；其他语言实现 `Messages` 类型，缺键会在类型检查时报错。
- `t(key, params)` 替换 `{name}` 占位符；`errorText(error)` 按 `CommandError.code` 查找 `error.<code>` 文案。
- 根据 `navigator.languages` 选择 `zh-CN` 或 `en`。

## 设计 token

组件只读取 `tokens.css` 中的 CSS 变量，不写死颜色、间距和圆角。暗色主题跟随系统，也可以通过 `<html data-theme="light|dark">` 强制指定。
