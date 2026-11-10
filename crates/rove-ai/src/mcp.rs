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

use crate::host::Limits;
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
/// Longest a tool may ask a command to run.
const MAX_TIMEOUT: u64 = 600;

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

fn tools(host: &str) -> Value {
    json!([
        {
            "name": "run_command",
            "description": format!("Run a shell command on {host}, the host the user is working on, and return its exit code and output. The user may be asked to approve it first; a refusal is returned as an error. Prefer this over local shells for anything on that host."),
            "inputSchema": { "type": "object", "properties": {
                "command": { "type": "string", "description": "A POSIX shell command line" },
                "cwd": { "type": "string", "description": "Absolute working directory; the login directory when omitted" },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT }
            }, "required": ["command"] }
        },
        {
            "name": "read_file",
            "description": format!("Read a text file on {host}."),
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "Absolute path" },
                "line": { "type": "integer", "minimum": 1, "description": "First line to return (1-based)" },
                "limit": { "type": "integer", "minimum": 1, "description": "Most lines to return" }
            }, "required": ["path"] }
        },
        {
            "name": "write_file",
            "description": format!("Create or replace a text file on {host} with the given content. Usually needs the user's approval."),
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "Absolute path" },
                "content": { "type": "string" }
            }, "required": ["path", "content"] }
        },
        {
            "name": "list_dir",
            "description": format!("List a directory on {host}."),
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "Absolute path" }
            }, "required": ["path"] }
        },
        {
            "name": "terminal_snapshot",
            "description": format!("Return the most recent lines shown in the user's terminal on {host}, to see what they are looking at."),
            "inputSchema": { "type": "object", "properties": {
                "lines": { "type": "integer", "minimum": 1, "maximum": 2000 }
            } }
        }
    ])
}

fn text(result: String, error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": result }], "isError": error })
}

fn argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, (i64, String)> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, format!("missing argument {name}")))
}

fn absolute(path: &str) -> Result<(), (i64, String)> {
    let ok =
        (path.starts_with('/') || path.get(1..3) == Some(":\\") || path.get(1..3) == Some(":/"))
            && path.len() <= 4096
            && !path.contains('\0');
    if ok {
        Ok(())
    } else {
        Err((-32602, "the path must be absolute".into()))
    }
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
        "tools/list" => Ok(json!({ "tools": tools(&session.host().label()) })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            call(name, &arguments, session).await
        }
        _ => Err((-32601, format!("unsupported method {method}"))),
    }
}

async fn call(name: &str, arguments: &Value, session: &AiSession) -> Result<Value, (i64, String)> {
    match name {
        "run_command" => {
            let command = argument(arguments, "command")?;
            let cwd = arguments.get("cwd").and_then(Value::as_str);
            if let Some(cwd) = cwd {
                absolute(cwd)?;
            }
            let limits = arguments
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| Limits {
                    timeout: Duration::from_secs(seconds.clamp(1, MAX_TIMEOUT)),
                    ..Limits::default()
                });
            Ok(match session.run_command(command, cwd, limits).await {
                Ok(output) => {
                    let mut report = format!(
                        "exit code: {}\n",
                        output
                            .exit_code
                            .map_or("none".to_owned(), |code| code.to_string())
                    );
                    if !output.stdout.is_empty() {
                        report.push_str(&format!("stdout:\n{}\n", output.stdout));
                    }
                    if !output.stderr.is_empty() {
                        report.push_str(&format!("stderr:\n{}\n", output.stderr));
                    }
                    if output.truncated {
                        report.push_str("(output truncated)\n");
                    }
                    text(report, output.exit_code != Some(0))
                }
                Err(error) => text(error.message, true),
            })
        }
        "read_file" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            Ok(match session.read_file(path).await {
                Ok(file) => {
                    let skip = arguments
                        .get("line")
                        .and_then(Value::as_u64)
                        .map_or(0, |line| line.saturating_sub(1) as usize);
                    let take = arguments
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map_or(usize::MAX, |limit| limit as usize);
                    let mut content: String = file
                        .content
                        .split_inclusive('\n')
                        .skip(skip)
                        .take(take)
                        .collect();
                    if file.truncated {
                        content.push_str("\n(file truncated)");
                    }
                    text(content, false)
                }
                Err(error) => text(error.message, true),
            })
        }
        "write_file" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            let content = argument(arguments, "content")?;
            Ok(match session.write_file(path, content).await {
                Ok(()) => text(format!("wrote {} bytes to {path}", content.len()), false),
                Err(error) => text(error.message, true),
            })
        }
        "list_dir" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            Ok(match session.list_dir(path).await {
                Ok((entries, more)) => {
                    let mut listing: String = entries
                        .iter()
                        .map(|entry| match entry.kind {
                            "dir" => format!("{}/\n", entry.name),
                            "file" => format!("{}\t{}\n", entry.name, entry.size),
                            kind => format!("{}\t({kind})\n", entry.name),
                        })
                        .collect();
                    if more {
                        listing.push_str("(more entries not shown)\n");
                    }
                    text(listing, false)
                }
                Err(error) => text(error.message, true),
            })
        }
        "terminal_snapshot" => {
            let lines = arguments
                .get("lines")
                .and_then(Value::as_u64)
                .map_or(200, |lines| lines.clamp(1, 2000) as usize);
            Ok(match session.read_terminal(lines).await {
                Ok(snapshot) => text(snapshot, false),
                Err(error) => text(error.message, true),
            })
        }
        _ => Err((-32602, format!("unknown tool {name}"))),
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
