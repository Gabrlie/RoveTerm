//! The built-in MCP server: tools that run on a host (commands, files, the user's terminal), each
//! through an AI session's policy. Agents start MCP servers as child processes speaking MCP over
//! stdio, so the app binary has a bridge mode (`--mcp-bridge`) that connects that stdio to the
//! running app over loopback TCP. The bridge finds the app through two environment variables set
//! by the agent RoveTerm started ([`ENV_ADDR`], [`ENV_TOKEN`]), or, for agents the user runs on
//! their own, through a config file the app writes when that access is on
//! (`--mcp-bridge --mcp-config <file>`, [`BridgeConfig`]). The token is random, and checked before
//! anything else. What a connection serves is an [`McpTools`]: one session's tools, or the app's
//! tools across hosts.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::session::AiSession;

/// The name the server is given to agents, and reports itself.
pub const SERVER_NAME: &str = "roveterm";
/// Where the bridge connects (`127.0.0.1:port`).
pub const ENV_ADDR: &str = "ROVETERM_MCP_ADDR";
/// The token the bridge presents.
pub const ENV_TOKEN: &str = "ROVETERM_MCP_TOKEN";
/// MCP protocol versions understood here, newest first.
const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// Longest message accepted.
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
/// Time a connection has to present its token.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// The tools one MCP connection serves.
#[async_trait]
pub trait McpTools: Send + Sync {
    /// What the agent is told at `initialize`.
    fn instructions(&self) -> String;
    /// The tools' names, descriptions and argument schemas.
    fn specs(&self) -> Value;
    /// The client named itself at `initialize` (`clientInfo.title`, else `name`).
    fn client_named(&self, _name: &str) {}
    /// Runs tool `name`; arguments that do not fit are an invalid-input error.
    async fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<crate::tools::ToolResult, (i64, String)>;
    /// The connection ended.
    async fn closed(&self) {}
}

/// Makes the tools of each new connection.
pub type ToolsFactory = Arc<dyn Fn() -> Arc<dyn McpTools> + Send + Sync>;

/// One session's tools, on its host.
pub struct SessionTools(pub Arc<AiSession>);

#[async_trait]
impl McpTools for SessionTools {
    fn instructions(&self) -> String {
        format!(
            "Tools here act on {}, the host the user is working on in RoveTerm. Use them for anything on that host instead of local shells.",
            self.0.host().label()
        )
    }

    fn specs(&self) -> Value {
        crate::tools::specs(&self.0.host().label())
    }

    async fn call(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<crate::tools::ToolResult, (i64, String)> {
        crate::tools::call(name, arguments, &self.0).await
    }
}

/// Where the bridge of an agent the user runs finds the app: written by the app to a file only
/// the user can read while that access is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// `127.0.0.1:port`.
    pub addr: String,
    pub token: String,
}

/// A listener for bridge connections.
pub struct McpListener {
    listener: TcpListener,
    token: String,
}

/// A random 256-bit token, hex encoded.
fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl McpListener {
    /// Listens on a free loopback port.
    pub async fn bind() -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind("127.0.0.1:0").await?,
            token: new_token()?,
        })
    }

    /// The address and token, for [`BridgeConfig`].
    pub fn config(&self) -> std::io::Result<BridgeConfig> {
        Ok(BridgeConfig {
            addr: self.listener.local_addr()?.to_string(),
            token: self.token.clone(),
        })
    }

    /// The environment for the bridge process: address and token.
    pub fn bridge_env(&self) -> std::io::Result<Vec<(String, String)>> {
        Ok(vec![
            (ENV_ADDR.to_owned(), self.listener.local_addr()?.to_string()),
            (ENV_TOKEN.to_owned(), self.token.clone()),
        ])
    }

    /// Serves bridge connections for `session` until the returned task is aborted.
    pub fn serve(self, session: Arc<AiSession>) -> tokio::task::JoinHandle<()> {
        self.serve_with(Arc::new(move || {
            Arc::new(SessionTools(session.clone())) as Arc<dyn McpTools>
        }))
    }

    /// Serves bridge connections, each with tools from `factory`, until the returned task is
    /// aborted.
    pub fn serve_with(self, factory: ToolsFactory) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok((stream, _)) = self.listener.accept().await {
                let tools = factory();
                let token = self.token.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);
                    let mut presented = String::new();
                    let read = tokio::time::timeout(
                        HANDSHAKE,
                        (&mut reader).take(128).read_line(&mut presented),
                    )
                    .await;
                    if !matches!(read, Ok(Ok(_)))
                        || !same(presented.trim_end().as_bytes(), token.as_bytes())
                    {
                        tracing::warn!("refused an MCP bridge connection with a wrong token");
                        let _ = writer.shutdown().await;
                        return;
                    }
                    serve(reader, writer, tools.clone()).await;
                    tools.closed().await;
                });
            }
        })
    }
}

/// Runs the bridge: connects to the app named by `config` (a [`BridgeConfig`] file), else by the
/// environment, and copies stdin and stdout.
pub async fn bridge(config: Option<&std::path::Path>) -> std::io::Result<()> {
    let BridgeConfig { addr, token } = match config {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|error| {
                std::io::Error::other(format!(
                    "{error}: turn on access for local agents in RoveTerm's AI settings ({})",
                    path.display()
                ))
            })?;
            serde_json::from_str(&text)
                .map_err(|_| std::io::Error::other("the bridge config is not valid"))?
        }
        None => BridgeConfig {
            addr: std::env::var(ENV_ADDR)
                .map_err(|_| std::io::Error::other(format!("{ENV_ADDR} is not set")))?,
            token: std::env::var(ENV_TOKEN)
                .map_err(|_| std::io::Error::other(format!("{ENV_TOKEN} is not set")))?,
        },
    };
    if !addr.starts_with("127.0.0.1:") {
        return Err(std::io::Error::other(
            "the bridge only connects to loopback",
        ));
    }
    let stream = TcpStream::connect(&addr).await?;
    let (mut from_app, mut to_app) = stream.into_split();
    to_app.write_all(format!("{token}\n").as_bytes()).await?;
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        result = tokio::io::copy(&mut stdin, &mut to_app) => result.map(|_| ()),
        result = tokio::io::copy(&mut from_app, &mut stdout) => result.map(|_| ()),
    }
}

/// Speaks MCP over `reader` and `writer` with `tools`.
pub async fn serve(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    tools: Arc<dyn McpTools>,
) {
    let mut lines = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match (&mut lines)
            .take(MAX_MESSAGE as u64 + 1)
            .read_line(&mut line)
            .await
        {
            Ok(0) | Err(_) => return,
            Ok(read) if read > MAX_MESSAGE => return,
            Ok(_) => {}
        }
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(id) = message.get("id").cloned().filter(|id| !id.is_null()) else {
            // Notifications (`notifications/initialized`, cancellations) need no answer.
            continue;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let reply = match handle(method, &params, tools.as_ref()).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, text)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } })
            }
        };
        if writer
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .is_err()
            || writer.flush().await.is_err()
        {
            return;
        }
    }
}

fn text(result: crate::tools::ToolResult) -> Value {
    json!({ "content": [{ "type": "text", "text": result.text }], "isError": result.is_error })
}

async fn handle(
    method: &str,
    params: &Value,
    tools: &dyn McpTools,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = VERSIONS
                .iter()
                .find(|version| **version == asked)
                .unwrap_or(&VERSIONS[0]);
            let client = params
                .pointer("/clientInfo/title")
                .or_else(|| params.pointer("/clientInfo/name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let client: String = client
                .chars()
                .filter(|c| !c.is_control())
                .take(60)
                .collect();
            if !client.trim().is_empty() {
                tools.client_named(client.trim());
            }
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "title": "RoveTerm", "version": env!("CARGO_PKG_VERSION") },
                "instructions": tools.instructions(),
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools.specs() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            tools.call(name, &arguments).await.map(text)
        }
        _ => Err((-32601, format!("unsupported method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
        assert_eq!(new_token().unwrap().len(), 64);
    }
}
