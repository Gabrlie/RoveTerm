# 新增一个 Tauri 命令

本指南带你新增一个供前端调用的命令。命令只负责校验参数和调用服务，业务逻辑放在 crate 中（[防御性规则](../defensive-patterns/README.md#ipc-命令)）。

1. **确定服务**。命令要调用的服务必须已经由某个插件注册在 `Context` 上。如果 `AppState` 里还没有它的句柄，在 `src-tauri/src/state.rs` 的 `AppState::new` 中用 `ctx.require::<T>()` 取出来。
2. **编写命令**。在 `src-tauri/src/commands/` 下对应领域的文件中添加：
   ```rust
   #[tauri::command]
   pub async fn host_rename(state: State<'_, AppState>, id: String, name: String) -> Result<Host, CommandError> {
       let id: HostId = id.parse()?;               // 字符串 ID 先解析为 newtype
       if name.chars().count() > 128 {
           return Err(CommandError::new("input.invalid", "name too long"));
       }
       Ok(state.vault.rename_host(id, &name)?)     // 错误经 From 转为带 code 的 CommandError
   }
   ```
3. **映射错误码**。如果服务返回了新的错误类型，在 `src-tauri/src/error.rs` 中实现 `From<T> for CommandError`，为每种错误指定稳定的 `domain.reason` 错误码。
4. **登记命令**。把命令加入 `src-tauri/src/lib.rs` 的 `generate_handler!` 列表。
5. **前端封装**。在 `ui/src/lib/ipc.ts` 中添加带类型的调用函数；界面上显示的错误文案按 `code` 从 i18n 字典中取。
6. **验证**：
   1. `cargo clippy -p roveterm` 没有警告。
   2. `pnpm check:ui` 通过。
   3. 在 `pnpm tauri dev` 中调用该命令，并用非法参数确认返回 `input.invalid`。
