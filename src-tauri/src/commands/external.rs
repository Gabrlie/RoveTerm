//! Access for agents the user runs on their own (Claude Code or Codex in a terminal): while it is
//! on, RoveTerm serves an MCP server on a loopback port and writes its address and token to
//! `<data>/mcp.json`, readable only by the user; the agent starts `roveterm --mcp-bridge
//! --mcp-config <that file>`. The agent sees the saved hosts and works on them through the same
//! tools, approval rules, reviewer and records as RoveTerm's own assistant; credentials never
//! leave the vault. Each connection gets one conversation per host it touches, shown in that
//! host's AI panel and ended when the connection closes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rove_ai::detect;
use rove_ai::mcp::{McpListener, McpTools};
use rove_ai::session::AiSession;
use rove_ai::tools::ToolResult;
use rove_vault::HostId;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

use crate::error::CommandError;
use crate::state::AppState;

/// The host tools an outside agent may use; each takes the host by name or id.
const HOST_TOOLS: [&str; 7] = [
    "run_command",
    "read_file",
    "write_file",
    "edit_file",
    "list_dir",
    "read_notes",
    "update_notes",
];
/// Longest an install command may take.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The running server, while access is on.
pub struct ExternalAccess {
    config: PathBuf,
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ExternalAccess {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            config: data_dir.join("mcp.json"),
            task: std::sync::Mutex::new(None),
        }
    }

    fn running(&self) -> bool {
        self.task
            .lock()
            .map(|task| task.as_ref().is_some_and(|task| !task.is_finished()))
            .unwrap_or(false)
    }

    /// Starts serving and writes the config file; does nothing when already serving.
    async fn enable(&self, app: &AppHandle) -> Result<(), CommandError> {
        if self.running() {
            return Ok(());
        }
        let io = |error: std::io::Error| CommandError::new("app.io", error.to_string());
        let listener = McpListener::bind().await.map_err(io)?;
        write_private(
            &self.config,
            &serde_json::to_vec(&listener.config().map_err(io)?)
                .map_err(|error| CommandError::new("app.io", error.to_string()))?,
        )
        .map_err(io)?;
        let app = app.clone();
        let task = listener.serve_with(Arc::new(move || {
            Arc::new(HostTools {
                app: app.clone(),
                client: std::sync::Mutex::new("Local agent".into()),
                sessions: tokio::sync::Mutex::new(HashMap::new()),
            }) as Arc<dyn McpTools>
        }));
        if let Ok(mut slot) = self.task.lock() {
            *slot = Some(task);
        }
        tracing::info!(config = %self.config.display(), "access for local agents is on");
        Ok(())
    }

    /// Stops serving and removes the config file (also on exit; the next start writes it again
    /// when access is on). Conversations already open end when their agents disconnect.
    pub fn disable(&self) {
        if let Some(task) = self.task.lock().ok().and_then(|mut task| task.take()) {
            task.abort();
            tracing::info!("access for local agents is off");
        }
        match std::fs::remove_file(&self.config) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "removing the MCP config failed"),
        }
    }
}

/// Writes `bytes` to `path`, readable and writable by the user only (Unix); on Windows the
/// file inherits the user profile's access.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        // An older file keeps its mode through `open`: set it again.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

/// Turns access on at startup when the device preferences say so.
pub async fn restore(app: &AppHandle) {
    let state = app.state::<AppState>();
    let on = state
        .vault
        .device_preferences()
        .map(|prefs| prefs.agents.expose)
        .unwrap_or(false);
    if on && let Err(error) = state.external.enable(app).await {
        tracing::warn!(error = %error.message, "access for local agents could not start");
    }
}

/// The tools of one outside agent's connection.
struct HostTools {
    app: AppHandle,
    client: std::sync::Mutex<String>,
    /// Its conversation on each host it touched.
    sessions: tokio::sync::Mutex<HashMap<HostId, (String, Arc<AiSession>)>>,
}

impl HostTools {
    fn client(&self) -> String {
        self.client
            .lock()
            .map(|client| client.clone())
            .unwrap_or_default()
    }

    /// The saved host named `wanted` (its id, or its name ignoring case).
    fn host(&self, wanted: &str) -> Result<rove_vault::Host, (i64, String)> {
        let state = self.app.state::<AppState>();
        let hosts = state
            .vault
            .list_hosts()
            .map_err(|error| (-32603, error.to_string()))?;
        let wanted = wanted.trim();
        if let Some(host) = hosts.iter().find(|host| host.id.to_string() == wanted) {
            return Ok(host.clone());
        }
        let named: Vec<&rove_vault::Host> = hosts
            .iter()
            .filter(|host| host.name.eq_ignore_ascii_case(wanted))
            .collect();
        match named.as_slice() {
            [host] => Ok((*host).clone()),
            [] => Err((
                -32602,
                format!("no saved host is named {wanted:?}; list_hosts shows them"),
            )),
            _ => Err((
                -32602,
                format!("several hosts are named {wanted:?}; give the id from list_hosts"),
            )),
        }
    }

    async fn session(&self, host: HostId) -> Result<Arc<AiSession>, (i64, String)> {
        let mut sessions = self.sessions.lock().await;
        if let Some((_, session)) = sessions.get(&host) {
            return Ok(session.clone());
        }
        let state = self.app.state::<AppState>();
        let (id, session) =
            crate::commands::ai::start_external(&self.app, &state, &self.client(), host)
                .await
                .map_err(|error| (-32603, error.message))?;
        sessions.insert(host, (id, session.clone()));
        Ok(session)
    }

    fn list_hosts(&self) -> Result<ToolResult, (i64, String)> {
        let state = self.app.state::<AppState>();
        let hosts = state
            .vault
            .list_hosts()
            .map_err(|error| (-32603, error.to_string()))?;
        let listed: Vec<Value> = hosts
            .iter()
            .map(|host| {
                json!({
                    "id": host.id.to_string(),
                    "name": host.name,
                    "group": host.group,
                    "tags": host.tags,
                    "address": format!("{}@{}:{}", host.username, host.hostname, host.port),
                    "system": host.os.as_ref().and_then(|os| serde_json::to_value(os).ok()),
                })
            })
            .collect();
        Ok(ToolResult {
            text: serde_json::to_string_pretty(&listed).unwrap_or_default(),
            is_error: false,
        })
    }
}

#[async_trait]
impl McpTools for HostTools {
    fn instructions(&self) -> String {
        "RoveTerm manages the user's servers. Call list_hosts to see them; every other tool takes `host` (a name or id from list_hosts) and works on that server over RoveTerm's connection. RoveTerm checks each call against the user's approval rules: changes may wait for the user's approval in RoveTerm, and a refusal comes back as an error. You never see credentials.".into()
    }

    fn specs(&self) -> Value {
        host_specs()
    }

    fn client_named(&self, name: &str) {
        if let Ok(mut client) = self.client.lock() {
            name.clone_into(&mut client);
        }
    }

    async fn call(&self, name: &str, arguments: &Value) -> Result<ToolResult, (i64, String)> {
        if name == "list_hosts" {
            return self.list_hosts();
        }
        if !HOST_TOOLS.contains(&name) {
            return Err((-32602, format!("unknown tool {name}")));
        }
        let wanted = arguments
            .get("host")
            .and_then(Value::as_str)
            .ok_or_else(|| (-32602, "missing argument host".to_owned()))?;
        let host = self.host(wanted)?;
        let session = self.session(host.id).await?;
        let mut arguments = arguments.clone();
        if let Some(fields) = arguments.as_object_mut() {
            fields.remove("host");
        }
        rove_ai::tools::call(name, &arguments, &session).await
    }

    async fn closed(&self) {
        let sessions: Vec<String> = self
            .sessions
            .lock()
            .await
            .drain()
            .map(|(_, (id, _))| id)
            .collect();
        let state = self.app.state::<AppState>();
        for id in sessions {
            crate::commands::ai::end_external(&self.app, &state, &id).await;
        }
    }
}

/// `list_hosts`, and the host tools each with a required `host` argument.
fn host_specs() -> Value {
    let mut tools = vec![json!({
        "name": "list_hosts",
        "description": "List the servers saved in RoveTerm: id, name, group, tags, address and system.",
        "inputSchema": { "type": "object", "properties": {} }
    })];
    if let Some(specs) = rove_ai::tools::specs("the host named in `host`", true).as_array() {
        for spec in specs {
            let Some(name) = spec["name"].as_str() else {
                continue;
            };
            if !HOST_TOOLS.contains(&name) {
                continue;
            }
            let mut spec = spec.clone();
            spec["inputSchema"]["properties"]["host"] = json!({ "type": "string", "description": "The server: a name or id from list_hosts" });
            if let Some(required) = spec["inputSchema"]["required"].as_array_mut() {
                required.insert(0, json!("host"));
            } else {
                spec["inputSchema"]["required"] = json!(["host"]);
            }
            tools.push(spec);
        }
    }
    Value::Array(tools)
}

/// Whether access is on, and how an agent reaches it.
#[derive(Debug, Serialize)]
pub struct ExternalStatus {
    pub enabled: bool,
    /// The file the bridge reads.
    pub config: String,
    /// The command an agent runs (the app with its bridge arguments).
    pub command: Vec<String>,
}

fn command(config: &Path) -> Result<Vec<String>, CommandError> {
    let exe =
        std::env::current_exe().map_err(|error| CommandError::new("app.io", error.to_string()))?;
    Ok(vec![
        exe.to_string_lossy().into_owned(),
        "--mcp-bridge".into(),
        "--mcp-config".into(),
        config.to_string_lossy().into_owned(),
    ])
}

/// Whether agents the user runs may use the saved hosts, and the command they start.
#[tauri::command]
pub fn external_status(state: State<'_, AppState>) -> Result<ExternalStatus, CommandError> {
    Ok(ExternalStatus {
        enabled: state.external.running(),
        config: state.external.config.to_string_lossy().into_owned(),
        command: command(&state.external.config)?,
    })
}

/// Turns access for agents the user runs on or off, on this device.
#[tauri::command]
pub async fn external_set(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<ExternalStatus, CommandError> {
    let mut prefs = state.vault.device_preferences()?;
    prefs.agents.expose = enabled;
    state.vault.set_device_preferences(prefs)?;
    if enabled {
        state.external.enable(&app).await?;
    } else {
        state.external.disable();
    }
    external_status(state)
}

/// Adds RoveTerm's MCP server to `client` (`claude` or `codex`) with that agent's own command;
/// returns what it printed. An earlier entry is replaced.
#[tauri::command]
pub async fn external_install(
    state: State<'_, AppState>,
    client: String,
) -> Result<String, CommandError> {
    let (program, remove, add): (&str, &[&str], &[&str]) = match client.as_str() {
        "claude" => (
            "claude",
            &["mcp", "remove", "--scope", "user", "roveterm"],
            &["mcp", "add", "--scope", "user", "roveterm", "--"],
        ),
        "codex" => (
            "codex",
            &["mcp", "remove", "roveterm"],
            &["mcp", "add", "roveterm", "--"],
        ),
        _ => {
            return Err(CommandError::new(
                "input.invalid",
                "not an agent RoveTerm can set up",
            ));
        }
    };
    let path = detect::search_path();
    let exe = detect::which(program, &path).ok_or_else(|| {
        CommandError::new(
            "ai.no_agent",
            format!("{program} is not installed on this computer"),
        )
    })?;
    let bridge = command(&state.external.config)?;
    let run = |args: Vec<String>| {
        let exe = exe.clone();
        let path = path.clone();
        async move {
            let mut command = tokio::process::Command::new(&exe);
            command
                .args(&args)
                .env("PATH", &path)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            tokio::time::timeout(INSTALL_TIMEOUT, command.output()).await
        }
    };
    // A missing earlier entry is no error.
    let _ = run(remove.iter().map(|arg| (*arg).to_owned()).collect()).await;
    let mut args: Vec<String> = add.iter().map(|arg| (*arg).to_owned()).collect();
    args.extend(bridge);
    let output = run(args)
        .await
        .map_err(|_| CommandError::new("ai.timeout", format!("{program} did not finish in time")))?
        .map_err(|error| CommandError::new("ai.spawn", error.to_string()))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let text: String = text.trim().chars().take(2000).collect();
    if output.status.success() {
        tracing::info!(
            agent = program,
            "RoveTerm's MCP server was added to a local agent"
        );
        Ok(text)
    } else {
        Err(CommandError::new(
            "ai.agent",
            if text.is_empty() {
                format!("{program} failed")
            } else {
                text
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_tools_take_the_host_first() {
        let specs = host_specs();
        let names: Vec<&str> = specs
            .as_array()
            .unwrap()
            .iter()
            .map(|spec| spec["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "list_hosts",
                "run_command",
                "read_file",
                "write_file",
                "edit_file",
                "list_dir",
                "read_notes",
                "update_notes"
            ]
        );
        let run = &specs[1]["inputSchema"];
        assert_eq!(run["required"][0], "host");
        assert_eq!(run["properties"]["host"]["type"], "string");
    }

    #[cfg(unix)]
    #[test]
    fn the_config_is_readable_by_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"{}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
