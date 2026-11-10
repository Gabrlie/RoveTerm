//! The built-in MCP server: tools that run on the AI session's host (commands, files, the user's
//! terminal), each through the session's policy. Agents start MCP servers as child processes
//! speaking MCP over stdio, so the app binary has a bridge mode (`--mcp-bridge`) that connects that
//! stdio to the running app over loopback TCP. The bridge finds the app through two environment
//! variables set for it by the agent ([`ENV_ADDR`], [`ENV_TOKEN`]); the token is random per
//! session, kept only in memory, and checked before anything else.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::session::AiSession;

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

/// A listener for one session's bridge connections.
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

    /// The environment for the bridge process: address and token.
    pub fn bridge_env(&self) -> std::io::Result<Vec<(String, String)>> {
        Ok(vec![
            (ENV_ADDR.to_owned(), self.listener.local_addr()?.to_string()),
            (ENV_TOKEN.to_owned(), self.token.clone()),
        ])
    }

    /// Serves bridge connections for `session` until the returned task is aborted.
    pub fn serve(self, session: Arc<AiSession>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok((stream, _)) = self.listener.accept().await {
                let session = session.clone();
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
                    serve(reader, writer, session).await;
                });
            }
        })
    }
}

/// Runs the bridge: connects to the app named by the environment and copies stdin and stdout.
pub async fn bridge() -> std::io::Result<()> {
    let addr = std::env::var(ENV_ADDR)
        .map_err(|_| std::io::Error::other(format!("{ENV_ADDR} is not set")))?;
    let token = std::env::var(ENV_TOKEN)
        .map_err(|_| std::io::Error::other(format!("{ENV_TOKEN} is not set")))?;
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

/// Speaks MCP over `reader` and `writer` with tools of `session`.
pub async fn serve(
    reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    session: Arc<AiSession>,
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
        let reply = match handle(method, &params, &session).await {
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

async fn handle(method: &str, params: &Value, session: &AiSession) -> Result<Value, (i64, String)> {
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
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "roveterm", "title": "RoveTerm", "version": env!("CARGO_PKG_VERSION") },
                "instructions": format!("Tools here act on {}, the host the user is working on in RoveTerm. Use them for anything on that host instead of local shells.", session.host().label()),
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": crate::tools::specs(&session.host().label()) })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            crate::tools::call(name, &arguments, session)
                .await
                .map(text)
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
