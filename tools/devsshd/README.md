# devsshd

开发用的 SSH 服务器，供集成测试和本地调试使用。它以当前用户身份执行命令，**只能监听本机回环地址**。

## 功能

- 密码认证和公钥认证（只接受配置中的用户名）。
- PTY shell：基于 `rove-pty` 启动真实 shell，支持窗口尺寸变化。
- `exec` 请求：通过 `sh -c` 执行，合并 stdout 和 stderr 后返回，并带上退出码。
- `DevServer::connections()` 统计已接受的 TCP 连接数，用于验证连接复用。

## 命令行

```sh
cargo run -p devsshd -- --addr 127.0.0.1:2222 --user demo --password demo
```

| 参数 | 默认值 | 说明 |
|---|---|---|
| `--addr` | `127.0.0.1:2222` | 监听地址 |
| `--user` | `demo` | 接受的用户名 |
| `--password` | `demo` | 接受的密码 |
| `--host-key` | `target/devsshd_host_key` | 主机私钥路径；文件不存在时生成并保存，保证多次运行的指纹不变 |
| `--authorized-key` | 无 | 接受的公钥文件，可以重复指定 |
| `--shell` | 平台默认 | PTY 会话使用的 shell |

## 在测试中使用

```rust
let server = devsshd::DevServer::start("127.0.0.1:0", devsshd::DevConfig::demo()).await?;
let addr = server.addr();
```

`DevServer` 被 drop 时关闭监听。
