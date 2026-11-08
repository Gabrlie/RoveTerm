# 0001：用 Tauri 2 + Rust 核心构建跨平台应用

## 背景

RoveTerm 需要覆盖 Windows、macOS、Linux 和 Android，同时把内存占用和启动时间做到尽量低。

## 决定

- 核心逻辑（SSH、PTY、加密、同步、AI 协议）全部用 Rust 写成与 UI 无关的 crate。
- UI 用 Tauri 2 + 系统 WebView，前端为 Svelte 5 + xterm.js（WebGL 渲染器）。
- 终端字节通过 `tauri::ipc::Channel` 传输，不走 JSON 事件。

## 放弃的方案

- **Electron**：每个应用自带 Chromium，内存和安装包体积不符合目标。
- **纯 Rust 原生 GPU UI**（GPUI、Slint、egui）：内存和启动最好，但 Android 支持弱，终端渲染器和全套 UI 组件都要自研，周期过长。
- **Flutter + Rust**：Android 体验好，但桌面端内存更高，终端渲染需要自行实现。

## 代价

- 依赖各平台 WebView：Linux 上 WebKitGTK 的 WebGL 性能较弱，需要保留 canvas 渲染作为退路。
- 前后端之间多一层 IPC，需要对终端输出做合并以免 IPC 过载。

## 验证

release 构建的空闲内存和冷启动时间达到 [testing.md](../testing.md#性能基线) 中的目标。
