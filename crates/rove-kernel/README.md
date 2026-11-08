# rove-kernel

RoveTerm 的插件内核：类型化服务注册表、事件总线、`Disposer`、`Plugin` 和 Profile 运行时。语义参考见 [docs/subsystems/kernel.md](../../docs/subsystems/kernel.md)。

## 契约

- `Context::provide::<S>()` 为类型 `S`（通常是 `dyn Trait`）注册唯一的 Provider；重复注册返回 `ServiceConflict`。返回的 `Disposer` 只移除自己注册的那一项。
- 事件处理器在分发前做快照：处理器内部可以注册或撤销其他处理器，变化从下一次分发开始生效。
- broadcast 处理器在发出事件的线程上同步执行，不能阻塞。
- `Runtime::boot` 按 profile 顺序应用插件；任何一步失败都会先卸载已应用的插件再返回错误。
- `Runtime` 被 drop 时按应用的逆序卸载插件，每个插件内部按注册的逆序撤销。

## 配置

内核本身没有配置项。Profile 的 TOML 格式见 [subsystems/kernel.md](../../docs/subsystems/kernel.md#profile)。

## 局限

- 插件在编译期静态链接，不支持运行时加载（[决策 0004](../../docs/decisions/0004-static-plugin-kernel.md)）。
- `Plugin::apply` 是同步的；需要异步初始化的插件在服务内部惰性完成初始化。
