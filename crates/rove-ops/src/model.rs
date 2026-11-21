use std::{collections::BTreeSet, fmt, str::FromStr};

use rove_vault::HostId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::OpsError;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = OpsError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| OpsError::Invalid("invalid task identifier".into()))
            }
        }
    };
}

uuid_id!(OpsTaskId);
uuid_id!(OpsSubtaskId);

/// Opaque, bounded source identity. Retries of one source event reuse the same value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpsRequestId(String);

impl OpsRequestId {
    pub fn new(value: impl Into<String>) -> Result<Self, OpsError> {
        let value = value.into();
        if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
            return Err(OpsError::Invalid(
                "invalid source request identifier".into(),
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for OpsRequestId {
    type Err = OpsError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}
impl TryFrom<String> for OpsRequestId {
    type Error = OpsError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<OpsRequestId> for String {
    fn from(value: OpsRequestId) -> Self {
        value.0
    }
}
impl fmt::Display for OpsRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Reference to a body already sealed with Vault and saved in Journal's private body store.
/// No task field stores the referenced prompt, checkpoint or report text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateRef {
    pub session: String,
    pub reference: Uuid,
}

impl PrivateRef {
    pub fn new(session: impl Into<String>, reference: impl AsRef<str>) -> Result<Self, OpsError> {
        let value = Self {
            session: session.into(),
            reference: Uuid::parse_str(reference.as_ref())
                .map_err(|_| OpsError::Invalid("invalid private body reference".into()))?,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), OpsError> {
        if self.session.is_empty()
            || self.session.len() > 256
            || self.session.chars().any(char::is_control)
        {
            return Err(OpsError::Invalid("invalid private body session".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskSource {
    User,
    Schedule { schedule: String, due: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub host_id: HostId,
    /// Non-secret identity of the resolved host, route and connection target.
    pub fingerprint: String,
    #[serde(default)]
    pub input: Option<PrivateRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskScope {
    pub targets: Vec<Target>,
    pub can_modify: bool,
}

impl TaskScope {
    pub(crate) fn same_authority(&self, other: &Self) -> bool {
        self.can_modify == other.can_modify
            && self.targets.len() == other.targets.len()
            && self
                .targets
                .iter()
                .map(|target| (target.host_id, &target.fingerprint))
                .collect::<std::collections::BTreeMap<_, _>>()
                == other
                    .targets
                    .iter()
                    .map(|target| (target.host_id, &target.fingerprint))
                    .collect::<std::collections::BTreeMap<_, _>>()
    }

    pub(crate) fn validate(&self) -> Result<(), OpsError> {
        if self.targets.len() > 1024 {
            return Err(OpsError::Invalid("too many task targets".into()));
        }
        let mut hosts = BTreeSet::new();
        for target in &self.targets {
            if !hosts.insert(target.host_id)
                || target.fingerprint.is_empty()
                || target.fingerprint.len() > 2048
                || target.fingerprint.chars().any(char::is_control)
            {
                return Err(OpsError::Invalid("invalid or repeated task target".into()));
            }
            if let Some(input) = &target.input {
                input.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpsTaskState {
    Queued,
    Running,
    WaitingApproval,
    Blocked,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    BudgetExhausted,
    NeedsVerification,
    TargetChanged,
    UnresolvedEffect,
    WorkerInterrupted,
    UserDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDraft {
    pub request_id: OpsRequestId,
    pub conversation: String,
    pub source: TaskSource,
    pub input: PrivateRef,
    pub scope: TaskScope,
    pub acceptance: Option<PrivateRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpsSubtask {
    pub id: OpsSubtaskId,
    pub host_id: HostId,
    pub fingerprint: String,
    pub state: OpsTaskState,
    pub session: Option<String>,
    pub checkpoint: Option<PrivateRef>,
    pub inputs: Vec<PrivateRef>,
    pub input_version: usize,
    pub evidence: Vec<PrivateRef>,
    pub reason: Option<BlockReason>,
    pub error_code: Option<String>,
    pub uncertain_effect: bool,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpsTask {
    pub id: OpsTaskId,
    pub revision: u64,
    pub request_id: OpsRequestId,
    pub conversation: String,
    pub source: TaskSource,
    pub state: OpsTaskState,
    #[serde(default)]
    pub deleted: bool,
    pub scope: TaskScope,
    pub inputs: Vec<PrivateRef>,
    pub acceptance: Option<PrivateRef>,
    pub report: Option<PrivateRef>,
    pub subtasks: Vec<OpsSubtask>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl OpsTask {
    pub(crate) fn validate(&self) -> Result<(), OpsError> {
        self.scope.validate()?;
        if self.conversation.is_empty()
            || self.conversation.len() > 256
            || self.conversation.chars().any(char::is_control)
        {
            return Err(OpsError::Invalid(
                "invalid task conversation identifier".into(),
            ));
        }
        if let TaskSource::Schedule { schedule, due } = &self.source
            && (schedule.is_empty()
                || schedule.len() > 256
                || schedule.chars().any(char::is_control)
                || *due < 0)
        {
            return Err(OpsError::Invalid("invalid schedule source identity".into()));
        }
        if self.inputs.is_empty() {
            return Err(OpsError::Invalid(
                "a task needs a private input reference".into(),
            ));
        }
        for input in &self.inputs {
            input.validate()?;
        }
        if let Some(reference) = &self.acceptance {
            reference.validate()?;
        }
        if let Some(reference) = &self.report {
            reference.validate()?;
        }
        let mut ids = BTreeSet::new();
        let mut hosts = BTreeSet::new();
        for sub in &self.subtasks {
            if !ids.insert(sub.id)
                || !hosts.insert(sub.host_id)
                || !self.scope.targets.iter().any(|target| {
                    target.host_id == sub.host_id && target.fingerprint == sub.fingerprint
                })
            {
                return Err(OpsError::Forbidden);
            }
            if sub.state == OpsTaskState::Completed
                && (sub.evidence.is_empty() || sub.uncertain_effect)
            {
                return Err(OpsError::Invalid(
                    "completed work needs verified evidence".into(),
                ));
            }
            if let Some(reference) = &sub.checkpoint {
                reference.validate()?;
            }
            for reference in sub.inputs.iter().chain(sub.evidence.iter()) {
                reference.validate()?;
            }
        }
        if self.state == OpsTaskState::Completed
            && (self.subtasks.is_empty()
                || self.subtasks.len() != self.scope.targets.len()
                || self
                    .subtasks
                    .iter()
                    .any(|sub| sub.state != OpsTaskState::Completed))
        {
            return Err(OpsError::Invalid(
                "a task is complete only when every subtask is verified".into(),
            ));
        }
        Ok(())
    }

    pub fn subtask(&self, id: OpsSubtaskId) -> Result<&OpsSubtask, OpsError> {
        self.subtasks
            .iter()
            .find(|sub| sub.id == id)
            .ok_or(OpsError::SubtaskNotFound(id))
    }

    pub(crate) fn subtask_mut(&mut self, id: OpsSubtaskId) -> Result<&mut OpsSubtask, OpsError> {
        self.subtasks
            .iter_mut()
            .find(|sub| sub.id == id)
            .ok_or(OpsError::SubtaskNotFound(id))
    }

    pub(crate) fn aggregate(&mut self) {
        use OpsTaskState::*;
        self.state = if self.subtasks.is_empty() {
            Queued
        } else if self.subtasks.len() == self.scope.targets.len()
            && self.subtasks.iter().all(|sub| sub.state == Completed)
        {
            Completed
        } else if self.subtasks.iter().any(|sub| sub.state == Running) {
            Running
        } else if self.subtasks.iter().any(|sub| sub.state == WaitingApproval) {
            WaitingApproval
        } else if self.subtasks.len() < self.scope.targets.len()
            || self.subtasks.iter().any(|sub| sub.state == Queued)
        {
            Queued
        } else if self.subtasks.iter().any(|sub| sub.state == Blocked) {
            Blocked
        } else if self.subtasks.iter().any(|sub| sub.state == Interrupted) {
            Interrupted
        } else if self.subtasks.iter().any(|sub| sub.state == Failed) {
            Failed
        } else {
            Cancelled
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOutcome {
    Completed { evidence: PrivateRef },
    Blocked { reason: BlockReason },
    Failed { code: String },
    MaxRequests,
    Cancelled,
    Interrupted,
}

impl WorkerOutcome {
    pub fn from_stop_reason(reason: &str, evidence: Option<PrivateRef>) -> Self {
        match reason {
            "end_turn" => evidence
                .map(|evidence| Self::Completed { evidence })
                .unwrap_or(Self::Blocked {
                    reason: BlockReason::NeedsVerification,
                }),
            "max_turn_requests" => Self::MaxRequests,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed {
                code: "worker.failed".into(),
            },
        }
    }
}
