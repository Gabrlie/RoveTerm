//! Whether an agent's operation may run now or needs the user's approval. Only this policy
//! decides: options an agent offers (such as "always allow") play no part. One `Policy` belongs to
//! one AI session, which is bound to one host, so approvals remembered here apply to that host only.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::risk::{self, Assessment, Risk, Rules};

/// How much the user is asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Read-only operations run; modifications are asked about; dangerous ones always are.
    #[default]
    AskByRisk,
    /// Everything is asked about.
    AskAll,
    /// Everything runs and is recorded, except dangerous operations, which are still asked about.
    Auto,
}

/// The kind of an agent's own tool, as ACP names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[serde(other)]
    Other,
}

/// Something an agent wants to do on its session's host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Run a shell command line.
    Command {
        command: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
    },
    ListDir {
        path: String,
    },
    /// One of the agent's own tools; `command` is set when it runs a shell command.
    AgentTool {
        kind: ToolKind,
        title: String,
        command: Option<String>,
    },
}

impl Operation {
    /// The operation's risk under `rules`.
    pub fn assess(&self, rules: &Rules) -> Assessment {
        match self {
            Self::Command { command } => risk::assess_command(command, rules),
            Self::ReadFile { path } | Self::ListDir { path } => risk::assess_read(path),
            Self::WriteFile { path } => risk::assess_write(path),
            Self::AgentTool {
                command: Some(command),
                ..
            } => risk::assess_command(command, rules),
            Self::AgentTool { kind, .. } => {
                let mut assessment = Assessment::new();
                let (risk, code) = match kind {
                    ToolKind::Read | ToolKind::Search | ToolKind::Think | ToolKind::Fetch => {
                        (Risk::ReadOnly, "")
                    }
                    ToolKind::Delete => (Risk::Modify, "delete"),
                    ToolKind::Edit | ToolKind::Move => (Risk::Modify, "write_file"),
                    // A command the agent did not show, or something unknown.
                    ToolKind::Execute | ToolKind::SwitchMode | ToolKind::Other => {
                        assessment.opaque = true;
                        (Risk::Modify, "agent_tool")
                    }
                };
                assessment.risk = risk;
                if !code.is_empty() {
                    assessment.reasons.push(risk::Reason {
                        code,
                        detail: String::new(),
                    });
                }
                assessment
            }
        }
    }

    /// What remembering an approval of this operation would allow from now on, or `None` when it
    /// cannot be remembered.
    fn grants(&self, assessment: &Assessment) -> Option<Vec<String>> {
        if assessment.risk == Risk::Dangerous || assessment.opaque {
            return None;
        }
        match self {
            Self::Command { .. }
            | Self::AgentTool {
                command: Some(_), ..
            } => {
                if assessment.programs.is_empty() {
                    return None;
                }
                Some(
                    assessment
                        .programs
                        .iter()
                        .map(|program| format!("run:{program}"))
                        .collect(),
                )
            }
            // Writes are remembered per directory, secret reads per file.
            Self::WriteFile { path } => Some(vec![format!("write:{}", parent(path))]),
            Self::ReadFile { path } | Self::ListDir { path } => Some(vec![format!("read:{path}")]),
            Self::AgentTool { kind, .. } => Some(vec![format!("tool:{kind:?}")]),
        }
    }
}

fn parent(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(0) => "/",
        Some(at) => &path[..at],
        None => ".",
    }
}

/// The policy's answer for one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Runs without asking.
    Allow,
    /// The user decides. `rememberable` tells whether "allow for this session" may be offered.
    Ask { rememberable: bool },
}

/// The user's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    AllowOnce,
    /// Allows this operation and others like it for the rest of the session.
    AllowSession,
    Deny,
}

/// Decisions for one AI session.
#[derive(Debug, Clone)]
pub struct Policy {
    mode: Mode,
    rules: Rules,
    granted: HashSet<String>,
}

impl Policy {
    pub fn new(mode: Mode, rules: Rules) -> Self {
        Self {
            mode,
            rules,
            granted: HashSet::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// Decides on `operation`, already assessed as `assessment`.
    pub fn decide(&self, operation: &Operation, assessment: &Assessment) -> Decision {
        let grants = operation.grants(assessment);
        let ask = Decision::Ask {
            rememberable: grants.is_some(),
        };
        if assessment.risk == Risk::Dangerous {
            return Decision::Ask {
                rememberable: false,
            };
        }
        let granted = grants
            .as_ref()
            .is_some_and(|grants| grants.iter().all(|grant| self.granted.contains(grant)));
        match self.mode {
            Mode::Auto => Decision::Allow,
            _ if granted => Decision::Allow,
            Mode::AskByRisk if assessment.risk == Risk::ReadOnly => Decision::Allow,
            Mode::AskByRisk | Mode::AskAll => ask,
        }
    }

    /// Records the user's `answer` to an approval of `operation`; returns whether it may run.
    /// "Allow for the session" is honoured only where [`Policy::decide`] offered it.
    pub fn answer(
        &mut self,
        operation: &Operation,
        assessment: &Assessment,
        answer: Answer,
    ) -> bool {
        match answer {
            Answer::Deny => false,
            Answer::AllowOnce => true,
            Answer::AllowSession => {
                if let Some(grants) = operation.grants(assessment) {
                    self.granted.extend(grants);
                }
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(line: &str) -> Operation {
        Operation::Command {
            command: line.to_owned(),
        }
    }

    fn decide(policy: &Policy, operation: &Operation) -> Decision {
        policy.decide(operation, &operation.assess(policy.rules()))
    }

    const ASK: Decision = Decision::Ask { rememberable: true };
    const ASK_ONCE: Decision = Decision::Ask {
        rememberable: false,
    };

    #[test]
    fn modes() {
        let ls = command("ls -la");
        let restart = command("systemctl restart nginx");
        let wipe = command("rm -rf /");
        let by_risk = Policy::new(Mode::AskByRisk, Rules::default());
        assert_eq!(decide(&by_risk, &ls), Decision::Allow);
        assert_eq!(decide(&by_risk, &restart), ASK);
        assert_eq!(decide(&by_risk, &wipe), ASK_ONCE);
        let all = Policy::new(Mode::AskAll, Rules::default());
        assert_eq!(decide(&all, &ls), ASK);
        assert_eq!(decide(&all, &restart), ASK);
        let auto = Policy::new(Mode::Auto, Rules::default());
        assert_eq!(decide(&auto, &restart), Decision::Allow);
        assert_eq!(decide(&auto, &wipe), ASK_ONCE);
    }

    #[test]
    fn session_approvals_cover_the_same_programs_only() {
        let mut policy = Policy::new(Mode::AskByRisk, Rules::default());
        let restart = command("systemctl restart nginx");
        let assessment = restart.assess(policy.rules());
        assert!(policy.answer(&restart, &assessment, Answer::AllowSession));
        assert_eq!(
            decide(&policy, &command("systemctl reload nginx")),
            Decision::Allow
        );
        assert_eq!(
            decide(
                &policy,
                &command("systemctl restart nginx && apt upgrade -y")
            ),
            ASK
        );
        // Dangerous operations are never remembered, even when asked to.
        let wipe = command("rm -rf /");
        let assessment = wipe.assess(policy.rules());
        assert!(policy.answer(&wipe, &assessment, Answer::AllowSession));
        assert_eq!(decide(&policy, &wipe), ASK_ONCE);
        assert!(!policy.answer(&wipe, &assessment, Answer::Deny));
    }

    #[test]
    fn opaque_commands_are_asked_each_time() {
        let mut policy = Policy::new(Mode::AskByRisk, Rules::default());
        let script = command("bash deploy.sh");
        assert_eq!(decide(&policy, &script), ASK_ONCE);
        let assessment = script.assess(policy.rules());
        policy.answer(&script, &assessment, Answer::AllowSession);
        assert_eq!(decide(&policy, &script), ASK_ONCE);
    }

    #[test]
    fn file_operations() {
        let mut policy = Policy::new(Mode::AskByRisk, Rules::default());
        assert_eq!(
            decide(
                &policy,
                &Operation::ReadFile {
                    path: "/var/log/syslog".into()
                }
            ),
            Decision::Allow
        );
        assert_eq!(
            decide(
                &policy,
                &Operation::ReadFile {
                    path: "/root/.ssh/id_rsa".into()
                }
            ),
            ASK
        );
        let write = Operation::WriteFile {
            path: "/srv/app/a.yml".into(),
        };
        assert_eq!(decide(&policy, &write), ASK);
        let assessment = write.assess(policy.rules());
        policy.answer(&write, &assessment, Answer::AllowSession);
        assert_eq!(
            decide(
                &policy,
                &Operation::WriteFile {
                    path: "/srv/app/b.yml".into()
                }
            ),
            Decision::Allow
        );
        assert_eq!(
            decide(
                &policy,
                &Operation::WriteFile {
                    path: "/srv/other/b.yml".into()
                }
            ),
            ASK
        );
        assert_eq!(
            decide(
                &policy,
                &Operation::WriteFile {
                    path: "/etc/hosts".into()
                }
            ),
            ASK_ONCE
        );
    }

    #[test]
    fn agent_tools() {
        let policy = Policy::new(Mode::AskByRisk, Rules::default());
        let read = Operation::AgentTool {
            kind: ToolKind::Read,
            title: "Read file".into(),
            command: None,
        };
        assert_eq!(decide(&policy, &read), Decision::Allow);
        let edit = Operation::AgentTool {
            kind: ToolKind::Edit,
            title: "Edit".into(),
            command: None,
        };
        assert_eq!(decide(&policy, &edit), ASK);
        let shell = Operation::AgentTool {
            kind: ToolKind::Execute,
            title: "Run".into(),
            command: Some("rm -rf ~".into()),
        };
        assert_eq!(decide(&policy, &shell), ASK_ONCE);
        let hidden = Operation::AgentTool {
            kind: ToolKind::Execute,
            title: "Run".into(),
            command: None,
        };
        assert_eq!(decide(&policy, &hidden), ASK_ONCE);
        let unknown: ToolKind = serde_json::from_str("\"something_new\"").unwrap();
        assert_eq!(unknown, ToolKind::Other);
    }
}
