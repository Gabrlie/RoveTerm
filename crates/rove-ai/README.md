# rove-ai

RoveTerm 中的 AI Agent：内置助手（主力）、ACP 客户端（兼容通道，只保证通用能力）、审批策略、审阅者和绑定主机的工具。Agent 是不可信的外部进程，它在主机上的每个操作都先经过 `Policy`。设计理由见 [0006](../../docs/decisions/0006-ai-acp.md)，参考见 [docs/subsystems/ai.md](../../docs/subsystems/ai.md)。

## 契约

- `shell::parse(line)` 把命令行拆成流水线、简单命令、去掉引号的词、重定向和嵌套命令文本（`$(…)`、反引号、子 shell）。不执行也不展开任何内容；无法静态确定的词标为 `dynamic`。
- `risk::assess_command(line, rules)` 给出 `Assessment { risk, programs, opaque, reasons }`，取各命令段的最高风险（`ReadOnly` < `Modify` < `Dangerous`）。静态规则仅用于直接放行明确只读的命令、标注风险原因与约束会话级记住权限，不在自动/无人值守模式下先于 AI 审阅拦截。执行内容无法静态查看（外部脚本、动态程序名、嵌套过深等）时标记 `opaque`，命令行内联脚本（如 `python3 -c`、`python3 - <<'PY'`、`python3 - <<<`）与普通参数展开按可见命令评估。动态输出重定向和软件包管理变更分别给出 `dynamic_target` 和 `package` 原因，与危险/不透明标注一并交给审阅者判断。
- `assess_read(path)`：读取秘密文件（私钥、`/etc/shadow`、云凭据、`.env` 等）为 `Modify`，其他为 `ReadOnly`。`assess_write(path)`：写系统路径和设备为 `Dangerous`，其他为 `Modify`。
- `Policy::decide(operation, assessment)` 按 `Mode` 返回 `Allow` 或 `Ask { rememberable }`。`Policy::answer` 记录用户的回答，`AllowSession` 按程序名记住命令、按目录记住写入、按文件记住读取、按种类记住 Agent 自带工具。危险或不透明操作、可见内联脚本、动态输出重定向与通过 shell 读取秘密文件逐次判断。
- `Rules` 是用户追加的只读程序名和危险文本，由 `validate()` 校验。
- `Host` 是 AI 会话所在的主机：`LocalHost`（本机，`sh -c`，Windows 上 `cmd /C`）或 `RemoteHost`（经 `ConnectionSource` 取连接，用 exec 通道运行命令、SFTP 读写文件；应用以保存的凭据在需要时后台连接，从不询问用户，拿不到连接时返回 `Unreachable` 及原因）。
- `run` 在 `cwd`（默认登录目录）中运行命令，每个输出流最多保留 `Limits.output` 字节，超过 `Limits.timeout` 返回 `Timeout`（本机进程随之被结束）。`read_text` 最多读 `Limits.file` 字节并标出截断，含 NUL 的文件返回 `Binary`；`write_text` 超过上限返回 `TooLarge`；`list_dir` 按名称排序，最多 `Limits.entries` 项。

- `acp`：ACP 客户端（协议版本 1，按行分隔的 JSON-RPC）。`spawn` 以给定的完整环境启动 Agent（`agent_env` 按 `ENV_ALLOWED` 白名单从本进程取变量，再加上调用方给的变量），stderr 只进日志；`connect` 在任意读写流上工作（测试用）；`spawn_for_auth` 仅用于探测 `initialize` 与执行 `authenticate`，拒绝全部文件、终端和权限回调。声明 `fs.readTextFile`、`fs.writeTextFile`、`terminal`、`auth.terminal` 与 `_meta["terminal-auth"]` 能力，`parse_auth_methods` 区分 `agent`、`terminal` 与 `env_var` 登录方式。来自 Agent 的消息单条最大 8 MiB，文本按 256 KiB 截断，路径必须是绝对路径，参数在交给 `Client` 前校验。需要登录的请求返回 `AuthRequired`。
- `session::AiSession` 实现 `Client`：每个操作先评估、由 `Policy` 决定，需要时发出 `AiEvent::Approval` 并等待 `answer`；执行后发出 `AiEvent::Executed`（命令输出、文件字节数与 SHA-256、写入前后的哈希与行数、拒绝或失败）。Agent 自带工具的权限请求按其命令（`rawInput.command`，`bash -lc` 按脚本本身）与涉及的文件评估，只给出调用 id 时用之前报告的字段补全，只选择 Agent 的一次性选项；对 RoveTerm 自己 MCP 工具的请求直接放行。只接受绑定的 ACP 会话 id（可重新绑定）；`set_replaying` 期间不上报 Agent 的更新。整文件替换要求两次读取的完整快照一致，仅文件不存在时允许按新建处理。
- `assistant::Assistant` 是内置助手：一段与模型服务商关于会话主机的对话。每轮把对话发给模型，模型调用的工具经工具箱（`McpTools`；`new` 用会话主机的工具 `SessionTools`，即 `tools::call` 与 `AiSession` 的审批与审计，`with_toolbox` 可换成其他工具与系统提示，如总管的）执行，结果以 `<untrusted-data>` 标记包裹后回填，直到模型不再调用工具；每轮最多 40 次模型调用。回复与思考以 `AiEvent::Update` 流式发出。`cancel` 丢弃进行中的请求并撤回待审批项；下一轮开始时为被取消而没有结果的工具调用补上“已取消”。对话超过约 40 万字符时从最早的工具结果开始省略。切换供应商时保留工具结果，并移除旧供应商专有的协议字段。
- `Assistant::with_models` 提供可切换的模型，`set_model` 从下一次请求起换用，`config_options` 以会话设置给出；`seed` 在第一轮之前放入继续对话的历史。
- `Assistant::with_checkpoint_sink` 在模型请求、回复和工具调用前后保存版本化检查点；sink 成功表示已持久化，失败则停止后续模型或工具调用。检查点含完整在用消息、工具结果、压缩状态与供应商协议字段，调用方须加密保存，不包含 API key。恢复拒绝未知版本和未确认的 `pending_tool`；用户明确核对中断结果后可调用 `resolve_interrupted()` 补入未知结果，恢复过程不重放旧调用。
- `AiSession::restrict_read_only()` 单向收紧任务范围：任何非只读操作立即拒绝并记入 deferred，审批模式、已有授权和审阅者都不能放宽这一范围。
- `backend::AgentBackend` 是应用驱动一个 AI 会话的统一接口（`prompt`、`cancel`、`stop`、`session`、`set_config_option`），由内置助手与 ACP 的 `Running` 实现；`AgentRef` 标识回答的 Agent：`builtin`（服务商 id）或 `acp`（智能体 id）。以后接入其他来源的智能体只需新增实现。
- `detect`：内置智能体表（Codex、Claude Code、Gemini CLI、Qwen Code：ACP 程序名、参数、npm 包、API key 环境变量与登录方式）；`detect(path)` 在 PATH 上查找已安装的程序，否则在有 npx 时经 npx 运行；`search_path()` 合并登录 shell 的 PATH、本进程的 PATH 与常见安装目录。
- `quota`：读取智能体的套餐额度：`codex_quota` 从 Codex 的会话日志读最新的 `rate_limits`，`claude_limits` 解析 Claude Code `/usage` 报告中的限额行；不涉及登录令牌。
- `tools`：Agent 在会话主机上的工具表：`specs(host)` 给出名称、说明与参数 schema，`call(name, arguments, session)` 经 `AiSession` 审批并执行，返回文本与是否失败；内置 MCP server 与内置助手共用。外部 MCP 监听器的 `serve_until` 在关闭时等待已有连接结束并清理其工具会话。
- `mcp`：内置 MCP server（协议 2024-11-05 至 2025-06-18），工具 `run_command`、`read_file`、`write_file`、`edit_file`、`list_dir`、`terminal_snapshot`，全部经 `AiSession` 审批并在会话主机上执行；路径必须是绝对路径。`McpListener` 在回环随机端口上监听，连接须先发送 token 行（10 秒内，常数时间比较）；`bridge_env` 给出桥接进程的环境变量，`config` 给出写入配置文件用的地址与 token（`BridgeConfig`）。连接提供的工具由 `McpTools` 决定（说明、工具列表、调用、客户端在 `initialize` 中报出的名称、连接结束），`serve` 用单个会话的工具（`SessionTools`），`serve_with` 为每个连接用工厂新建一份。`bridge(config)` 是应用二进制 `--mcp-bridge` 模式的实现：从 `--mcp-config` 指定的文件或环境变量取得地址与 token，连上应用后复制 stdin / stdout。
- `AiSession::set_terminal_source` 提供用户终端的最近内容，`read_terminal` 读取它（只读操作，同样记录）。
- `audit::Auditor` 把一个 AI 会话写成 Journal 记录（`session_id` 为 AI 会话 id，payload 带主机）：开始与结束、用户提示、回复与思考（按消息合并）、审批的请求与回答、每个已执行或被拒绝的操作、Agent 自带工具完成时的报告、回合结束、标题（`ai/renamed`：第一条提示自动生成，用户改名另写；`renamed()` 供未运行的会话改名）。`started` 写入 `SessionInfo`（`agent_ref`、主机上下文、Agent 自己的会话 id），`transcript` 从记录还原内置助手的对话。`conversation_body` / `missing_records` 把一段对话打成同步记录（截短、限 64 KiB）并在收到时算出本机缺少的记录。文件与终端读取、写入的最终文本以及命令输出在工具自身的限额内完整交给本机加密审计。`Auditor::wakeup` 记录总管的内部汇总输入。总管启用 `protect_text`，汇总输入随本机私有审计密封，同步投影去除其正文和本机引用。
- `runner::start(spec, session)` 为已建好的 `AiSession` 启动 Agent 并打开会话（内置 MCP server 须在此之前开始服务：Agent 在 `session/new` 返回前就会启动它）：需要登录时用 `AgentSpec.auth_method` 认证后重试，切换到 `preferred_modes` 中第一个可用的模式。`AgentSpec.resume` 给出 Agent 自己的旧会话 id 且 Agent 支持 `loadSession` 时用 `session/load` 重开（失败则开新会话），`Started` 报告会话设置（`configOptions`）、Agent 会话 id 与是否重开；`Running::set_config_option` 修改设置。`Running::cancel` 撤回等待中的审批并通知 Agent；`stop` 结束运行中的命令和 Agent 进程。

- `llm::Client` 是内置助手的模型客户端：对 `Endpoint`（格式、带版本路径的基础地址、模型、可选的 max_tokens 与推理强度）做一次流式补全并提供函数工具，格式为 OpenAI Chat Completions（`/chat/completions`）、OpenAI Responses（`/responses`，`store: false`）、Anthropic Messages（`/messages`，未配置时 `max_tokens` 为 32000，自适应思考加 `output_config.effort`，签名的思考块经 `Reply.opaque` 原样回传）。基础地址必须是 http(s) 且不含凭据和查询；重定向只允许同源；单次回复最多 8 MiB；回复开始前与两段之间超过空闲时限返回 `Timeout`，没有正常结束的流返回 `Protocol`。模型以 400 拒绝推理参数时去掉强度重试一次并记住（`effort_refused`）。`list_models` 读取服务商的 `/models`。`Reply.stats` 带输入输出 token、首字时间与生成时长。API key 只放在请求头，错误信息中出现的 key 被替换为 `***`。

## 局限

- 分级是启发式：混淆写法可能低估风险，因此只有明确识别为只读的命令才会自动放行。
- `ssh host cmd` 中的远程命令按同一规则评估，但不知道它在哪台主机上执行。
- 远程命令超时时只是放弃等待，服务器上的进程不一定随之结束。
- 本机在 Windows 上用 `cmd` 运行命令，而分级按 POSIX shell 语法进行。
- `terminal/*` 的命令结束后才有输出，期间 `terminal/output` 为空。
