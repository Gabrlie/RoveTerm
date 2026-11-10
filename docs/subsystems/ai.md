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
