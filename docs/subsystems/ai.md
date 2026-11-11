# ai

`rove-ai` 让 AI Agent 在当前主机上协作：内置助手（用户自己的模型服务商与 key），或本机安装的 ACP 智能体（Codex、Claude Code、Gemini CLI、Qwen Code 等）。Agent 不可信：它的每个操作都由本地 `Policy` 决定放行还是请用户审批，并写入 Journal。设计理由见 [决策 0006](../decisions/0006-ai-acp.md)。

定位：内置助手是主力（运维助手：命令、读写与补丁编辑、审阅、上下文压缩、提示缓存）。ACP 智能体是兼容通道，只保证通用能力——对话、审批、模型与设置菜单（`/` 菜单）、命令——不再针对单个智能体定制界面或适配其专有设置。想用 Claude Code、Codex 等订阅的用户，推荐在它们自己的终端里运行，并经"开放给本机智能体"使用 RoveTerm 的主机（审批与记录同样生效）。

## 风险分级

`risk::assess_command` 先用 `shell::parse` 恢复命令行的结构，再逐段判断，取最高级：

| 级别 | 含义 | 例子 |
|---|---|---|
| `ReadOnly` | 只读 | `ls`、`cat … \| grep`、`git status`、`systemctl status`、`docker ps`、`kubectl get` |
| `Modify` | 修改文件、进程或设置；也包括所有未识别的程序 | `systemctl restart`、`git commit`、`sed -i`、`echo > file`、`apt install`、`bash script.sh` |
| `Dangerous` | 可能毁掉数据或让主机停机 | 递归删除根目录、家目录、系统目录或通配；`mkfs`、`dd of=`；关机重启；对系统目录递归改权限；写系统路径或磁盘设备；fork bomb；下载后管道给 shell；清空防火墙；`git push --force`、`git reset --hard`；删除用户、`crontab -r`；`docker system prune`、`kubectl delete`；`DROP DATABASE` |

解析规则：

- 引号与转义会先去掉，`r''m`、`\rm`、`"rm"`、`/bin/rm` 都按 `rm` 判断。
- `$(…)`、反引号、子 shell 中的命令单独评估；`bash -c`、`su -c`、`eval`、`watch`、`ssh host cmd`、`find -exec` 中的命令也会被评估。
- `sudo`、`env`、`nohup`、`timeout`、`xargs` 等包装器会被跳过，评估真正运行的程序；`sudo` 和 `doas` 至少为 `Modify`。
- 程序名来自变量或展开（`$CMD`）、运行脚本或解释器代码时，结果标为 `opaque`：至少为 `Modify`，且不能"本会话允许"。
- 重定向：写普通文件为 `Modify`，写系统路径或磁盘设备为 `Dangerous`，`/dev/null` 等不计。here-document 的内容不当作命令。

文件操作：读取为 `ReadOnly`，读取秘密文件（SSH 私钥、`/etc/shadow`、云凭据、`.env`、`*.pem`、`*.key`）为 `Modify`；写入为 `Modify`，写系统路径为 `Dangerous`。Agent 自带的工具按 ACP 的工具种类判断：读取、搜索、思考、抓取为只读，编辑、移动、删除为修改，执行但未给出命令的为不透明的修改。

用户可以在 `Rules` 中追加只读程序名和"危险文本"（命令中出现即为 `Dangerous`，如生产数据库主机名）。

## 主机

AI 会话绑定一台主机（`host::Host`），会话中的命令与文件操作都在这台主机上执行：

| 主机 | 命令 | 文件 |
|---|---|---|
| `LocalHost` | `sh -c`（Windows 上 `cmd /C`），超时后结束进程 | 本机文件系统 |
| `RemoteHost` | 应用已有连接上的 exec 通道，`cwd` 以 `cd '…' &&` 前缀进入 | 同一连接上的 SFTP，连接变化时重新打开 |

远程主机只使用应用已打开的连接（终端或文件面板），没有连接时返回 `NotConnected`，不会自行连接或询问凭据。默认上限（`Limits`）：每个输出流 64 KiB、命令 120 秒、文件 1 MiB、目录 1000 项；超出的部分被截断并标出，不是文本的文件不读取。

## 审批

每个 AI 会话有一个 `Policy`，会话绑定一台主机：

| 模式 | 只读 | 修改 | 危险 |
|---|---|---|---|
| 询问（`ask_all`） | 询问 | 询问 | 每次询问 |
| 审阅（`ask_by_risk`，默认） | 放行 | 询问 | 每次询问 |
| 自动（`auto`） | 放行 | 交给判断者，没把握时询问 | 每次询问 |

审阅模式下每个修改都由用户决定；自动模式下由 AI 判断者先判断（见下文"审阅者"），没有判断者时（没有服务商或 key）同样询问用户。

询问时用户可以允许一次、本会话允许或拒绝。本会话允许只对非危险、非不透明的操作提供，按以下粒度记住：

- 命令：按其中的全部程序名，必须都已允许才自动放行。
- 写文件：按目录。
- 读秘密文件：按文件。
- Agent 自带工具：按工具种类。

Agent 在权限请求中附带的选项（如"始终允许"）不参与决策。

## ACP 会话

兼容通道：以下只描述通用行为，各智能体的专有设置原样列在 `/` 菜单中。

`runner::start` 以子进程启动 Agent，经 stdio 上按行分隔的 JSON-RPC（ACP 版本 1）通信：

1. `initialize`：声明客户端能力 `fs.readTextFile`、`fs.writeTextFile`、`terminal`，读取 Agent 名称与登录方式。
2. `session/new`：工作目录为会话主机对应的目录，附带内置 MCP server。Agent 要求登录时，用配置的方式 `authenticate` 后重试。
3. 若 Agent 提供配置中偏好的模式（Codex 为 `read-only`），用 `session/set_mode` 切换过去，使 Agent 自带的工具在改动任何东西前都发出权限请求。
4. `session/prompt` 发出用户的话，等待回合结束；期间 `session/update` 转为 `AiEvent::Update`（回复与思考片段、工具调用、计划、模式变化），其余类型忽略。

Agent 发来的请求由 `AiSession` 回答，全部经过审批：

| 请求 | 处理 |
|---|---|
| `fs/read_text_file`、`fs/write_text_file` | 在会话主机上读写；路径必须是绝对路径，可按行号与行数截取 |
| `terminal/create` 等 | 把程序、参数和环境变量拼成一行 shell 命令后评估，在会话主机上运行；`output`、`wait_for_exit`、`kill`、`release` 操作这次运行 |
| `session/request_permission` | Agent 自带工具（在本机运行）：请求只带工具调用 id 时，用之前 `tool_call` 通知报告过的字段补全（标题、`rawInput`、涉及的文件、说明文字）；按 `rawInput.command` 评估（`bash -lc "<脚本>"` 按脚本本身评估与显示）与涉及的文件评估，审批卡片显示标题、命令或第一个文件、Agent 给出的理由或参数（`AgentTool.path`、`detail`）。Codex 在调用 RoveTerm 自己的 MCP 工具前发来的请求（`rawInput.server_name` 为 `roveterm`）直接放行，工具内部仍走策略，避免同一操作问两次。放行或拒绝时只选择 Agent 的一次性选项，"始终允许"由本地策略管理 |

Agent 的环境变量只从白名单（`acp::ENV_ALLOWED`：`PATH`、`HOME`、`LANG`、`LC_*`、`TMPDIR`、代理与证书变量、Windows 的系统目录变量等）继承，再加上调用方给出的变量（如从 vault 解密的 API key）。消息单条最大 8 MiB，转发给界面的文本按 256 KiB 截断。

Agent 的错误响应若在 `data.message` 中给出原因（Codex 的回合失败），显示这段原因而不是笼统的 `Internal error`。

取消回合时撤回所有等待中的审批（视为拒绝）并发送 `session/cancel`；结束会话时中止运行中的命令并结束 Agent 进程。

## 工具与内置 MCP server

工具的名称、说明、参数和实现集中在 `tools.rs`，内置 MCP server 与内置助手提供的是同一套工具。

Agent 自带的 shell 在本机运行，到不了远程主机，因此 RoveTerm 在 `session/new` 中附带自己的 MCP server，工具作用于会话主机并经过同一审批：

| 工具 | 作用 |
|---|---|
| `run_command` | 运行 shell 命令，返回退出码与输出；可指定工作目录和超时（最多 600 秒） |
| `read_file` / `write_file` | 读写文本文件，可按行截取 |
| `read_notes` / `update_notes` | 读取或整段替换 RoveTerm 对该主机的备注（仅已保存的主机提供，`NotesSource` 由应用以 vault 实现）：读取按只读记录（`ai/notes_read`），替换按修改评估并显示差异（`ai/notes_write`）；内置助手的系统提示词附带当前备注（`<host-notes>`，注明可能过时） |
| `edit_file` | 把文件中的一段文字（`old`，须原样且只出现一次，`replace_all` 时可多处）替换为 `new`（`edit.rs`）；替换不成立时不发起审批直接报错，审批卡片显示删除与新增的行（`Operation::EditFile`，按写入评估风险、按目录记住授权），结果记为 `ai/file_write`，审计中可看到差异；系统提示词引导改已有文件时优先用它 |
| `list_dir` | 列目录 |
| `terminal_snapshot` | 读取用户在该主机上的终端最近显示的内容 |

Agent 以子进程方式启动 MCP server，所以应用二进制带一个桥接模式：`roveterm --mcp-bridge` 从环境变量 `ROVETERM_MCP_ADDR`、`ROVETERM_MCP_TOKEN` 得知运行中应用的回环端口和 token，连上后先发送 token，再把 stdin / stdout 原样转发。应用为每个 AI 会话监听一个随机回环端口，token 随机生成、只在内存中，经环境变量（而非命令行参数）交给桥接进程；token 不对的连接直接关闭。另有 `--mcp-bridge --mcp-config <文件>`：从文件读取地址与 token，供用户自己运行的 Agent 使用（见"开放给本机智能体"）。每个连接的工具由 `McpTools` 提供，连接结束时得到通知。

## 应用接入

`src-tauri/src/commands/ai.rs` 管理运行中的 AI 会话（`AppState.ai`），事件以 `ai-update`（`{ id, event }`）发给界面，同时由 `Auditor` 写入 Journal：

| 命令 | 作用 |
|---|---|
| `ai_agents()` | 本机可用的 Agent 与默认项：每个服务商对应一个内置助手（`builtin:<id>`，附模型），然后是检测到的 ACP 智能体与按命令添加的智能体（`acp:<id>`，附运行的程序、是否经 npx）；各项是否已存 key、是否启用（`enabled`，用户关闭的检测项仍列出，供设置页重新打开，`ai_start` 只启动启用的）、是否按命令添加（`custom`） |
| `ai_set_key(agent, key)` | 密封保存或删除某个 Agent 的 API key |
| `ai_delete(id)` | 删除一段对话：运行中先停止，等 Journal 写完排队的记录后移除该会话的 `ai/*` 记录，并追加一条不属于任何会话的 `ai/conversation_deleted`（`payload.session`），同步据此删除其他设备上的同一对话（见 [sync](sync.md#应用中的同步)） |
| `ai_provider_models(provider, key)` | 列出服务商提供的模型及其上下文窗口（列表给出时：`context_length`、`context_window`、`max_model_len`、`max_input_tokens` 等，超过 1 亿的不采信；`GET /models`；OpenAI 兼容用 Bearer，Anthropic 用 x-api-key 并按 `after_id` 翻页，最多 20 页），排序去重后返回 id；key 未给出时用已保存的，vault 未解锁时不带 key |
| `ai_provider_test(provider, key)` | 用一次最小请求检查服务商的地址、模型和 key（未给出时用已保存的），返回模型的回复 |
| `ai_start(context, agent, terminal, resume)` | 在 `local`、已保存主机或快速连接上启动 `agent`；内置助手直接构造 `Assistant`，ACP 智能体按检测结果启动并附带内置 MCP server；`terminal` 为用户在该主机的终端会话，供 `terminal_snapshot` 读取。返回会话设置（`config_options`，见下文）。`resume` 为之前的会话 id 时继续那段对话：须是同一 Agent、当前未打开，沿用同一 id |
| `ai_set_option(id, option, value)` | 修改会话设置（如 `model`），返回修改后的全部设置 |
| `ai_quota(agent, refresh)` | 本机智能体的套餐额度（各限额窗口的已用百分比、窗口长度、重置时间）：Codex 读 `$CODEX_HOME`（默认 `~/.codex`）下 `sessions/` 中最新会话日志末尾的 `token_count` 事件的 `rate_limits`，并从最新日志的最后一个 `token_count` 读出上下文占用（`info.last_token_usage` 对 `info.model_context_window`，`context`），每次都重读不复用；Claude Code 单独启动一个不带工具的短会话，发送其本地命令 `/usage`（不经过模型），按报告中的 `**名称** — **N%**` 行解析，启动与应答各限时 30 秒。Claude Code 的结果复用 5 分钟，`refresh` 时重读；其他智能体返回空。RoveTerm 不读取任何登录令牌（`rove_ai::quota`，有单元测试） |
| `ai_rename(id, title)` | 给会话起名（1–120 个字符，运行中或已结束均可），写入 `ai/renamed` 记录 |
| `ai_prompt(id, text)` | 发出提示并等待回合结束，返回结束原因 |
| `ai_cancel` / `ai_answer` / `ai_set_mode` / `ai_stop` | 取消回合、回答审批、切换审批模式、结束会话 |
| `ai_sessions()` / `ai_records(id)` | 从 Journal 读出最近 100 个 AI 会话（含从其他设备同步来的，见 [sync](sync.md#应用中的同步)；列表按开始时间、记录按发生时间排序；同一 id 继续过的只列一次，按最近一次开始；附标题、`agent_ref` 与主机上下文），以及某个会话的全部 `ai/*` 记录（最多 5000 条），供审计回看与继续对话 |

启动时：

- 查找 ACP 智能体用登录 shell 的 PATH（`$SHELL -ilc`，并补上 Homebrew、`~/.local/bin` 等常见目录），因为 macOS 与 Linux 的图形应用启动时 PATH 很短。
- ACP 智能体的登录方式按 `DevicePrefs.agents.auth`：使用该智能体保存的 key 时，按检测表放进它的 key 环境变量并优先选择 key 登录方式，否则用它自己的账户登录。内置助手用对应服务商保存的 key（没有时不带 key，适用于本地模型）。
- 远程主机的会话以数据目录下的空目录 `ai-workspace` 为 Agent 的工作目录，本机会话以家目录为工作目录。
- 内置 MCP server 以应用自身的可执行文件加 `--mcp-bridge` 交给 Agent，并在启动 Agent 之前就开始接受桥接连接：Agent 在 `session/new` 返回前就会启动 MCP server 并等待它应答。

应用退出时结束所有 AI 会话。

ACP 的 `available_commands_update` 解析为 `SessionUpdate::Commands`（每次是完整列表；名称去掉开头的 `/`，空名或含空白的丢弃，最多 200 条，说明与参数提示各截到 500 个字符）。ACP 的 `usage_update`（`used` / `size`，对话占用的上下文 token 数与窗口大小）解析为 `SessionUpdate::Usage` 交给界面。内置助手每次请求后发出 `SessionUpdate::Stats`：输入（含缓存）与输出 token、首字时间（发出请求到第一段文字或推理）、tokens/s（输出 token 除以生成时长）；token 数取自服务商的 usage（Chat 请求带 `stream_options.include_usage`，Responses 读 `response.completed`，Anthropic 读 `message_start` 与 `message_delta`），未报告的项为空。统计不写入 Journal。知道模型的上下文窗口时（服务商的 `contexts`，否则 `context_window`），内置助手每次请求后还发出 `SessionUpdate::Usage`（输入加输出 token 与窗口大小）。

会话设置与继续对话：

- 会话设置采用 ACP 的 session config options（`configOptions`、`session/set_config_option`、`config_option_update` 通知），保留单选项（分组的选项展平）与开关项（`type: "boolean"`，如 Claude Code 的快速模式，设置时传布尔值）；类别为 `mode` 的项不交给界面，审批由 RoveTerm 的模式管理。只在较旧的 `models` 字段（`availableModels`、`currentModelId`）报告模型的智能体，合成一个 `model` 设置，改用 `session/set_model`。智能体对修改不回报设置列表时，按已接受的值更新。Codex 以此提供模型与推理强度。内置助手提供 `model`（服务商的默认模型加 `models`）与 `effort`（minimal、low、medium、high、xhigh、max，未设置时为 medium）两个设置，对话中修改从下一次请求起生效，历史保留。
- 只启动 Agent、没有发过消息的会话不留记录：`Auditor::started` 暂存开始信息，随第一条记录（通常是第一条提示）写入，什么都没记录过的会话结束时也不写结束记录。
- `ai/session_started` 记录 `SessionInfo`：Agent 名称与版本、审批模式、`agent_ref`、主机上下文、Agent 自己的会话 id、是否为继续。会话的第一条提示写一条自动的 `ai/renamed`（第一行，最多 60 个字符），用户改名再写一条，最新的为准；继续时已有标题则不再自动命名。
- 继续 ACP 会话：Agent 声明 `loadSession` 能力时用 `session/load` 重开它自己的会话，期间 Agent 回放的旧对话不再上报（界面从 Journal 还原）；重开失败则开新会话。继续内置助手：用 `audit::transcript` 从记录还原提示与回复（执行过的操作以一行说明附在回复中，不含工具输出）作为历史。

## 运维总管

总管是所有已保存主机上的内置助手（`AiContext::Fleet`，`src-tauri/src/commands/fleet.rs`），自己不在任何主机上执行操作，只有三个工具：

- `list_hosts`、`read_notes(host)`（与开放给本机智能体的同名工具同源）；
- `delegate(tasks: [{host, task}])`：每项在该主机上启动一个子代理（内置助手 + 主机工具与备注，沿用用户的审批模式、规则与判断者），最多 4 个同时运行、一次最多 16 项，各自跑完一个回合后返回最后的回复（每项截到 4 KiB）。

总管的系统提示词要求它为每个子代理写自足的任务，并把结果当作数据。总管的工具调用以工具行报告在它自己的会话中（显示并记入 Journal）。子代理是独立的会话（`run_subagent`），`SessionInfo.parent` 为总管的会话 id，不出现在历史列表中；开始与结束经 `ai-subtask` 事件通知界面。总管回合被取消时，正在运行的子代理随之结束。总管只能用内置助手（默认服务商为内置时用它，否则用第一个服务商），`Assistant::with_toolbox` 让它复用同一对话循环。

## 开放给本机智能体

用户在终端中自己运行的 Agent（Claude Code、Codex 等）可以经 RoveTerm 使用已保存的主机（`src-tauri/src/commands/external.rs`）。按设备开关（`DevicePrefs.agents.expose`，默认关闭）：

- 开启后应用在回环随机端口提供 MCP server，地址与随机 token 写入 `<数据目录>/mcp.json`（Unix 上权限 0600）；关闭或退出时删除该文件、停止监听，下次启动时按开关重新写入。Agent 运行 `roveterm --mcp-bridge --mcp-config <该文件>`。
- 设置页的"一键接入"用 Agent 自己的命令添加（先移除旧项）：`claude mcp add --scope user roveterm -- <命令>`、`codex mcp add roveterm -- <命令>`，程序按登录 shell 的 PATH 查找，60 秒超时。其他 Agent 可手动添加显示的命令（stdio）。
- 工具：`list_hosts`（id、名称、分组、标签、`用户@主机:端口`、系统，不含凭据），以及 `run_command`、`read_file`、`write_file`、`edit_file`、`list_dir`，各多一个必填的 `host`（名称或 id，名称不分大小写，重名时要求 id）。
- 每个连接对它用到的每台主机各开一个会话（`ai::start_external`）：沿用该用户的审批模式、规则与审阅者，按 Host 上下文记入 Journal（`agent` 为客户端在 `initialize` 报出的名称，无 `agent_ref`，不能在 RoveTerm 中继续）；后端为 `ExternalBackend`（不接受提示，停止时撤回审批与命令）。开始与结束经 `ai-external` 事件通知界面，连接断开时结束其全部会话。

## 模型客户端

内置助手不经外部程序，直接调用模型服务商（`llm::Client`）。服务商以 `Endpoint` 描述：格式、包含版本路径的基础地址（如 `https://api.openai.com/v1`、`https://api.anthropic.com/v1`、`http://localhost:11434/v1`）、模型、可选的 `max_tokens` 与推理强度。

| 格式 | 路径 | 认证 | 流式事件 |
|---|---|---|---|
| `chat` | `/chat/completions` | `Authorization: Bearer` | `choices[0].delta` 的 `content`、`reasoning_content`、`tool_calls`（按 `index` 拼接参数），`[DONE]` 或 `finish_reason` 结束 |
| `responses` | `/responses` | `Authorization: Bearer` | `response.output_text.delta`、`response.reasoning_summary_text.delta`、`response.output_item.done` 中的 `function_call`，`response.completed` 结束 |
| `anthropic` | `/messages` | `x-api-key` 与 `anthropic-version` | 按内容块索引累积文本、思考（含签名）和 `tool_use` 的 `input_json_delta`，`message_stop` 结束 |

对话以统一的 `Message`（用户、助手含工具调用、工具结果）传给客户端，由它转换成各格式的请求；Anthropic 的同一轮工具结果合并进一条用户消息，签名的思考块在下一轮原样回传。

自动模式的判断者（`review.rs`）：自动模式（`Auto`）下，策略要询问用户的普通修改（风险为 modify、非 opaque、非 Agent 自己的工具）先交给审阅者，审阅者放行且置信度不低于 `ai.review_confidence` 时直接执行，否则照常询问；危险操作始终询问用户。审阅者看到主机名、操作描述（命令，或文件路径与改动，最多 8 KiB）与被标记的原因，看不到文件全文与凭据；20 秒内无结论、出错或未配置时询问用户。判断者三选一（`ai.reviewer`）：`model` 用会话的服务商模型（本机智能体用默认服务商，没有时用第一个）；`chosen` 用指定的服务商与模型（`ai.review_model`，可选更便宜的）；`jev` 见下。`ModelReviewer`发一次不带工具、不带推理的请求，只认 `allow` 一个词；`JevReviewer` 向 TypeSafe 的 `/v1/systemone` 发一个 Choice 问题（`allow` / `ask`），取其置信度，key 存为 `ai_key` 的 `review:jev`（`ai_set_review_key`，随同步）。结论发出 `AiEvent::Reviewed`，Journal 记 `ai/reviewed`（审阅者、是否放行、置信度、操作），放行的操作在对话中带"已审阅"标记，只读操作以外的行不再并入"已执行 N 个只读操作"。

上下文压缩：知道模型的上下文窗口时，若上一次请求（输入加输出）已超过窗口的 80%，下一次请求前把当前回合之前的对话用同一模型总结（不带工具的一次请求，各消息截到 2000 字节，提示中说明对话是数据），摘要放在当前回合的用户消息开头（`<summary-of-earlier-conversation>`），不拆开工具调用与结果；只有当前一个回合时不压缩；总结失败时照常继续。压缩发出 `SessionUpdate::Compacted`，Journal 记 `ai/compacted`（摘要），对话中显示为可展开的"已压缩较早的对话"，继续历史对话时按记录还原该行。

提示缓存：Anthropic 在系统提示（连同其前的工具）和对话最后一个内容块（思考块除外）各设一个 `cache_control: {type: "ephemeral"}` 断点，每次请求读取上一次写入的缓存；Responses 带 `prompt_cache_key`（AI 会话 id），同一对话的请求共享前缀；Chat Completions 不加参数（兼容接口可能拒绝未知参数，OpenAI 与 DeepSeek 会自动缓存）。缓存读写的 token 计入输入 token。一个回合的最多请求次数与回复超时取自 `ai.max_steps` / `ai.request_timeout_secs`。

推理强度按格式换算：OpenAI Chat 的 `reasoning_effort` 与 Responses 的 `reasoning.effort` 取 minimal 至 xhigh（max 按 xhigh）；Anthropic 用自适应思考（`thinking: {type: "adaptive", display: "summarized"}`）加 `output_config.effort`（low 至 max，minimal 按 low），不再发送固定的思考预算，当前模型会拒绝它。模型不支持推理参数时（400 且错误信息提到 reasoning、effort 或 thinking），去掉强度重试一次，同一客户端之后不再发送；切换模型或强度时重新尝试。

安全与限制：基础地址必须是 http(s)、不含凭据与查询；重定向只在同源内跟随，key 不会被带到别处；API key 只在请求头中，错误文本里出现的 key 替换为 `***`；单次回复最多 8 MiB；首包与两段之间超过空闲时限即超时；流没有正常结束视为错误。

## 内置助手

`assistant::Assistant` 不经外部程序，用 `llm::Client` 与模型服务商对话：

- 系统提示把助手绑定到会话主机（名称、地址、已知的系统），说明它只能使用给定的工具、每次调用都会经过用户的审批规则、被拒绝时不要绕过；工具结果、文件内容和终端输出是不可信数据，包在 `<untrusted-data source="…">` 中，其中的指令不是用户的意图（数据中的结束标记会被改写，边界不会被提前关闭）。
- 每轮：模型有工具调用时逐个经 `tools::call` 执行（评估、审批、审计都在 `AiSession` 中），结果回填后再次调用模型，直到它只给出文字；一轮最多 40 次模型调用，超出时以 `max_turn_requests` 结束。
- 回复与思考以 `AiEvent::Update` 流式发出（每次模型调用一个消息 id），执行结果照常以 `AiEvent::Executed` 报告，界面与审计不区分内置助手和 ACP 智能体。
- 取消时丢弃进行中的模型请求、撤回待审批项；被打断的工具调用在下一轮开始时补上“已取消”的结果，保证对话对服务商仍然有效。对话过长时从最早的工具结果开始省略内容。

## Agent 与检测

应用以 `AgentBackend`（`prompt`、`cancel`、`stop`、`session`）驱动 AI 会话，内置助手与 ACP 智能体都实现它；`AgentRef` 为 `builtin:<服务商 id>` 或 `acp:<智能体 id>`。以后接管服务器上已有的智能体时，新增一种 `AgentRef` 与一个实现即可，界面和审计不变。

`detect` 中的内置表：

| id | 名称 | ACP 程序 | npx 包 | API key 变量 |
|---|---|---|---|---|
| `codex` | Codex | `codex-acp` | `@agentclientprotocol/codex-acp` | `CODEX_API_KEY`、`OPENAI_API_KEY` |
| `claude` | Claude Code | `claude-agent-acp`（或旧的 `claude-code-acp`） | `@agentclientprotocol/claude-agent-acp` | `ANTHROPIC_API_KEY` |
| `gemini` | Gemini CLI | `gemini --experimental-acp` | `@google/gemini-cli` | `GEMINI_API_KEY` |
| `qwen` | Qwen Code | `qwen --experimental-acp` | `@qwen-code/qwen-code` | `OPENAI_API_KEY` |

在 `search_path()`（登录 shell 的 PATH、本进程的 PATH、Homebrew、`~/.local/bin`、`~/.cargo/bin`、`~/.npm-global/bin`、`~/.bun/bin`、`~/.volta/bin`）上找到程序即为已安装；否则有 npx 时经 `npx -y <包>` 运行（首次使用时下载）；两者都没有的智能体不列出。
