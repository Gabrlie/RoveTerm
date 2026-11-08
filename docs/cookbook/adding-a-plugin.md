# 新增一个插件

本指南带你新增一个插件并在 profile 中启用。设计理由见[决策 0004](../decisions/0004-static-plugin-kernel.md)。

1. **确定 seam**。如果插件提供一项新能力，先在 Definition 所属的 crate 里写 trait；如果实现已有的 seam，直接依赖该 trait。
2. **定义配置**。为插件写一个配置结构，标注 `#[derive(Deserialize)]` 和 `#[serde(deny_unknown_fields, default)]`。
3. **实现 `Plugin`**：
   ```rust
   impl Plugin for MyPlugin {
       fn name(&self) -> &'static str { "my-plugin" }
       fn requires(&self) -> &'static [&'static str] { &["vault"] }
       fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
           let config: MyConfig = config.parse()?;
           let service = Arc::new(MyService::new(config));
           Ok(vec![ctx.provide::<dyn MySeam>(service)?])
       }
   }
   ```
4. **登记到 catalog**。把插件加入 `src-tauri/src/profile.rs` 的插件列表。
5. **在 profile 中启用**。把插件名加入对应 profile 的 `plugins` 数组，并按依赖关系排序。
6. **验证**：
   1. `cargo test -p <crate>` 通过。
   2. 在配置段写一个拼错的键，确认应用启动时报 `PluginConfig` 错误。
   3. 从 profile 中删掉被依赖的插件，确认启动时报 `MissingDependency` 错误。
