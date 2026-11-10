//! One AI session: an agent conversation bound to one host. Every operation the agent asks for —
//! through ACP or the built-in MCP tools — is assessed, decided by the session's [`Policy`],
//! approved by the user when needed, run on the [`Host`], and reported as [`AiEvent`]s, from which
//! the app shows progress and records the audit trail.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, oneshot, watch};

use crate::acp::{
    self, Client, ClientError, PermissionOption, PermissionOutcome, SessionUpdate, TerminalState,
    ToolCall,
};
use crate::host::{self, CommandOutput, Entry, Host, HostError, Limits, shell_quote};
use crate::policy::{Answer, Decision, Mode, Operation, Policy, ToolKind};
use crate::risk::{self, Assessment, Rules};

/// What happened in a session, for the UI and the audit trail.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AiEvent {
    /// The agent streamed something.
    Update { update: SessionUpdate },
    /// An operation waits for the user's answer through [`AiSession::answer`].
    Approval {
        request_id: u64,
        operation: Operation,
        assessment: Assessment,
        /// "Allow for this session" may be offered.
        rememberable: bool,
        /// One of the agent's own tools, which runs on this machine whatever the session's host.
        agent_tool: bool,
    },
    /// An approval was answered (`None`: withdrawn because the turn was cancelled).
    Approved {
        request_id: u64,
        answer: Option<Answer>,
    },
    /// An operation ran, or was refused, on the session's host.
    Executed {
        operation: Operation,
        assessment: Assessment,
        /// `true` when it ran after the user approved it rather than by policy.
        approved: bool,
        outcome: Outcome,
    },
}

/// The result of an operation, as recorded.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    Command {
        output: CommandOutput,
    },
    /// The agent's own tool was allowed to run (on this machine).
    AgentTool,
    Read {
        bytes: usize,
        sha256: String,
        truncated: bool,
    },
    Written {
        bytes: usize,
        sha256: String,
        previous_sha256: Option<String>,
        lines_before: usize,
        lines_after: usize,
    },
    Listed {
        entries: usize,
    },
    Denied,
    Failed {
        error: String,
    },
}

/// Receives a session's events.
pub type EventSink = Arc<dyn Fn(AiEvent) + Send + Sync>;

/// Gives the recent text of the user's terminal on the session's host, for the agent to see.
#[async_trait]
pub trait TerminalSource: Send + Sync {
    /// The last `lines` lines, or `None` when no terminal of the host is open.
    async fn recent(&self, lines: usize) -> Option<String>;
}

/// Settings of one session.
#[derive(Debug, Clone, Default)]
pub struct SessionSettings {
    pub mode: Mode,
    pub rules: Rules,
    pub limits: Limits,
}

/// A command started with `terminal/create`.
struct Job {
    /// The result once it has ended; an error text when it failed to run.
    done: watch::Receiver<Option<Result<CommandOutput, String>>>,
    task: tokio::task::JoinHandle<()>,
}

pub struct AiSession {
    host: Arc<dyn Host>,
    policy: Mutex<Policy>,
    limits: Limits,
    sink: EventSink,
    /// The agent's id for this session, once known; requests for other sessions are refused.
    acp_session: OnceLock<String>,
    approvals: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    next_id: AtomicU64,
    jobs: Mutex<HashMap<String, Job>>,
    terminal: OnceLock<Arc<dyn TerminalSource>>,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn failure(error: HostError) -> ClientError {
    let code = match error {
        HostError::Fs(rove_fs::FsError::NotFound(_)) => acp::code::NOT_FOUND,
        _ => acp::code::INTERNAL,
    };
    ClientError::new(code, error.to_string())
}

/// The command line an agent tool runs, from its raw input (`{"command": "…"}` or
/// `{"command": ["bash", "-lc", "…"]}`).
fn tool_command(raw_input: Option<&Value>) -> Option<String> {
    let command = raw_input?
        .get("command")
        .or_else(|| raw_input?.get("cmd"))?;
    match command {
        Value::String(line) => Some(line.clone()),
        Value::Array(words) => {
            let words: Option<Vec<&str>> = words.iter().map(Value::as_str).collect();
            Some(
                words?
                    .iter()
                    .map(|word| shell_quote(word))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
        _ => None,
    }
}

/// Picks the agent's option for an allow (`allow`) or a refusal, preferring the one-time kinds:
/// remembering is the policy's business, not the agent's.
fn pick(options: &[PermissionOption], allow: bool) -> PermissionOutcome {
    let kinds: &[&str] = if allow {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    kinds
        .iter()
        .find_map(|kind| options.iter().find(|option| option.kind == *kind))
        .map_or(PermissionOutcome::Cancelled, |option| {
            PermissionOutcome::Selected(option.option_id.clone())
        })
}

impl AiSession {
    pub fn new(host: Arc<dyn Host>, settings: SessionSettings, sink: EventSink) -> Self {
        Self {
            host,
            policy: Mutex::new(Policy::new(settings.mode, settings.rules)),
            limits: settings.limits,
            sink,
            acp_session: OnceLock::new(),
            approvals: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            jobs: Mutex::new(HashMap::new()),
            terminal: OnceLock::new(),
        }
    }

    /// Lets the agent read the user's terminal through [`AiSession::read_terminal`].
    pub fn set_terminal_source(&self, source: Arc<dyn TerminalSource>) {
        let _ = self.terminal.set(source);
    }

    /// The last `lines` lines of the user's terminal on the host, after the policy allows it.
    pub async fn read_terminal(&self, lines: usize) -> Result<String, ClientError> {
        let operation = Operation::ReadTerminal;
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied(
                "the user did not allow reading the terminal",
            ));
        }
        let text = match self.terminal.get() {
            Some(source) => source.recent(lines).await,
            None => None,
        };
        let Some(text) = text else {
            self.report(
                operation,
                assessment,
                approved,
                Outcome::Failed {
                    error: "no terminal is open".into(),
                },
            );
            return Err(ClientError::new(
                acp::code::NOT_FOUND,
                "no terminal of this host is open",
            ));
        };
        let outcome = Outcome::Read {
            bytes: text.len(),
            sha256: sha256(text.as_bytes()),
            truncated: false,
        };
        self.report(operation, assessment, approved, outcome);
        Ok(text)
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    /// Binds the session to the agent's session id.
    pub fn bind(&self, acp_session: &str) {
        let _ = self.acp_session.set(acp_session.to_owned());
    }

    pub async fn set_mode(&self, mode: Mode) {
        self.policy.lock().await.set_mode(mode);
    }

    /// Answers approval `request_id`; returns whether it was waiting.
    pub async fn answer(&self, request_id: u64, answer: Answer) -> bool {
        match self.approvals.lock().await.remove(&request_id) {
            Some(waiter) => waiter.send(answer).is_ok(),
            None => false,
        }
    }

    /// Withdraws every waiting approval as refused, when the turn is cancelled.
    pub async fn withdraw_approvals(&self) {
        for (request_id, _) in self.approvals.lock().await.drain() {
            (self.sink)(AiEvent::Approved {
                request_id,
                answer: None,
            });
        }
    }

    /// Ends every command still running.
    pub async fn stop_jobs(&self) {
        for (_, job) in self.jobs.lock().await.drain() {
            job.task.abort();
        }
    }

    fn check_session(&self, session_id: &str) -> Result<(), ClientError> {
        match self.acp_session.get() {
            Some(own) if own != session_id => Err(ClientError::new(
                acp::code::INVALID_PARAMS,
                "unknown session",
            )),
            _ => Ok(()),
        }
    }

    /// Decides on `operation` and, when asked, waits for the user. Returns whether it may run and
    /// whether the user approved it.
    async fn authorize(
        &self,
        operation: &Operation,
        assessment: &Assessment,
        agent_tool: bool,
    ) -> (bool, bool) {
        let decision = self.policy.lock().await.decide(operation, assessment);
        let rememberable = match decision {
            Decision::Allow => return (true, false),
            Decision::Ask { rememberable } => rememberable,
        };
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.approvals.lock().await.insert(request_id, sender);
        (self.sink)(AiEvent::Approval {
            request_id,
            operation: operation.clone(),
            assessment: assessment.clone(),
            rememberable,
            agent_tool,
        });
        let Ok(answer) = receiver.await else {
            return (false, false);
        };
        // "Allow for this session" where it was not offered counts as once.
        let answer = if answer == Answer::AllowSession && !rememberable {
            Answer::AllowOnce
        } else {
            answer
        };
        (self.sink)(AiEvent::Approved {
            request_id,
            answer: Some(answer),
        });
        let allowed = self
            .policy
            .lock()
            .await
            .answer(operation, assessment, answer);
        (allowed, allowed)
    }

    fn report(
        &self,
        operation: Operation,
        assessment: Assessment,
        approved: bool,
        outcome: Outcome,
    ) {
        (self.sink)(AiEvent::Executed {
            operation,
            assessment,
            approved,
            outcome,
        });
    }

    async fn assess(&self, operation: &Operation) -> Assessment {
        operation.assess(self.policy.lock().await.rules())
    }

    /// Runs `command` on the host after the policy allows it.
    pub async fn run_command(
        &self,
        command: &str,
        cwd: Option<&str>,
        limits: Option<Limits>,
    ) -> Result<CommandOutput, ClientError> {
        let operation = Operation::Command {
            command: command.to_owned(),
        };
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied("the user did not allow this command"));
        }
        let result = self
            .host
            .run(command, cwd, &limits.unwrap_or(self.limits))
            .await;
        match result {
            Ok(output) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Command {
                        output: output.clone(),
                    },
                );
                Ok(output)
            }
            Err(error) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Failed {
                        error: error.to_string(),
                    },
                );
                Err(failure(error))
            }
        }
    }

    /// Reads a text file on the host after the policy allows it.
    pub async fn read_file(&self, path: &str) -> Result<host::FileText, ClientError> {
        let operation = Operation::ReadFile {
            path: path.to_owned(),
        };
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied(
                "the user did not allow reading this file",
            ));
        }
        match host::read_text(self.host.as_ref(), path, &self.limits).await {
            Ok(text) => {
                let outcome = Outcome::Read {
                    bytes: text.content.len(),
                    sha256: sha256(text.content.as_bytes()),
                    truncated: text.truncated,
                };
                self.report(operation, assessment, approved, outcome);
                Ok(text)
            }
            Err(error) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Failed {
                        error: error.to_string(),
                    },
                );
                Err(failure(error))
            }
        }
    }

    /// Writes a text file on the host after the policy allows it.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), ClientError> {
        let operation = Operation::WriteFile {
            path: path.to_owned(),
        };
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied(
                "the user did not allow writing this file",
            ));
        }
        // The previous contents, for the audit trail.
        let previous = host::read_text(self.host.as_ref(), path, &self.limits)
            .await
            .ok();
        match host::write_text(self.host.as_ref(), path, content, &self.limits).await {
            Ok(()) => {
                let outcome = Outcome::Written {
                    bytes: content.len(),
                    sha256: sha256(content.as_bytes()),
                    previous_sha256: previous
                        .as_ref()
                        .map(|text| sha256(text.content.as_bytes())),
                    lines_before: previous
                        .as_ref()
                        .map_or(0, |text| text.content.lines().count()),
                    lines_after: content.lines().count(),
                };
                self.report(operation, assessment, approved, outcome);
                Ok(())
            }
            Err(error) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Failed {
                        error: error.to_string(),
                    },
                );
                Err(failure(error))
            }
        }
    }

    /// Lists a directory on the host after the policy allows it.
    pub async fn list_dir(&self, path: &str) -> Result<(Vec<Entry>, bool), ClientError> {
        let operation = Operation::ListDir {
            path: path.to_owned(),
        };
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied(
                "the user did not allow listing this directory",
            ));
        }
        match host::list_dir(self.host.as_ref(), path, &self.limits).await {
            Ok((entries, more)) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Listed {
                        entries: entries.len(),
                    },
                );
                Ok((entries, more))
            }
            Err(error) => {
                self.report(
                    operation,
                    assessment,
                    approved,
                    Outcome::Failed {
                        error: error.to_string(),
                    },
                );
                Err(failure(error))
            }
        }
    }
}

#[async_trait]
impl Client for AiSession {
    async fn session_update(&self, session_id: &str, update: SessionUpdate) {
        if self.check_session(session_id).is_ok() {
            (self.sink)(AiEvent::Update { update });
        }
    }

    async fn request_permission(
        &self,
        session_id: &str,
        call: ToolCall,
        options: Vec<PermissionOption>,
    ) -> PermissionOutcome {
        if self.check_session(session_id).is_err() {
            return PermissionOutcome::Cancelled;
        }
        let kind: ToolKind = call
            .kind
            .as_deref()
            .and_then(|kind| serde_json::from_value(Value::String(kind.to_owned())).ok())
            .unwrap_or(ToolKind::Other);
        let operation = Operation::AgentTool {
            kind,
            title: call.title.clone().unwrap_or_default(),
            command: tool_command(call.raw_input.as_ref()),
        };
        let mut assessment = self.assess(&operation).await;
        // Files the tool changes count as writes.
        if matches!(kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) {
            for path in &call.locations {
                assessment.merge(risk::assess_write(path));
            }
        }
        let (allowed, approved) = self.authorize(&operation, &assessment, true).await;
        let outcome = if allowed {
            Outcome::AgentTool
        } else {
            Outcome::Denied
        };
        self.report(operation, assessment, approved, outcome);
        pick(&options, allowed)
    }

    async fn read_text_file(
        &self,
        session_id: &str,
        path: &str,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, ClientError> {
        self.check_session(session_id)?;
        let text = self.read_file(path).await?;
        if line.is_none() && limit.is_none() {
            return Ok(text.content);
        }
        let skip = line.map_or(0, |line| line.saturating_sub(1) as usize);
        let take = limit.map_or(usize::MAX, |limit| limit as usize);
        Ok(text
            .content
            .split_inclusive('\n')
            .skip(skip)
            .take(take)
            .collect())
    }

    async fn write_text_file(
        &self,
        session_id: &str,
        path: &str,
        content: &str,
    ) -> Result<(), ClientError> {
        self.check_session(session_id)?;
        self.write_file(path, content).await
    }

    async fn create_terminal(
        &self,
        session_id: &str,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: Option<&str>,
        output_limit: Option<usize>,
    ) -> Result<String, ClientError> {
        self.check_session(session_id)?;
        // One shell line: assignments, then the program and its quoted arguments. A command
        // without arguments may itself be a whole line.
        let mut line: Vec<String> = env
            .iter()
            .map(|(name, value)| format!("{name}={}", shell_quote(value)))
            .collect();
        line.push(if args.is_empty() {
            command.to_owned()
        } else {
            shell_quote(command)
        });
        line.extend(args.iter().map(|arg| shell_quote(arg)));
        let line = line.join(" ");
        let operation = Operation::Command {
            command: line.clone(),
        };
        let assessment = self.assess(&operation).await;
        let (allowed, approved) = self.authorize(&operation, &assessment, false).await;
        if !allowed {
            self.report(operation, assessment, approved, Outcome::Denied);
            return Err(ClientError::denied("the user did not allow this command"));
        }
        let limits = Limits {
            output: output_limit.map_or(self.limits.output, |limit| limit.min(self.limits.output)),
            ..self.limits
        };
        let id = format!("term-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, done) = watch::channel(None);
        let host = self.host.clone();
        let sink = self.sink.clone();
        let cwd = cwd.map(str::to_owned);
        let task = tokio::spawn(async move {
            let result = host.run(&line, cwd.as_deref(), &limits).await;
            let outcome = match &result {
                Ok(output) => Outcome::Command {
                    output: output.clone(),
                },
                Err(error) => Outcome::Failed {
                    error: error.to_string(),
                },
            };
            sink(AiEvent::Executed {
                operation,
                assessment,
                approved,
                outcome,
            });
            let _ = sender.send(Some(result.map_err(|error| error.to_string())));
        });
        self.jobs
            .lock()
            .await
            .insert(id.clone(), Job { done, task });
        Ok(id)
    }

    async fn terminal_output(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<TerminalState, ClientError> {
        self.check_session(session_id)?;
        let jobs = self.jobs.lock().await;
        let job = jobs
            .get(terminal_id)
            .ok_or_else(|| ClientError::new(acp::code::INVALID_PARAMS, "unknown terminal"))?;
        let state = match &*job.done.borrow() {
            None if job.task.is_finished() => TerminalState {
                exit: Some(None),
                ..TerminalState::default()
            },
            None => TerminalState::default(),
            Some(Ok(output)) => TerminalState {
                output: format!("{}{}", output.stdout, output.stderr),
                truncated: output.truncated,
                exit: Some(output.exit_code.and_then(|code| u32::try_from(code).ok())),
            },
            Some(Err(error)) => TerminalState {
                output: error.clone(),
                truncated: false,
                exit: Some(None),
            },
        };
        Ok(state)
    }

    async fn wait_for_terminal_exit(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<Option<u32>, ClientError> {
        self.check_session(session_id)?;
        let mut done = {
            let jobs = self.jobs.lock().await;
            jobs.get(terminal_id)
                .ok_or_else(|| ClientError::new(acp::code::INVALID_PARAMS, "unknown terminal"))?
                .done
                .clone()
        };
        // A killed job never sends; its sender is dropped and waiting ends.
        let result = done
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|value| value.clone());
        Ok(match result {
            Some(Ok(output)) => output.exit_code.and_then(|code| u32::try_from(code).ok()),
            _ => None,
        })
    }

    async fn kill_terminal(&self, session_id: &str, terminal_id: &str) -> Result<(), ClientError> {
        self.check_session(session_id)?;
        if let Some(job) = self.jobs.lock().await.get(terminal_id) {
            job.task.abort();
        }
        Ok(())
    }

    async fn release_terminal(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<(), ClientError> {
        self.check_session(session_id)?;
        if let Some(job) = self.jobs.lock().await.remove(terminal_id) {
            job.task.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_tool_commands() {
        assert_eq!(
            tool_command(Some(&json!({ "command": "ls -la" }))),
            Some("ls -la".into())
        );
        assert_eq!(
            tool_command(Some(&json!({ "command": ["bash", "-lc", "rm -rf /"] }))),
            Some("'bash' '-lc' 'rm -rf /'".into())
        );
        assert_eq!(tool_command(Some(&json!({ "path": "/a" }))), None);
    }

    #[test]
    fn prefers_one_time_options() {
        let options = vec![
            PermissionOption {
                option_id: "always".into(),
                name: "Always".into(),
                kind: "allow_always".into(),
            },
            PermissionOption {
                option_id: "once".into(),
                name: "Once".into(),
                kind: "allow_once".into(),
            },
            PermissionOption {
                option_id: "no".into(),
                name: "No".into(),
                kind: "reject_once".into(),
            },
        ];
        assert_eq!(
            pick(&options, true),
            PermissionOutcome::Selected("once".into())
        );
        assert_eq!(
            pick(&options, false),
            PermissionOutcome::Selected("no".into())
        );
        assert_eq!(pick(&[], true), PermissionOutcome::Cancelled);
    }
}
