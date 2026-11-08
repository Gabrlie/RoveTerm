# kernel

`rove-kernel` 提供 `Context`（类型化服务注册表与事件总线）、`Disposer`、`Plugin` 和 Profile 加载。它不包含任何业务逻辑。

## Context

| 方法 | 语义 |
|---|---|
| `provide::<S>(Arc<S>) -> Result<Disposer>` | 注册 `S` 的唯一 Provider；已有 Provider 时返回 `ServiceConflict` |
| `get::<S>() -> Option<Arc<S>>` | 查找 Provider |
| `require::<S>() -> Result<Arc<S>>` | 查找 Provider；缺失时返回 `MissingService` |
| `events() -> &EventBus` | 事件总线 |

服务按类型索引。Definition 一般是 trait，Consumer 用 `ctx.require::<dyn Trait>()` 取得 Provider，因此只依赖 Definition。

## Disposer

每一项注册都返回 `Disposer`，drop 时撤销且只撤销一次；`Context` 已被 drop 时什么也不做。`Disposers` 是有序集合，drop 时按逆序撤销。

## EventBus

| 模式 | 注册 | 分发 | 语义 |
|---|---|---|---|
| broadcast | `on::<E>(Fn(&E))` | `emit(&E)` | 同步、按注册顺序执行；用于 Live 事件 |
| serial | `on_serial::<E>(async Fn(Arc<E>))` | `emit_serial(E).await` | 依次 await，前一个完成后才执行下一个 |
| waterfall | `intercept::<W>(priority, async Fn(Input, Next<W>))` | `run::<W>(input, capability).await` | 按优先级从高到低串成链；拦截器调用 `next.run(input)` 才会交给下一层，否则短路 |

事件类型实现 `Event`；waterfall 用一个标记类型实现 `Waterfall`，声明 `Input` 和 `Output`。同优先级的拦截器按注册顺序执行。

## Plugin

```rust
pub trait Plugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn requires(&self) -> &'static [&'static str] { &[] }
    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError>;
}
```

`apply` 校验配置、注册服务，并返回全部 `Disposer`。`requires` 中列出的插件必须在 profile 中排在它前面。

## Profile

```toml
[profile]
name = "desktop"
plugins = ["vault", "journal", "session"]   # 应用顺序

[plugin.session]                           # 每个插件一个配置段
coalesce_ms = 6
```

- `Profile::from_layers` 依次合并多层 TOML：表按键逐层深度合并，其他值（包括数组）整体替换。
- 顶层未知键、重复插件名会在解析时报错。
- `PluginConfig::parse::<T>()` 把配置段反序列化为 `T`；`T` 应标注 `#[serde(deny_unknown_fields)]`，拼错的键会在加载时报错。

## Runtime

`Runtime::boot(profile, catalog)` 依次应用插件，以下情况会报错：插件不在 catalog 中、依赖的插件未先应用、配置段对应的插件未启用、`apply` 失败。报错前会先卸载已经应用的插件。`Runtime` 被 drop 时按逆序卸载全部插件。`boot_in` 可以在一个已经注册了宿主服务的 `Context` 上启动。
