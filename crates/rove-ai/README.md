# rove-ai

RoveTerm 中的 AI Agent：ACP 客户端、审批策略和绑定主机的工具。Agent 是不可信的外部进程，它在主机上的每个操作都先经过 `Policy`。设计理由见 [0006](../../docs/decisions/0006-ai-acp.md)，参考见 [docs/subsystems/ai.md](../../docs/subsystems/ai.md)。

## 契约

- `shell::parse(line)` 把命令行拆成流水线、简单命令、去掉引号的词、重定向和嵌套命令文本（`$(…)`、反引号、子 shell）。不执行也不展开任何内容；无法静态确定的词标为 `dynamic`。
- `risk::assess_command(line, rules)` 给出 `Assessment { risk, programs, opaque, reasons }`：逐段评估，取最高的 `Risk`（`ReadOnly` < `Modify` < `Dangerous`）。不认识的程序一律为 `Modify`；展开、脚本和解释器代码使结果 `opaque`。
- `assess_read(path)`：读取秘密文件（私钥、`/etc/shadow`、云凭据、`.env` 等）为 `Modify`，其他为 `ReadOnly`。`assess_write(path)`：写系统路径和设备为 `Dangerous`，其他为 `Modify`。
- `Policy::decide(operation, assessment)` 按 `Mode` 返回 `Allow` 或 `Ask { rememberable }`；`Dangerous` 总是询问且不可记住。`Policy::answer` 记录用户的回答，`AllowSession` 只在可记住时生效：命令按程序名、写入按目录、读取按文件、Agent 自带工具按种类。
- `Rules` 是用户追加的只读程序名和危险文本，由 `validate()` 校验。
- `Host` 是 AI 会话所在的主机：`LocalHost`（本机，`sh -c`，Windows 上 `cmd /C`）或 `RemoteHost`（经 `ConnectionSource` 取应用已有的连接，用 exec 通道运行命令、SFTP 读写文件；没有连接时返回 `NotConnected`，从不自行连接或询问凭据）。
- `run` 在 `cwd`（默认登录目录）中运行命令，每个输出流最多保留 `Limits.output` 字节，超过 `Limits.timeout` 返回 `Timeout`（本机进程随之被结束）。`read_text` 最多读 `Limits.file` 字节并标出截断，含 NUL 的文件返回 `Binary`；`write_text` 超过上限返回 `TooLarge`；`list_dir` 按名称排序，最多 `Limits.entries` 项。

- `acp`：ACP 客户端（协议版本 1，按行分隔的 JSON-RPC）。`spawn` 以给定的完整环境启动 Agent（`agent_env` 按 `ENV_ALLOWED` 白名单从本进程取变量，再加上调用方给的变量），stderr 只进日志；`connect` 在任意读写流上工作（测试用）。声明 `fs.readTextFile`、`fs.writeTextFile`、`terminal` 能力。来自 Agent 的消息单条最大 8 MiB，文本按 256 KiB 截断，路径必须是绝对路径，参数在交给 `Client` 前校验。需要登录的请求返回 `AuthRequired`。
- `session::AiSession` 实现 `Client`：每个操作先评估、由 `Policy` 决定，需要时发出 `AiEvent::Approval` 并等待 `answer`；执行后发出 `AiEvent::Executed`（命令输出、文件字节数与 SHA-256、写入前后的哈希与行数、拒绝或失败）。Agent 自带工具的权限请求按其命令（`rawInput.command`）与涉及的文件评估，只选择 Agent 的一次性选项。只接受绑定的 ACP 会话 id。
- `mcp`：内置 MCP server（协议 2024-11-05 至 2025-06-18），工具 `run_command`、`read_file`、`write_file`、`list_dir`、`terminal_snapshot`，全部经 `AiSession` 审批并在会话主机上执行；路径必须是绝对路径。`McpListener` 在回环随机端口上监听，连接须先发送 token 行（10 秒内，常数时间比较）；`bridge_env` 给出桥接进程的环境变量。`bridge()` 是应用二进制 `--mcp-bridge` 模式的实现：连上应用后复制 stdin / stdout。
- `AiSession::set_terminal_source` 提供用户终端的最近内容，`read_terminal` 读取它（只读操作，同样记录）。
- `audit::Auditor` 把一个 AI 会话写成 Journal 记录（`session_id` 为 AI 会话 id，payload 带主机）：开始与结束、用户提示、回复与思考（按消息合并）、审批的请求与回答、每个已执行或被拒绝的操作、Agent 自带工具完成时的报告、回合结束。每个文本字段最多保留 16 KiB，超出时标出 `clipped` 并给出完整内容的 SHA-256。
- `runner::start` 启动 Agent 并打开会话：需要登录时用 `AgentSpec.auth_method` 认证后重试，切换到 `preferred_modes` 中第一个可用的模式。`Running::cancel` 撤回等待中的审批并通知 Agent；`stop` 结束运行中的命令和 Agent 进程。

## 局限

- 分级是启发式：混淆写法可能低估风险，因此只有明确识别为只读的命令才会自动放行。
- `ssh host cmd` 中的远程命令按同一规则评估，但不知道它在哪台主机上执行。
- 远程命令超时时只是放弃等待，服务器上的进程不一定随之结束。
- 本机在 Windows 上用 `cmd` 运行命令，而分级按 POSIX shell 语法进行。
- `terminal/*` 的命令结束后才有输出，期间 `terminal/output` 为空。
