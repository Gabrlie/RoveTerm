# devsshd

开发用的 SSH 服务器，供集成测试和本地调试使用。它以当前用户身份执行命令，**只能监听本机回环地址**。

## 功能

- 密码认证和公钥认证（只接受配置中的用户名）。
- PTY shell：基于 `rove-pty` 启动真实 shell，支持窗口尺寸变化。
- `exec` 请求：通过 `sh -c` 执行，合并 stdout 和 stderr 后返回，并带上退出码。
- SFTP 子系统：直接读写真实文件系统，相对路径以 `--sftp-root`（默认为当前目录）为基准；不做 chroot，客户端可以访问当前用户能访问的任何路径。
- 本地端口转发（`direct-tcpip`）：服务器代为连接目标地址。
- 远程端口转发（`tcpip-forward`）：无论客户端请求哪个地址，都只绑定 `127.0.0.1`；请求端口 0 时由系统分配。
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
| `--sftp-root` | 当前目录 | SFTP 相对路径的基准目录 |

## 在测试中使用

```rust
let server = devsshd::DevServer::start("127.0.0.1:0", devsshd::DevConfig::demo()).await?;
let addr = server.addr();
```

`DevServer` 被 drop 时关闭监听。
