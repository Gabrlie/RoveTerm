# ai

`rove-ai` 让外部 AI Agent（首先是经 `codex-acp` 适配器的 Codex）在当前主机上协作。Agent 不可信：它的每个操作都由本地 `Policy` 决定放行还是请用户审批，并写入 Journal。设计理由见 [决策 0006](../decisions/0006-ai-acp.md)。

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
| 按风险询问（默认） | 放行 | 询问 | 每次询问 |
| 全部询问 | 询问 | 询问 | 每次询问 |
| 自动 | 放行 | 放行 | 每次询问 |

询问时用户可以允许一次、本会话允许或拒绝。本会话允许只对非危险、非不透明的操作提供，按以下粒度记住：

- 命令：按其中的全部程序名，必须都已允许才自动放行。
- 写文件：按目录。
- 读秘密文件：按文件。
- Agent 自带工具：按工具种类。

Agent 在权限请求中附带的选项（如"始终允许"）不参与决策。

## ACP 会话

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
| `session/request_permission` | Agent 自带工具（在本机运行）：按 `rawInput.command` 与涉及的文件评估；放行或拒绝时只选择 Agent 的一次性选项，"始终允许"由本地策略管理 |

Agent 的环境变量只从白名单（`acp::ENV_ALLOWED`：`PATH`、`HOME`、`LANG`、`LC_*`、`TMPDIR`、代理与证书变量、Windows 的系统目录变量等）继承，再加上调用方给出的变量（如从 vault 解密的 API key）。消息单条最大 8 MiB，转发给界面的文本按 256 KiB 截断。

取消回合时撤回所有等待中的审批（视为拒绝）并发送 `session/cancel`；结束会话时中止运行中的命令并结束 Agent 进程。

## 内置 MCP 工具

Agent 自带的 shell 在本机运行，到不了远程主机，因此 RoveTerm 在 `session/new` 中附带自己的 MCP server，工具作用于会话主机并经过同一审批：

| 工具 | 作用 |
|---|---|
| `run_command` | 运行 shell 命令，返回退出码与输出；可指定工作目录和超时（最多 600 秒） |
| `read_file` / `write_file` | 读写文本文件，可按行截取 |
| `list_dir` | 列目录 |
| `terminal_snapshot` | 读取用户在该主机上的终端最近显示的内容 |

Agent 以子进程方式启动 MCP server，所以应用二进制带一个桥接模式：`roveterm --mcp-bridge` 从环境变量 `ROVETERM_MCP_ADDR`、`ROVETERM_MCP_TOKEN` 得知运行中应用的回环端口和 token，连上后先发送 token，再把 stdin / stdout 原样转发。应用为每个 AI 会话监听一个随机回环端口，token 随机生成、只在内存中，经环境变量（而非命令行参数）交给桥接进程；token 不对的连接直接关闭。

## 应用接入

`src-tauri/src/commands/ai.rs` 管理运行中的 AI 会话（`AppState.ai`），事件以 `ai-update`（`{ id, event }`）发给界面，同时由 `Auditor` 写入 Journal：

| 命令 | 作用 |
|---|---|
| `ai_status()` | 本机会启动的 Agent（来源：配置、PATH 中的 `codex-acp`、或 `npx -y @agentclientprotocol/codex-acp`）与是否已存 API key |
| `ai_set_key(key)` | 密封保存或删除 API key |
| `ai_start(context, terminal)` | 在 `local`、已保存主机或快速连接上启动 Agent；`terminal` 为用户在该主机的终端会话，供 `terminal_snapshot` 读取 |
| `ai_prompt(id, text)` | 发出提示并等待回合结束，返回结束原因 |
| `ai_cancel` / `ai_answer` / `ai_set_mode` / `ai_stop` | 取消回合、回答审批、切换审批模式、结束会话 |

启动时：

- 查找 Agent 用登录 shell 的 PATH（`$SHELL -ilc`，并补上 Homebrew、`~/.local/bin` 等常见目录），因为 macOS 与 Linux 的图形应用启动时 PATH 很短。
- 登录方式按 `DevicePrefs.agent.auth`：用 API key 时以 `CODEX_API_KEY`、`OPENAI_API_KEY` 传入并优先选择 Agent 的 API key 登录方式，否则用 Agent 自己的账户登录（Codex 为 ChatGPT）。
- 远程主机的会话以数据目录下的空目录 `ai-workspace` 为 Agent 的工作目录，本机会话以家目录为工作目录。
- 内置 MCP server 以应用自身的可执行文件加 `--mcp-bridge` 交给 Agent，并在启动 Agent 之前就开始接受桥接连接：Agent 在 `session/new` 返回前就会启动 MCP server 并等待它应答。

应用退出时结束所有 AI 会话。
