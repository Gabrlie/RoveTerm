# 0002：用 russh 实现 SSH

## 背景

SSH 是核心能力，需要支持多 channel 复用、端口转发、SFTP，并能交叉编译到 Android。

## 决定

使用纯 Rust、基于 tokio 的 `russh` 实现 SSH 客户端（以及测试用的 devsshd 服务器），SFTP 使用 `russh-sftp`。

## 放弃的方案

- **libssh2 / libssh 绑定**：依赖 C 库和 OpenSSL，Android 和 Windows 交叉编译复杂，异步集成差。
- **调用系统 `ssh` 命令**：Windows 和 Android 上不一定可用，无法做连接复用和精细的 host key 交互。

## 代价

- russh 的 API 变化较快，升级时需要跟进。
- 部分少见的认证方式（GSSAPI 等）不支持。

## 验证

集成测试连接 devsshd 覆盖密码、密钥认证、host key 确认和连接复用；`rove-ssh` 能编译到 `aarch64-linux-android`。
