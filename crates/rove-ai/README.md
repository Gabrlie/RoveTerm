# rove-ai

RoveTerm 中的 AI Agent：ACP 客户端、审批策略和绑定主机的工具。Agent 是不可信的外部进程，它在主机上的每个操作都先经过 `Policy`。设计理由见 [0006](../../docs/decisions/0006-ai-acp.md)，参考见 [docs/subsystems/ai.md](../../docs/subsystems/ai.md)。

## 契约

- `shell::parse(line)` 把命令行拆成流水线、简单命令、去掉引号的词、重定向和嵌套命令文本（`$(…)`、反引号、子 shell）。不执行也不展开任何内容；无法静态确定的词标为 `dynamic`。
- `risk::assess_command(line, rules)` 给出 `Assessment { risk, programs, opaque, reasons }`：逐段评估，取最高的 `Risk`（`ReadOnly` < `Modify` < `Dangerous`）。不认识的程序一律为 `Modify`；展开、脚本和解释器代码使结果 `opaque`。
- `assess_read(path)`：读取秘密文件（私钥、`/etc/shadow`、云凭据、`.env` 等）为 `Modify`，其他为 `ReadOnly`。`assess_write(path)`：写系统路径和设备为 `Dangerous`，其他为 `Modify`。
- `Policy::decide(operation, assessment)` 按 `Mode` 返回 `Allow` 或 `Ask { rememberable }`；`Dangerous` 总是询问且不可记住。`Policy::answer` 记录用户的回答，`AllowSession` 只在可记住时生效：命令按程序名、写入按目录、读取按文件、Agent 自带工具按种类。
- `Rules` 是用户追加的只读程序名和危险文本，由 `validate()` 校验。

## 局限

- 分级是启发式：混淆写法可能低估风险，因此只有明确识别为只读的命令才会自动放行。
- `ssh host cmd` 中的远程命令按同一规则评估，但不知道它在哪台主机上执行。
