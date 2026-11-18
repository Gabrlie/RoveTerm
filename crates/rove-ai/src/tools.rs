//! The tools an agent has on the session's host: run a command, read, write and edit files, list
//! a directory and read the user's terminal. The built-in MCP server and the built-in assistant
//! offer the same tools with the same descriptions; every call goes through [`AiSession`], which
//! applies the policy.

use std::time::Duration;

use serde_json::{Value, json};

use crate::host::Limits;
use crate::session::AiSession;

/// Longest a tool may ask a command to run.
pub const MAX_TIMEOUT: u64 = 600;

/// A tool's answer: its text and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

fn result(text: String, is_error: bool) -> ToolResult {
    ToolResult { text, is_error }
}

/// The tools' names, descriptions and argument schemas, for a session on `host`; `notes` adds
/// the tools of the host's notes (a saved host).
pub fn specs(host: &str, notes: bool) -> Value {
    let mut specs = base_specs(host);
    if notes && let Some(list) = specs.as_array_mut() {
        list.push(json!({
            "name": "read_notes",
            "description": format!("Read RoveTerm's notes about {host}: what runs there, what to watch for, earlier changes worth knowing."),
            "inputSchema": { "type": "object", "properties": {} }
        }));
        list.push(json!({
            "name": "update_notes",
            "description": format!("Replace RoveTerm's notes about {host} with `text` (read them first and keep what still holds). Record lasting facts: services, versions, paths, decisions, changes you made. Usually needs the user's approval."),
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string", "description": "The whole notes, Markdown, at most 16 KiB" }
            }, "required": ["text"] }
        }));
    }
    specs
}

fn base_specs(host: &str) -> Value {
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
            "name": "edit_file",
            "description": format!("Change part of a text file on {host}: replace `old` (copied exactly from the file, with enough surrounding lines to occur once) with `new`. Prefer this to write_file for changes to existing files. Usually needs the user's approval."),
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "Absolute path" },
                "old": { "type": "string", "description": "The exact text to replace" },
                "new": { "type": "string", "description": "The text to put in its place" },
                "replace_all": { "type": "boolean", "description": "Replace every occurrence rather than one" }
            }, "required": ["path", "old", "new"] }
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

/// Runs tool `name` with `arguments` for `session`: its text result and whether it failed. Arguments
/// that do not fit the tool are an error the caller reports as invalid input.
pub async fn call(
    name: &str,
    arguments: &Value,
    session: &AiSession,
) -> Result<ToolResult, (i64, String)> {
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
                    result(report, output.exit_code != Some(0))
                }
                Err(error) => result(error.message, true),
            })
        }
        "read_file" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            let skip = arguments
                .get("line")
                .and_then(Value::as_u64)
                .map_or(0, |n| n.saturating_sub(1) as usize);
            let take = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(usize::MAX, |n| n as usize);
            Ok(match session.read_lines(path, skip, take).await {
                Ok(file) => {
                    let mut content = file.content;
                    if file.truncated {
                        content.push_str("\n(file truncated)");
                    }
                    result(content, false)
                }
                Err(error) => result(error.message, true),
            })
        }
        "write_file" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            let content = argument(arguments, "content")?;
            Ok(match session.write_file(path, content).await {
                Ok(()) => result(format!("wrote {} bytes to {path}", content.len()), false),
                Err(error) => result(error.message, true),
            })
        }
        "edit_file" => {
            let path = argument(arguments, "path")?;
            absolute(path)?;
            let old = argument(arguments, "old")?;
            let new = argument(arguments, "new")?;
            let all = arguments
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(match session.edit_file(path, old, new, all).await {
                Ok(count) => result(format!("replaced {count} place(s) in {path}"), false),
                Err(error) => result(error.message, true),
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
                    result(listing, false)
                }
                Err(error) => result(error.message, true),
            })
        }
        "read_notes" => Ok(match session.read_notes().await {
            Ok(text) if text.is_empty() => result("(no notes yet)".into(), false),
            Ok(text) => result(text, false),
            Err(error) => result(error.message, true),
        }),
        "update_notes" => {
            let text = argument(arguments, "text")?;
            Ok(match session.update_notes(text).await {
                Ok(()) => result("the notes are updated".into(), false),
                Err(error) => result(error.message, true),
            })
        }
        "terminal_snapshot" => {
            let lines = arguments
                .get("lines")
                .and_then(Value::as_u64)
                .map_or(200, |lines| lines.clamp(1, 2000) as usize);
            Ok(match session.read_terminal(lines).await {
                Ok(snapshot) => result(snapshot, false),
                Err(error) => result(error.message, true),
            })
        }
        _ => Err((-32602, format!("unknown tool {name}"))),
    }
}
