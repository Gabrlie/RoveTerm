# 术语表

## seam

一项可替换的能力，由 **Definition**（声明接口的 trait）、**Provider**（实现该 trait 的插件）和 **Consumer**（调用方）三个角色组成。单独一个 trait 或一个实现不构成 seam。替换 Provider 即可改变整个产品在这项能力上的行为。

## Plugin（插件）

实现 `rove_kernel::Plugin` 的类型。它在 `apply` 中向 `Context` 注册服务、事件监听器和拦截器，并返回这些注册对应的 `Disposer`。

## Disposer

一次注册的撤销句柄。drop 时撤销对应注册；插件卸载时按注册的逆序 drop。

## Context

`rove-kernel` 提供的共享上下文，包含类型化服务注册表和事件总线。克隆成本很低，所有克隆指向同一份状态。

## Profile

一组启用的插件加一份 TOML 配置，例如 `desktop`、`mobile`、`headless`。

## Journal 事件

追加到 `JournalStore` 的持久事实，重启后仍然存在，用于会话恢复和审计。

## Live 事件

只在进程内广播、不持久化的事件，例如终端输出、host key 确认请求。

## 能力事件 / waterfall

挂在某项能力前面的拦截链。每个拦截器拿到请求和 `next`：调用 `next` 把请求交给下一层，不调用则短路并返回自己的结果。

## 主机上下文

一台目标主机及其连接、终端和文件系统的绑定关系。AI 的终端与文件操作在同一个主机上下文中执行。

## 设备主密钥

每台设备随机生成的 32 字节密钥，只存放在系统钥匙串（或 Android Keystore）中，用于加密本机 vault 中的凭据。它从不参与同步或备份。

## 同步密钥

由用户的同步密码经 Argon2id 派生出的密钥，用于密封要离开本机的数据（同步、导出）。
