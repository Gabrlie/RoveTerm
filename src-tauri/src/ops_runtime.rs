//! Host-bound model workers for the durable operations service.

use std::fmt::Write;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use rove_ai::assistant::{Assistant, AssistantCheckpoint, CheckpointSink};
use rove_ai::llm::Message;
use rove_ai::session::{AiEvent, Outcome};
use rove_ai::{Mode, Risk};
use rove_journal::NewRecord;
use rove_ops::{
    BlockReason, OpsRequestId, OpsSubtaskId, OpsTask, OpsTaskId, OpsTaskState, PrivateRef, Target,
    TaskDraft, TaskScope, TaskSource, WorkerOutcome,
};
use rove_vault::{HostId, ProviderPrefs};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::{commands::ai, connect::host_target, error::CommandError, state::AppState};

const MAX_TASK_TEXT: usize = 100_000;
const MAX_DISPATCH: usize = 16;

pub struct WorkerSpec {
    pub task_id: OpsTaskId,
    pub subtask_id: OpsSubtaskId,
    pub display_task: String,
    pub session: String,
    pub checkpoint: Option<AssistantCheckpoint>,
    pub persist: CheckpointSink,
    pub can_modify: bool,
    pub on_event: Arc<dyn Fn(&AiEvent) + Send + Sync>,
}

#[derive(Clone)]
pub struct OpsWorker {
    pub id: String,
    pub assistant: Arc<Assistant>,
}

struct WorkerEnd {
    app: AppHandle,
    id: Option<String>,
}
impl WorkerEnd {
    async fn close(&mut self, subtask: Option<&rove_ops::OpsSubtask>) {
        if let Some(id) = &self.id {
            let state = self.app.state::<AppState>();
            let status = match subtask.map(|sub| &sub.state) {
                Some(OpsTaskState::Completed) => "completed",
                Some(OpsTaskState::Failed) => "failed",
                Some(OpsTaskState::Blocked) => "blocked",
                Some(OpsTaskState::Cancelled) => "cancelled",
                _ => "interrupted",
            };
            let details = subtask
                .and_then(|sub| sub.evidence.last())
                .and_then(|reference| read_private::<Value>(&state, reference).ok())
                .map(|body| HostReport::from_body(&body));
            let summary = details.as_ref().and_then(|report| {
                report.summary.clone().or_else(|| {
                    report
                        .error
                        .as_deref()
                        .and_then(|error| first_line(error, 100))
                })
            });
            ai::end_subagent(&self.app, &state, id, Some(status), summary.as_deref()).await;
            self.id = None;
        }
    }
}
impl Drop for WorkerEnd {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                ai::end_subagent(&app, &state, &id, Some("interrupted"), None).await;
            });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchInput {
    pub host_id: HostId,
    pub task: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
}

pub struct DispatchRequest {
    pub title: Option<String>,
    pub provider: ProviderPrefs,
    pub lead_mode: Mode,
    pub unattended: Option<bool>,
    pub allow_changes: bool,
    pub request_id: OpsRequestId,
    pub source: TaskSource,
    pub tasks: Vec<DispatchInput>,
}

#[derive(Serialize, Deserialize)]
struct DispatchBundle {
    version: u32,
    provider: ProviderPrefs,
    mode: Mode,
    unattended: Option<bool>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Followup {
    text: String,
    host_ids: Vec<HostId>,
    confirmation: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedCheckpoint {
    version: u32,
    checkpoint: AssistantCheckpoint,
    input_count: usize,
}

#[derive(Default)]
struct EvidenceCounts {
    successful: AtomicU64,
    failed: AtomicU64,
    denied: AtomicU64,
    storage_failed: AtomicBool,
}

fn store_error() -> CommandError {
    CommandError::new(
        "ops.store",
        "operations data could not be persisted or read",
    )
}

fn private_error() -> CommandError {
    CommandError::new("ops.private", "private operations data could not be read")
}

pub fn error(error: rove_ops::OpsError) -> CommandError {
    let code = match error {
        rove_ops::OpsError::Invalid(_) | rove_ops::OpsError::RequestConflict => "ops.invalid",
        rove_ops::OpsError::Journal(_) | rove_ops::OpsError::Conflict => "ops.store",
        _ => "ops.failed",
    };
    CommandError::new(code, error.to_string())
}

pub fn save_private(
    state: &AppState,
    session: &str,
    body: &Value,
) -> Result<PrivateRef, CommandError> {
    let result = seal_private(state.vault.as_ref(), state.journal.as_ref(), session, body);
    if result.is_err() {
        state.ops.halt_on_storage_failure();
    }
    result
}

fn seal_private(
    vault: &rove_vault::Vault,
    journal: &dyn rove_journal::JournalStore,
    session: &str,
    body: &Value,
) -> Result<PrivateRef, CommandError> {
    let reference = uuid::Uuid::now_v7();
    let sealed = vault
        .seal_audit(session, reference, body)
        .map_err(|_| store_error())?;
    journal
        .append_private(
            NewRecord {
                kind: "ops/private".into(),
                session_id: Some(session.to_owned()),
                payload: json!({"version":1,"reference":reference}),
            },
            session,
            &reference.to_string(),
            &sealed,
        )
        .map_err(|_| store_error())?;
    PrivateRef::new(session, reference.to_string()).map_err(error)
}

pub fn read_private<T: DeserializeOwned>(
    state: &AppState,
    reference: &PrivateRef,
) -> Result<T, CommandError> {
    open_private(state.vault.as_ref(), state.journal.as_ref(), reference)
}

pub(crate) fn host_name(state: &AppState, id: HostId) -> String {
    state
        .vault
        .host(id)
        .map_or_else(|_| id.to_string(), |host| host.name)
}

#[derive(Serialize, Default)]
pub struct OpsTaskCounts {
    total: usize,
    queued: usize,
    running: usize,
    waiting_approval: usize,
    completed: usize,
    blocked: usize,
    failed: usize,
    cancelled: usize,
    interrupted: usize,
}

impl OpsTaskCounts {
    fn add(&mut self, state: &OpsTaskState) {
        self.total += 1;
        *match state {
            OpsTaskState::Queued => &mut self.queued,
            OpsTaskState::Running => &mut self.running,
            OpsTaskState::WaitingApproval => &mut self.waiting_approval,
            OpsTaskState::Completed => &mut self.completed,
            OpsTaskState::Blocked => &mut self.blocked,
            OpsTaskState::Failed => &mut self.failed,
            OpsTaskState::Cancelled => &mut self.cancelled,
            OpsTaskState::Interrupted => &mut self.interrupted,
        } += 1;
    }
}

#[derive(Serialize)]
pub struct OpsCheckView {
    criterion: String,
    summary: Option<String>,
}

#[derive(Serialize)]
pub struct OpsSubtaskView {
    id: Option<OpsSubtaskId>,
    host_id: HostId,
    host_name: String,
    state: OpsTaskState,
    session: Option<String>,
    task: String,
    summary: Option<String>,
    reply: Option<String>,
    checks: Vec<OpsCheckView>,
    reason: Option<BlockReason>,
    error: Option<String>,
    uncertain_effect: bool,
    attempts: u32,
}

#[derive(Serialize)]
pub struct OpsTaskView {
    id: OpsTaskId,
    revision: u64,
    conversation: String,
    title: String,
    source: TaskSource,
    state: OpsTaskState,
    read_only: bool,
    created_at: i64,
    updated_at: i64,
    counts: OpsTaskCounts,
    subtasks: Vec<OpsSubtaskView>,
}

fn nonempty(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some(_) => text.chars().take(max - 1).chain(['…']).collect(),
        None => text.to_owned(),
    }
}

fn first_line(text: &str, max: usize) -> Option<String> {
    text.lines()
        .find_map(|line| {
            nonempty(line.trim_start_matches(|ch: char| {
                ch.is_whitespace() || matches!(ch, '#' | '-' | '*' | '>')
            }))
        })
        .map(|line| truncate(line, max))
}

fn legacy_checks(body: &Value) -> Vec<OpsCheckView> {
    let verification = &body["verification"];
    verification["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, criterion)| {
            let check = verification["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|check| check["criterion_index"].as_u64() == Some(index as u64));
            Some(OpsCheckView {
                criterion: criterion.as_str()?.to_owned(),
                summary: check
                    .and_then(|check| check["summary"].as_str())
                    .and_then(nonempty)
                    .map(str::to_owned),
            })
        })
        .collect()
}

#[derive(Default)]
struct HostReport {
    summary: Option<String>,
    reply: Option<String>,
    checks: Vec<OpsCheckView>,
    error: Option<String>,
}

impl HostReport {
    fn from_body(body: &Value) -> Self {
        match body["kind"].as_str() {
            Some("host_report") => {
                let reply = body["reply"].as_str().and_then(nonempty);
                let error = body["error"]
                    .as_str()
                    .and_then(nonempty)
                    .map(str::to_owned);
                let checks = legacy_checks(body);
                let summary = reply.and_then(|reply| first_line(reply, 100)).or_else(|| {
                    checks.iter().find_map(|check| {
                        check
                            .summary
                            .as_deref()
                            .and_then(|text| first_line(text, 100))
                    })
                });
                Self {
                    summary,
                    reply: reply.map(str::to_owned),
                    checks,
                    error,
                }
            }
            Some("worker_failure") => Self {
                error: body["message"]
                    .as_str()
                    .and_then(nonempty)
                    .map(str::to_owned),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

pub(crate) fn task_view(state: &AppState, task: &OpsTask) -> OpsTaskView {
    let mut counts = OpsTaskCounts::default();
    let subtasks = task
        .scope
        .targets
        .iter()
        .map(|target| {
            let sub = task
                .subtasks
                .iter()
                .find(|sub| sub.host_id == target.host_id);
            let status = sub.map_or_else(|| task.state.clone(), |sub| sub.state.clone());
            counts.add(&status);
            let input = target
                .input
                .iter()
                .chain(sub.and_then(|sub| sub.inputs.first()))
                .find_map(|reference| read_private::<DispatchInput>(state, reference).ok());
            let report = sub
                .and_then(|sub| sub.evidence.last())
                .and_then(|reference| read_private::<Value>(state, reference).ok())
                .map_or_else(HostReport::default, |body| HostReport::from_body(&body));
            OpsSubtaskView {
                id: sub.map(|sub| sub.id),
                host_id: target.host_id,
                host_name: host_name(state, target.host_id),
                state: status,
                session: sub.and_then(|sub| sub.session.clone()),
                task: input.map_or_else(String::new, |input| input.task),
                summary: report.summary,
                reply: report.reply,
                checks: report.checks,
                reason: sub.and_then(|sub| sub.reason.clone()),
                error: report.error,
                uncertain_effect: sub.is_some_and(|sub| {
                    sub.uncertain_effect || sub.reason == Some(BlockReason::UnresolvedEffect)
                }),
                attempts: sub.map_or(0, |sub| sub.attempts),
            }
        })
        .collect::<Vec<_>>();
    let title = task
        .inputs
        .first()
        .and_then(|reference| read_private::<DispatchBundle>(state, reference).ok())
        .and_then(|bundle| bundle.title)
        .and_then(|title| nonempty(&title).map(str::to_owned))
        .or_else(|| {
            subtasks
                .first()?
                .task
                .lines()
                .find_map(nonempty)
                .map(|line| truncate(line, 60))
        })
        .unwrap_or_else(|| {
            let mut title = subtasks
                .iter()
                .take(2)
                .map(|sub| sub.host_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if subtasks.len() > 2 {
                let _ = write!(title, " +{}", subtasks.len() - 2);
            }
            title
        });
    OpsTaskView {
        id: task.id,
        revision: task.revision,
        conversation: task.conversation.clone(),
        title,
        source: task.source.clone(),
        state: task.state.clone(),
        read_only: !task.scope.can_modify,
        created_at: task.created_at,
        updated_at: task.updated_at,
        counts,
        subtasks,
    }
}

pub(crate) fn active_state(state: &OpsTaskState) -> bool {
    matches!(
        state,
        OpsTaskState::Queued | OpsTaskState::Running | OpsTaskState::WaitingApproval
    )
}

fn settled(task: &OpsTask) -> bool {
    task.subtasks.len() == task.scope.targets.len()
        && !task.subtasks.iter().any(|sub| active_state(&sub.state))
}

pub(crate) fn can_wake_lead(task: &OpsTask) -> bool {
    task.source == TaskSource::User
        && settled(task)
        && task
            .subtasks
            .iter()
            .any(|sub| sub.state != OpsTaskState::Cancelled)
}

pub(crate) fn task_status_summary(state: &AppState, task: &OpsTask) -> Value {
    build_task_summary(task, None, |id| host_name(state, id), None)
}

type EvidenceReader<'a> = dyn Fn(&PrivateRef) -> Result<Value, CommandError> + 'a;

pub(crate) fn task_report_summary(
    state: &AppState,
    task: &OpsTask,
    host_filter: Option<HostId>,
) -> Result<Value, CommandError> {
    if let Some(host) = host_filter
        && !task
            .scope
            .targets
            .iter()
            .any(|target| target.host_id == host)
    {
        return Err(CommandError::new(
            "input.invalid",
            "host is outside the task scope",
        ));
    }
    Ok(build_task_summary(
        task,
        host_filter,
        |id| host_name(state, id),
        Some(&|reference| read_private::<Value>(state, reference)),
    ))
}

fn build_task_summary(
    task: &OpsTask,
    host_filter: Option<HostId>,
    host_name: impl Fn(HostId) -> String,
    read_evidence: Option<&EvidenceReader<'_>>,
) -> Value {
    let mut counts = json!({
        "total":task.scope.targets.len(), "queued":0, "running":0, "waiting_approval":0,
        "completed":0, "blocked":0, "failed":0, "cancelled":0, "interrupted":0,
    });
    for sub in &task.subtasks {
        let state = json!(sub.state);
        if let Some(key) = state.as_str() {
            counts[key] = json!(counts[key].as_u64().unwrap_or_default() + 1);
        }
    }
    let subtasks = task.scope.targets.iter()
        .filter(|target| host_filter.is_none_or(|id| id == target.host_id))
        .map(|target| {
            let name = host_name(target.host_id);
            let Some(sub) = task.subtasks.iter().find(|sub| sub.host_id == target.host_id) else {
                let mut summary = json!({"host_id":target.host_id, "host_name":name});
                if read_evidence.is_some() {
                    summary["report_error"] = json!("No subtask was created for this host.");
                }
                return summary;
            };
            let mut summary = json!({
                "id":sub.id, "host_id":sub.host_id, "host_name":name, "state":sub.state,
                "reason":sub.reason, "error_code":sub.error_code, "uncertain_effect":sub.uncertain_effect,
                "attempts":sub.attempts, "session":sub.session,
            });
            if let Some(read) = read_evidence {
                match sub.evidence.last().map(read) {
                    Some(Ok(body)) => add_host_report(&mut summary, &body),
                    Some(Err(_)) => summary["report_error"] = json!("Could not read encrypted host evidence."),
                    None => summary["report_error"] = json!("No host evidence was recorded."),
                }
            }
            summary
        }).collect::<Vec<_>>();
    json!({
        "id":task.id, "revision":task.revision, "request_id":task.request_id,
        "conversation":task.conversation, "source":task.source, "state":task.state,
        "created_at":task.created_at, "updated_at":task.updated_at, "can_modify":task.scope.can_modify,
        "counts":counts, "settled":settled(task), "subtasks":subtasks,
    })
}

const MAX_REPORT_REPLY: usize = 8 * 1024;

fn add_host_report(summary: &mut Value, body: &Value) {
    match body["kind"].as_str() {
        Some("host_report") => {
            let reply = body["reply"].as_str().unwrap_or_default();
            let end = reply.floor_char_boundary(MAX_REPORT_REPLY);
            summary["report"] = json!({
                "kind":"host_report", "stop_reason":body["stop_reason"],
                "checks":legacy_checks(body),
                "successful_operations":body["successful_operations"],
                "failed_operations":body["failed_operations"], "denied_operations":body["denied_operations"],
                "reply":&reply[..end], "reply_clipped":end < reply.len(),
            });
            if let Some(error) = body["error"].as_str().and_then(nonempty) {
                summary["report"]["error"] = json!(error);
            }
            if let Some(verified) = body["verification"]["verified"].as_bool() {
                summary["report"]["verified"] = json!(verified);
            }
        }
        Some("worker_failure") => {
            summary["report"] = json!({
                "kind":"worker_failure", "failure_code":body["code"], "failure_message":body["message"],
            })
        }
        _ => summary["report_error"] = json!("The latest evidence does not contain a host report."),
    }
}

/// Renders report fields as Markdown text, escaping formatting in untrusted evidence.
#[cfg(any(desktop, test))]
pub(crate) fn format_report_fallback(summaries: &[Value]) -> String {
    fn plain(value: &str) -> String {
        let text = value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(char::is_control, "")
            .replace('<', "\\<")
            .replace('>', "\\>");
        if text.starts_with(['#', '-', '*', '+']) {
            format!("\\{text}")
        } else {
            text
        }
    }

    let mut markdown = String::from("# Operations report\n");
    let total: u64 = summaries
        .iter()
        .filter_map(|task| task["counts"]["total"].as_u64())
        .sum();
    let completed: u64 = summaries
        .iter()
        .filter_map(|task| task["counts"]["completed"].as_u64())
        .sum();
    let _ = writeln!(markdown, "\n{completed} of {total} host tasks completed.");
    for task in summaries {
        let id = task["id"].as_str().unwrap_or_default();
        let _ = writeln!(markdown, "\n## Task `{id}`");
        for sub in task["subtasks"].as_array().into_iter().flatten() {
            let name = plain(sub["host_name"].as_str().unwrap_or_default());
            let state = sub["state"]
                .as_str()
                .unwrap_or("not started")
                .replace('_', " ");
            let _ = writeln!(markdown, "\n### {name} — {state}\n");
            for (key, label) in [
                ("reason", "Reason"),
                ("error_code", "Error code"),
                ("report_error", "Evidence"),
            ] {
                if let Some(text) = sub[key].as_str() {
                    let _ = writeln!(markdown, "- {label} — {}", plain(text));
                }
            }
            if sub["uncertain_effect"] == true {
                markdown.push_str("- Confirm the remote outcome before continuing.\n");
            }
            let report = &sub["report"];
            if report["kind"] == "host_report" {
                if let Some(verified) = report["verified"].as_bool() {
                    let verification = if verified { "Verified" } else { "Unverified" };
                    let _ = writeln!(markdown, "- {verification}");
                }
                for check in report["checks"].as_array().into_iter().flatten() {
                    let criterion = plain(check["criterion"].as_str().unwrap_or_default());
                    let result = plain(check["summary"].as_str().unwrap_or("No result submitted"));
                    let _ = writeln!(markdown, "- {criterion} — {result}");
                }
                if let Some(reply) = report["reply"]
                    .as_str()
                    .filter(|reply| !reply.trim().is_empty())
                {
                    let suffix = if report["reply_clipped"] == true {
                        " …"
                    } else {
                        ""
                    };
                    let _ = writeln!(markdown, "- Worker findings — {}{suffix}", plain(reply));
                }
            } else if let Some(message) = report["failure_message"].as_str() {
                let _ = writeln!(markdown, "- Worker failure — {}", plain(message));
            }
        }
    }
    markdown
}

fn open_private<T: DeserializeOwned>(
    vault: &rove_vault::Vault,
    journal: &dyn rove_journal::JournalStore,
    reference: &PrivateRef,
) -> Result<T, CommandError> {
    let sealed = journal
        .private_body(&reference.session, &reference.reference.to_string())
        .map_err(|_| private_error())?;
    let value = vault
        .open_audit(&reference.session, reference.reference, &sealed)
        .map_err(|_| private_error())?;
    serde_json::from_value(value).map_err(|_| private_error())
}

fn fingerprint(state: &AppState, id: HostId) -> Result<String, CommandError> {
    let host = state.vault.host(id)?;
    let target = host_target(state, &host)?;
    target_fingerprint(&host, &target)
}

fn target_fingerprint(
    host: &rove_vault::Host,
    target: &rove_ssh::SshTarget,
) -> Result<String, CommandError> {
    serde_json::to_string(
        &json!({"host_id":host.id,"hostname":target.hostname.to_ascii_lowercase(),
        "username":target.username,"port":target.port,"route":target.route,"auth":host.auth}),
    )
    .map_err(|_| store_error())
}

pub(crate) fn emit(app: &AppHandle, task: &OpsTask) {
    if let Err(error) = app.emit("ops-update", &task_view(&app.state::<AppState>(), task)) {
        tracing::debug!(%error, "operations update unavailable");
    }
}

fn task_scope(targets: Vec<Target>, allow_changes: bool, unattended: Option<bool>) -> TaskScope {
    TaskScope {
        targets,
        can_modify: allow_changes && unattended != Some(false),
    }
}

pub async fn dispatch(
    app: &AppHandle,
    parent: &str,
    request: DispatchRequest,
) -> Result<OpsTask, CommandError> {
    let DispatchRequest {
        title,
        provider,
        lead_mode,
        unattended,
        allow_changes,
        request_id,
        source,
        tasks,
    } = request;
    let state = app.state::<AppState>();
    state.ensure_unlocked().await?;
    if tasks.is_empty() || tasks.len() > MAX_DISPATCH {
        return Err(CommandError::new(
            "input.invalid",
            "expected 1..=16 host tasks",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for task in &tasks {
        if !seen.insert(task.host_id)
            || task.task.trim().is_empty()
            || task.task.len() > MAX_TASK_TEXT
            || task.acceptance.len() > 32
            || task
                .acceptance
                .iter()
                .any(|item| item.trim().is_empty() || item.len() > 4096)
        {
            return Err(CommandError::new(
                "input.invalid",
                "host tasks need unique hosts, bounded text and acceptance checks",
            ));
        }
    }
    let mut targets = Vec::new();
    for task in &tasks {
        targets.push(Target {
            host_id: task.host_id,
            fingerprint: fingerprint(&state, task.host_id)?,
            input: Some(save_private(
                &state,
                parent,
                &serde_json::to_value(task).map_err(|_| store_error())?,
            )?),
        });
    }
    let input = save_private(
        &state,
        parent,
        &serde_json::to_value(DispatchBundle {
            version: 1,
            provider,
            mode: lead_mode,
            unattended,
            title,
        })
        .map_err(|_| store_error())?,
    )?;
    let acceptance = save_private(
        &state,
        parent,
        &json!({"hosts":tasks.iter().map(|task| json!({"host_id":task.host_id,"checks":task.acceptance})).collect::<Vec<_>>() }),
    )?;
    let task = state
        .ops
        .create(TaskDraft {
            request_id,
            conversation: parent.to_owned(),
            source,
            input,
            scope: task_scope(targets, allow_changes, unattended),
            acceptance: Some(acceptance),
        })
        .map_err(error)?;
    // Existing requests are not re-dispatched: ensure_subtask retains terminal worker states.
    for target in &task.scope.targets {
        state
            .ops
            .ensure_subtask(task.id, target.host_id, &target.fingerprint)
            .map_err(error)?;
    }
    let task = state.ops.get(task.id).map_err(error)?;
    emit(app, &task);
    kick(app, task.id);
    Ok(task)
}

/// Starts queued workers only. Interrupted, blocked and pending-approval tasks require explicit input.
pub fn kick(app: &AppHandle, id: OpsTaskId) {
    let ops = app.state::<AppState>().ops.clone();
    let claim = match ops.claim_task(id) {
        Ok(claim) => claim,
        Err(_) => return,
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut running = tokio::task::JoinSet::new();
        let ready = ops.get(id).and_then(|task| {
            if task.state != OpsTaskState::Cancelled {
                for target in &task.scope.targets {
                    ops.ensure_subtask(id, target.host_id, &target.fingerprint)?;
                }
            }
            ops.get(id)
        });
        if let Ok(task) = ready
            && task.state != OpsTaskState::Cancelled
        {
            for sub in task
                .subtasks
                .iter()
                .filter(|sub| sub.state == OpsTaskState::Queued)
            {
                running.spawn(run_worker(app.clone(), id, sub.id));
            }
        }
        while let Some(result) = running.join_next().await {
            if let Err(error) = result {
                tracing::error!(%error, "operations worker stopped unexpectedly");
            }
        }
        if let Err(error) = publish_report(&app, id) {
            tracing::warn!(code = error.code, "operations router report unavailable");
        }
        drop(claim);
        // Input can arrive as a router finishes; re-reading after releasing its claim closes that race.
        let Ok(task) = ops.get(id) else {
            return;
        };
        if task.state != OpsTaskState::Cancelled
            && task
                .subtasks
                .iter()
                .any(|sub| sub.state == OpsTaskState::Queued)
        {
            kick(&app, id);
        } else if can_wake_lead(&task) {
            ai::notify_ops_task_settled(&app, &task.conversation, task.id, task.revision).await;
        }
    });
}

fn pending_modifies(checkpoint: &AssistantCheckpoint) -> bool {
    let Some(id) = &checkpoint.pending_tool else {
        return false;
    };
    let call = checkpoint.messages.iter().rev().find_map(|message| {
        if let Message::Assistant { tool_calls, .. } = message {
            tool_calls.iter().find(|call| &call.id == id)
        } else {
            None
        }
    });
    let Some(call) = call else {
        return true;
    };
    match call.name.as_str() {
        "read_file" | "list_dir" | "read_notes" | "terminal_snapshot" | "read_terminal" => false,
        "run_command" => serde_json::from_str::<Value>(&call.arguments)
            .ok()
            .and_then(|args| {
                args.get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_none_or(|command| {
                rove_ai::risk::assess_command(&command, &rove_ai::Rules::default()).risk
                    != Risk::ReadOnly
            }),
        _ => true,
    }
}

fn observe(
    app: AppHandle,
    id: OpsTaskId,
    sub: OpsSubtaskId,
    counts: Arc<EvidenceCounts>,
) -> Arc<dyn Fn(&AiEvent) + Send + Sync> {
    Arc::new(move |event| {
        let state = app.state::<AppState>();
        let transition = match event {
            AiEvent::Approval { .. } => Some(true),
            AiEvent::Approved { .. } => Some(false),
            _ => None,
        };
        if let Some(waiting) = transition {
            if let Err(error) = state.ops.set_waiting_approval(id, sub, waiting)
                && matches!(error, rove_ops::OpsError::Journal(_))
            {
                counts.storage_failed.store(true, Ordering::Release);
            }
            if let Ok(task) = state.ops.get(id) {
                emit(&app, &task);
            }
        }
        if let AiEvent::Executed { outcome, .. } = event {
            match outcome {
                Outcome::Denied => {
                    counts.denied.fetch_add(1, Ordering::Relaxed);
                }
                Outcome::Failed { .. } => {
                    counts.failed.fetch_add(1, Ordering::Relaxed);
                }
                Outcome::Command { output } if output.exit_code != Some(0) => {
                    counts.failed.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    counts.successful.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    })
}

async fn run_worker(app: AppHandle, id: OpsTaskId, sub_id: OpsSubtaskId) {
    let ops = app.state::<AppState>().ops.clone();
    let lease = match ops.lease_worker(id, sub_id).await {
        Ok(lease) => lease,
        Err(_) => return,
    };
    if let Ok(task) = ops.get(id) {
        emit(&app, &task);
    }
    let mut end = WorkerEnd {
        app: app.clone(),
        id: None,
    };
    let result = execute_worker(&app, id, sub_id, lease.cancellation.clone(), &mut end).await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Ok(task) = ops.get(id) {
                let state = app.state::<AppState>();
                if let Ok(reference) = save_private(
                    &state,
                    &task.conversation,
                    &json!({"kind":"worker_failure","code":error.code,"message":error.message}),
                ) && let Err(error) = ops.record_evidence(id, sub_id, reference)
                {
                    tracing::warn!(%error, "operations failure evidence could not be linked");
                }
            }
            let code = match error.code {
                "host.not_found" => "host.not_found",
                "ops.target_changed" => "host.target_changed",
                _ => "worker.failed",
            };
            if code == "host.target_changed" || code == "host.not_found" {
                WorkerOutcome::Blocked {
                    reason: BlockReason::TargetChanged,
                }
            } else {
                WorkerOutcome::Failed { code: code.into() }
            }
        }
    };
    let finished = match ops.finish_subtask(id, sub_id, outcome) {
        Ok(task) => {
            emit(&app, &task);
            Some(task)
        }
        Err(error) => {
            tracing::warn!(%error, "operations worker outcome was not recorded");
            None
        }
    };
    end.close(finished.as_ref().and_then(|task| task.subtask(sub_id).ok()))
        .await;
    drop(lease);
    if let Err(error) = publish_report(&app, id) {
        tracing::warn!(code = error.code, "operations report could not be stored");
    }
    if let Ok(task) = ops.get(id)
        && let Ok(sub) = task.subtask(sub_id)
        && sub.input_version < task.inputs.len()
        && !sub.uncertain_effect
        && !matches!(
            sub.state,
            OpsTaskState::Cancelled | OpsTaskState::WaitingApproval
        )
    {
        let state = app.state::<AppState>();
        if task.inputs.iter().skip(sub.input_version).any(|reference| {
            read_private::<Followup>(&state, reference)
                .is_ok_and(|input| input.host_ids.contains(&sub.host_id))
        }) && let Ok(task) = ops.requeue_subtask(id, sub_id)
        {
            emit(&app, &task);
        }
    }
}

async fn execute_worker(
    app: &AppHandle,
    id: OpsTaskId,
    sub_id: OpsSubtaskId,
    cancellation: tokio_util::sync::CancellationToken,
    end: &mut WorkerEnd,
) -> Result<WorkerOutcome, CommandError> {
    let state = app.state::<AppState>();
    let task = state.ops.get(id).map_err(error)?;
    let sub = task.subtask(sub_id).map_err(error)?.clone();
    if fingerprint(&state, sub.host_id)? != sub.fingerprint {
        return Err(CommandError::new(
            "ops.target_changed",
            "the saved host target changed",
        ));
    }
    let bundle: DispatchBundle = read_private(&state, &task.inputs[0])?;
    if bundle.version != 1 {
        return Err(store_error());
    }
    let initial: DispatchInput = read_private(&state, &sub.inputs[0])?;
    let saved = sub
        .checkpoint
        .as_ref()
        .map(|reference| read_private::<SavedCheckpoint>(&state, reference))
        .transpose()?;
    if saved
        .as_ref()
        .is_some_and(|saved| saved.version != 1 || saved.input_count > sub.inputs.len())
    {
        return Err(store_error());
    }
    let consumed = saved.as_ref().map(|saved| saved.input_count).unwrap_or(1);
    let mut checkpoint = saved.map(|saved| saved.checkpoint);
    if sub.uncertain_effect || checkpoint.as_ref().is_some_and(pending_modifies) {
        return Ok(WorkerOutcome::Blocked {
            reason: BlockReason::UnresolvedEffect,
        });
    }
    if let Some(saved) = checkpoint.as_mut()
        && saved.pending_tool.is_some()
    {
        saved.resolve_interrupted();
    }
    let mut text = if checkpoint.is_some() {
        "Continue this operations task from its saved history. Do not repeat completed changes; verify current state before doing more.".to_owned()
    } else {
        let mut text = initial.task.clone();
        if !initial.acceptance.is_empty() {
            text.push_str("\n\nPlease check and report on:");
            for item in &initial.acceptance {
                let _ = write!(text, "\n- {item}");
            }
        }
        text
    };
    let mut followup_index = 0;
    for reference in sub.inputs.iter().skip(consumed) {
        let followup: Followup = read_private(&state, reference)?;
        if followup.host_ids.contains(&sub.host_id) {
            followup_index += 1;
            text.push_str(&format!(
                "\n\nUser follow-up {followup_index}:\n{}",
                followup.text
            ));
        }
    }
    let session = sub
        .session
        .clone()
        .unwrap_or_else(|| format!("ai-{}", uuid::Uuid::now_v7()));
    state
        .ops
        .update_subtask_session(id, sub_id, session.clone())
        .map_err(error)?;
    let ops = state.ops.clone();
    let sink_app = app.clone();
    let sink_session = session.clone();
    let can_modify = task.scope.can_modify;
    let input_count = sub.inputs.len();
    let persist: CheckpointSink = Arc::new(move |checkpoint| {
        let state = sink_app.state::<AppState>();
        let reference = save_private(
            &state,
            &sink_session,
            &serde_json::to_value(SavedCheckpoint {
                version: 1,
                checkpoint: checkpoint.clone(),
                input_count,
            })
            .map_err(|_| "ops.checkpoint".to_owned())?,
        )
        .map_err(|_| "ops.storage".to_owned())?;
        let current = ops.get(id).map_err(|_| "ops.storage".to_owned())?;
        let sub = current
            .subtask(sub_id)
            .map_err(|_| "ops.storage".to_owned())?;
        if can_modify && pending_modifies(&checkpoint) && !sub.uncertain_effect {
            ops.mark_effect_started(id, sub_id)
                .map_err(|_| "ops.storage".to_owned())?;
        }
        ops.update_checkpoint(id, sub_id, reference.clone())
            .map_err(|_| "ops.storage".to_owned())?;
        if checkpoint.pending_tool.is_none() && sub.uncertain_effect {
            ops.resolve_effect(id, sub_id, reference)
                .map_err(|_| "ops.storage".to_owned())?;
        }
        Ok(())
    });
    let counts = Arc::new(EvidenceCounts::default());
    let mode = if bundle.unattended.is_some() {
        Mode::Unattended
    } else {
        ai::ops_live_mode(app, &task.conversation)
            .await
            .unwrap_or(bundle.mode)
    };
    let worker = ai::start_ops_worker(
        app,
        &task.conversation,
        sub.host_id,
        &bundle.provider,
        mode,
        bundle.unattended,
        WorkerSpec {
            task_id: id,
            subtask_id: sub_id,
            display_task: initial.task,
            session,
            checkpoint,
            persist,
            can_modify,
            on_event: observe(app.clone(), id, sub_id, counts.clone()),
        },
    )
    .await?;
    end.id = Some(worker.id.clone());
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => { worker.assistant.cancel().await; Ok("cancelled".to_owned()) },
        result = worker.assistant.prompt(&text) => result,
    };
    let reply = worker.assistant.last_reply().await;
    let error_message = result.as_ref().err().map(ToString::to_string);
    let reason = result.as_deref().unwrap_or("failed");
    let successful = counts.successful.load(Ordering::Acquire);
    let failed = counts.failed.load(Ordering::Acquire);
    let denied = counts.denied.load(Ordering::Acquire);
    let report = save_private(
        &state,
        &task.conversation,
        &json!({
            "kind":"host_report", "host_id":sub.host_id, "worker_session":worker.id,
            "acceptance":initial.acceptance, "reply":reply, "error":error_message, "stop_reason":reason,
            "successful_operations":successful, "failed_operations":failed, "denied_operations":denied,
        }),
    )?;
    state
        .ops
        .record_evidence(id, sub_id, report.clone())
        .map_err(error)?;
    Ok(match reason {
        "end_turn" if counts.storage_failed.load(Ordering::Acquire) => WorkerOutcome::Blocked {
            reason: BlockReason::NeedsVerification,
        },
        "end_turn" if successful == 0 && denied > 0 => WorkerOutcome::Blocked {
            reason: BlockReason::UserDecision,
        },
        "end_turn" if successful == 0 && failed > 0 && reply.trim().is_empty() => {
            WorkerOutcome::Failed {
                code: "worker.failed".into(),
            }
        }
        _ => WorkerOutcome::from_stop_reason(reason, Some(report)),
    })
}

fn publish_report(app: &AppHandle, id: OpsTaskId) -> Result<(), CommandError> {
    let state = app.state::<AppState>();
    loop {
        let task = state.ops.get(id).map_err(error)?;
        if task.report.is_some() {
            return Ok(());
        }
        let reports = task.subtasks.iter().map(|sub| json!({"host_id":sub.host_id,"state":sub.state,
            "reason":sub.reason,"report":sub.evidence.last().and_then(|reference| read_private::<Value>(&state, reference).ok())})).collect::<Vec<_>>();
        let reference = save_private(
            &state,
            &task.conversation,
            &json!({"kind":"task_report","state":task.state,"hosts":reports}),
        )?;
        match state
            .ops
            .set_report_if_current(id, task.revision, reference)
        {
            Ok(task) => {
                emit(app, &task);
                return Ok(());
            }
            Err(rove_ops::OpsError::Conflict) => continue,
            Err(reason) => return Err(error(reason)),
        }
    }
}

pub async fn append_input(
    app: &AppHandle,
    id: OpsTaskId,
    text: &str,
    host_ids: Vec<HostId>,
    resolve_uncertain: bool,
) -> Result<OpsTask, CommandError> {
    let state = app.state::<AppState>();
    state.ensure_unlocked().await?;
    if text.trim().is_empty() || text.len() > MAX_TASK_TEXT || host_ids.is_empty() {
        return Err(CommandError::new(
            "input.invalid",
            "follow-up needs text and selected hosts",
        ));
    }
    let task = state.ops.get(id).map_err(error)?;
    let selected: std::collections::BTreeSet<_> = host_ids.iter().copied().collect();
    if selected.len() != host_ids.len()
        || selected.iter().any(|host| {
            !task
                .scope
                .targets
                .iter()
                .any(|target| target.host_id == *host)
        })
    {
        return Err(CommandError::new(
            "ops.scope",
            "a follow-up cannot expand the task scope",
        ));
    }
    let reference = save_private(
        &state,
        &task.conversation,
        &serde_json::to_value(Followup {
            text: text.to_owned(),
            host_ids,
            confirmation: resolve_uncertain,
        })
        .map_err(|_| store_error())?,
    )?;
    let task = state
        .ops
        .append_input(id, reference.clone())
        .map_err(error)?;
    for sub in &task.subtasks {
        if !selected.contains(&sub.host_id)
            || matches!(
                sub.state,
                OpsTaskState::Running | OpsTaskState::WaitingApproval
            )
        {
            continue;
        }
        if sub.uncertain_effect || sub.reason == Some(BlockReason::UnresolvedEffect) {
            if !resolve_uncertain {
                continue;
            }
            resolve_interrupted_checkpoint(&state, id, sub)?;
            state
                .ops
                .resolve_effect(id, sub.id, reference.clone())
                .map_err(error)?;
        }
        match state.ops.requeue_subtask(id, sub.id) {
            Ok(_) | Err(rove_ops::OpsError::Busy) => {
                // Active reservations consume persisted follow-ups after their lease ends.
            }
            Err(reason) => return Err(error(reason)),
        }
    }
    let task = state.ops.get(id).map_err(error)?;
    emit(app, &task);
    kick(app, id);
    Ok(task)
}

fn resolve_interrupted_checkpoint(
    state: &AppState,
    id: OpsTaskId,
    sub: &rove_ops::OpsSubtask,
) -> Result<(), CommandError> {
    if let Some(saved) = &sub.checkpoint {
        let mut checkpoint: SavedCheckpoint = read_private(state, saved)?;
        checkpoint.checkpoint.resolve_interrupted();
        let updated = save_private(
            state,
            &saved.session,
            &serde_json::to_value(checkpoint).map_err(|_| store_error())?,
        )?;
        state
            .ops
            .update_checkpoint(id, sub.id, updated)
            .map_err(error)?;
    }
    Ok(())
}

pub async fn retry_task(
    app: &AppHandle,
    id: OpsTaskId,
    host_id: Option<HostId>,
    confirm_uncertain: bool,
) -> Result<OpsTaskView, CommandError> {
    let state = app.state::<AppState>();
    if confirm_uncertain {
        state.ensure_unlocked().await?;
    }
    let task = state.ops.get(id).map_err(error)?;
    let host_subtask = host_id
        .map(|host_id| {
            task.subtasks
                .iter()
                .find(|sub| sub.host_id == host_id)
                .ok_or_else(|| {
                    CommandError::new("input.invalid", "host has no subtask in this task")
                })
        })
        .transpose()?;
    if confirm_uncertain && task.state != OpsTaskState::Cancelled {
        for sub in task.subtasks.iter().filter(|sub| {
            host_id.is_none_or(|host| sub.host_id == host)
                && (sub.uncertain_effect || sub.reason == Some(BlockReason::UnresolvedEffect))
        }) {
            if active_state(&sub.state) {
                if host_id.is_some() {
                    return Err(error(rove_ops::OpsError::Busy));
                }
                continue;
            }
            if sub.state == OpsTaskState::Cancelled {
                continue;
            }
            resolve_interrupted_checkpoint(&state, id, sub)?;
            let reference = save_private(
                &state,
                &task.conversation,
                &json!({"kind":"retry_confirmed","host_id":sub.host_id}),
            )?;
            state
                .ops
                .resolve_effect(id, sub.id, reference)
                .map_err(error)?;
        }
    }
    let task = match host_subtask {
        Some(sub) => state.ops.retry_subtask(id, sub.id),
        None => state.ops.retry_failed(id),
    }
    .map_err(error)?;
    emit(app, &task);
    kick(app, id);
    Ok(task_view(&state, &task))
}

pub fn reference_belongs(task: &OpsTask, reference: &PrivateRef) -> bool {
    task.inputs.contains(reference)
        || task.acceptance.as_ref() == Some(reference)
        || task.report.as_ref() == Some(reference)
        || task
            .scope
            .targets
            .iter()
            .any(|target| target.input.as_ref() == Some(reference))
        || task
            .subtasks
            .iter()
            .any(|sub| sub.evidence.contains(reference) || sub.inputs.contains(reference))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_ai::llm::{Format, ToolCall};
    use rove_journal::{JournalQuery, JournalStore, SqliteJournal};
    use rove_vault::{AuthMethod, HostDraft, HostRoute, MemoryKeyCache, Vault};

    fn checkpoint(name: &str, arguments: Value) -> AssistantCheckpoint {
        AssistantCheckpoint {
            version: 1,
            format: Format::Chat,
            model: "fixture".into(),
            messages: vec![Message::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: name.into(),
                    arguments: arguments.to_string(),
                }],
                opaque: None,
            }],
            turns: 1,
            last_used: 5,
            pending_tool: Some("call-1".into()),
        }
    }

    #[test]
    fn pending_tools_distinguish_read_only_work_from_modifications() {
        for name in [
            "read_file",
            "list_dir",
            "read_notes",
            "read_terminal",
            "terminal_snapshot",
        ] {
            assert!(!pending_modifies(&checkpoint(name, json!({}))));
        }
        assert!(!pending_modifies(&checkpoint(
            "run_command",
            json!({"command":"uname -a"})
        )));
        assert!(pending_modifies(&checkpoint(
            "run_command",
            json!({"command":"systemctl restart app"})
        )));
        assert!(pending_modifies(&checkpoint("write_file", json!({}))));
        assert!(pending_modifies(&checkpoint("unknown_tool", json!({}))));
        assert!(pending_modifies(&checkpoint("run_command", Value::Null)));
    }

    #[test]
    fn interactive_scope_allows_policy_governed_changes_and_schedules_keep_their_scope() {
        let target = Target {
            host_id: HostId::new(),
            fingerprint: "host-fixture".into(),
            input: None,
        };
        for (allow_changes, unattended, expected) in [
            (true, None, true),
            (false, None, false),
            (false, Some(false), false),
            (true, Some(false), false),
            (false, Some(true), false),
            (true, Some(true), true),
        ] {
            let scope = task_scope(vec![target.clone()], allow_changes, unattended);
            assert_eq!(scope.targets, vec![target.clone()]);
            assert_eq!(scope.can_modify, expected);
        }
    }

    #[test]
    fn checkpoint_envelope_retains_consumed_input_position() {
        let saved = SavedCheckpoint {
            version: 1,
            checkpoint: checkpoint("read_file", json!({"path":"/config"})),
            input_count: 3,
        };
        let restored: SavedCheckpoint =
            serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(restored.input_count, 3);
        assert_eq!(restored.checkpoint, saved.checkpoint);
    }

    #[test]
    fn private_task_bodies_are_encrypted_and_bound_to_their_session() {
        let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
        assert_eq!(vault.try_unlock(), rove_vault::LockState::Unlocked);
        let journal = SqliteJournal::open_in_memory().unwrap();
        let body = json!({"observation":"private operational record"});
        let reference = seal_private(&vault, &journal, "ai-fixture", &body).unwrap();
        let sealed = journal
            .private_body(&reference.session, &reference.reference.to_string())
            .unwrap();
        assert!(
            !sealed
                .windows(b"private operational record".len())
                .any(|window| window == b"private operational record")
        );
        assert_eq!(
            open_private::<Value>(&vault, &journal, &reference).unwrap(),
            body
        );
        let events = journal.read(&JournalQuery::default()).unwrap();
        assert!(
            !serde_json::to_string(&events[0].payload)
                .unwrap()
                .contains("observation")
        );
        let altered = PrivateRef {
            session: "ai-other".into(),
            ..reference.clone()
        };
        journal
            .append_private(
                NewRecord {
                    kind: "ops/private".into(),
                    session_id: Some(altered.session.clone()),
                    payload: json!({}),
                },
                &altered.session,
                &altered.reference.to_string(),
                &sealed,
            )
            .unwrap();
        assert!(open_private::<Value>(&vault, &journal, &altered).is_err());
    }

    #[test]
    fn ops_task_report_decrypts_host_evidence_and_preserves_other_hosts_on_error() {
        let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
        assert_eq!(vault.try_unlock(), rove_vault::LockState::Unlocked);
        let journal = SqliteJournal::open_in_memory().unwrap();
        let report = seal_private(&vault, &journal, "ai-fixture", &json!({
            "kind":"host_report", "stop_reason":"end_turn",
            "verification":{"verified":true,"criteria":["Service healthy","Port accessible"],"checks":[
                {"criterion_index":1,"summary":"Port open"},
                {"criterion_index":0,"summary":"Probe passed"}
            ]},
            "successful_operations":2,"failed_operations":1,"denied_operations":0,
            "reply":"界".repeat(MAX_REPORT_REPLY)
        })).unwrap();
        let failure = seal_private(
            &vault,
            &journal,
            "ai-fixture",
            &json!({
                "kind":"worker_failure", "code":"ssh.offline", "message":"Host unavailable"
            }),
        )
        .unwrap();
        let missing = PrivateRef {
            reference: uuid::Uuid::now_v7(),
            ..report.clone()
        };
        let mut task = OpsTask {
            id: OpsTaskId::new(),
            revision: 1,
            request_id: OpsRequestId::new("fixture").unwrap(),
            conversation: "ai-fixture".into(),
            source: TaskSource::User,
            state: OpsTaskState::Failed,
            deleted: false,
            scope: TaskScope {
                targets: Vec::new(),
                can_modify: false,
            },
            inputs: Vec::new(),
            acceptance: None,
            report: None,
            subtasks: Vec::new(),
            created_at: 0,
            updated_at: 0,
        };
        for (state, evidence) in [
            (OpsTaskState::Completed, report),
            (OpsTaskState::Failed, failure),
            (OpsTaskState::Blocked, missing),
        ] {
            let host_id = HostId::new();
            task.scope.targets.push(Target {
                host_id,
                fingerprint: "fixture".into(),
                input: None,
            });
            task.subtasks.push(rove_ops::OpsSubtask {
                id: OpsSubtaskId::new(),
                host_id,
                fingerprint: "fixture".into(),
                state,
                session: None,
                checkpoint: None,
                inputs: Vec::new(),
                input_version: 0,
                evidence: vec![evidence],
                reason: None,
                error_code: None,
                uncertain_effect: false,
                attempts: 1,
            });
        }
        let reader = |reference: &PrivateRef| open_private(&vault, &journal, reference);
        let summary = build_task_summary(&task, None, |_| "Saved host".into(), Some(&reader));
        assert_eq!(summary["settled"], true);
        assert_eq!(summary["counts"]["total"], 3);
        assert_eq!(summary["counts"]["completed"], 1);
        assert_eq!(summary["counts"]["failed"], 1);
        assert_eq!(summary["counts"]["blocked"], 1);
        assert_eq!(summary["subtasks"][0]["host_name"], "Saved host");
        let verified = &summary["subtasks"][0]["report"];
        assert_eq!(verified["verified"], true);
        assert_eq!(
            verified["checks"][0],
            json!({"criterion":"Service healthy","summary":"Probe passed"})
        );
        assert_eq!(verified["checks"][1]["summary"], "Port open");
        assert_eq!(verified["reply_clipped"], true);
        assert_eq!(
            verified["reply"].as_str().unwrap().len(),
            MAX_REPORT_REPLY / 3 * 3
        );
        assert_eq!(
            summary["subtasks"][1]["report"]["failure_code"],
            "ssh.offline"
        );
        assert_eq!(
            summary["subtasks"][1]["report"]["failure_message"],
            "Host unavailable"
        );
        assert_eq!(
            summary["subtasks"][2]["report_error"],
            "Could not read encrypted host evidence."
        );
        let filtered = build_task_summary(
            &task,
            Some(task.subtasks[1].host_id),
            |id| id.to_string(),
            Some(&reader),
        );
        assert_eq!(filtered["subtasks"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["counts"], summary["counts"]);
        assert_eq!(
            filtered["subtasks"][0]["report"],
            summary["subtasks"][1]["report"]
        );
        assert!(can_wake_lead(&task));
        task.subtasks[0].state = OpsTaskState::WaitingApproval;
        assert!(!can_wake_lead(&task));
        for sub in &mut task.subtasks {
            sub.state = OpsTaskState::Cancelled;
        }
        assert!(!can_wake_lead(&task));
        task.subtasks.pop();
        assert!(!settled(&task));
    }

    #[test]
    fn ops_report_fallback_renders_evidence_as_markdown_text() {
        let markdown = format_report_fallback(&[json!({
            "id":"task-1", "counts":{"total":2,"completed":1},
            "subtasks":[
                {"host_name":"web", "state":"completed", "report":{
                    "kind":"host_report", "verified":true, "checks":[{"criterion":"Health","summary":"Probe passed"}],
                    "reply":"<script>\n# found it at 10.0.0.1:22 /var/log/app_log"
                }},
                {"host_name":"db", "state":"blocked", "uncertain_effect":true,
                 "report":{"kind":"worker_failure","failure_message":"Disconnected"}}
            ]
        })]);
        assert!(markdown.starts_with("# Operations report\n\n1 of 2 host tasks completed."));
        assert!(markdown.contains("### web — completed\n\n- Verified\n- Health — Probe passed"));
        assert!(markdown.contains(r"\<script\> # found it at 10.0.0.1:22 /var/log/app_log"));
        assert!(markdown.contains("### db — blocked"));
        assert!(markdown.contains("Confirm the remote outcome"));
        assert!(markdown.contains("Worker failure — Disconnected"));
        assert!(!markdown.contains("host_name"));
    }

    #[test]
    fn ops_reports_without_verification_render_worker_findings() {
        let body = json!({
            "kind":"host_report","reply":"\n#\n> **Service healthy\nChecked /var/log",
            "acceptance":["Check the service"],"stop_reason":"end_turn",
        });
        let view = HostReport::from_body(&body);
        assert_eq!(view.summary.as_deref(), Some("Service healthy"));
        assert!(view.checks.is_empty());
        let mut sub = json!({"host_name":"web","state":"completed"});
        add_host_report(&mut sub, &body);
        assert!(sub["report"].get("verified").is_none());
        let markdown = format_report_fallback(&[json!({"subtasks":[sub]})]);
        assert!(markdown.contains("Worker findings"));
        assert!(!markdown.contains("Unverified"));
    }

    #[test]
    fn target_identity_ignores_display_changes_and_detects_connection_changes() {
        let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
        let mut host = vault
            .create_host(HostDraft {
                name: "first".into(),
                group: None,
                hostname: "Example.test".into(),
                port: 22,
                username: "test".into(),
                auth: AuthMethod::Agent,
                route: HostRoute::Direct,
                tags: vec![],
            })
            .unwrap();
        let first = target_fingerprint(&host, &rove_ssh::SshTarget::from_host(&host, &[])).unwrap();
        host.name = "renamed".into();
        host.updated_at += 1;
        host.tags.push("grouping".into());
        assert_eq!(
            target_fingerprint(&host, &rove_ssh::SshTarget::from_host(&host, &[])).unwrap(),
            first
        );
        host.port = 2222;
        assert_ne!(
            target_fingerprint(&host, &rove_ssh::SshTarget::from_host(&host, &[])).unwrap(),
            first
        );
    }

    #[cfg(unix)]
    mod ssh_recovery {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        use std::time::Duration;

        use devsshd::{DevConfig, DevServer};
        use rove_ai::assistant::{Assistant, AssistantCheckpoint, AssistantError, HostMeta};
        use rove_ai::audit::{Auditor, KIND_COMMAND};
        use rove_ai::host::{ConnectionSource, RemoteHost};
        use rove_ai::llm::{Client, Endpoint, Format};
        use rove_ai::session::{AiEvent, AiSession, SessionSettings};
        use rove_ai::{Answer, Mode};
        use rove_journal::{JournalQuery, JournalStore, NewRecord, SqliteJournal};
        use rove_kernel::EventBus;
        use rove_ops::{
            BlockReason, JournalTaskStore, OpsConfig, OpsRequestId, OpsSupervisor, OpsTaskState,
            PrivateRef, Target, TaskDraft, TaskScope, TaskSource,
        };
        use rove_ssh::{
            AuthSpec, HostKeyAnswer, HostKeyPrompt, HostKeyPrompts, SshConfig, SshConnection,
            SshConnector, SshTarget, VaultVerifier,
        };
        use rove_vault::{HostId, MemoryKeyCache, Vault};
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;
        use tokio::sync::mpsc;
        use zeroize::Zeroizing;

        use async_trait::async_trait;
        use serde_json::{Value, json};

        struct Source(Arc<SshConnection>);

        #[async_trait]
        impl ConnectionSource for Source {
            async fn connection(&self) -> Result<Arc<SshConnection>, String> {
                Ok(self.0.clone())
            }
        }

        fn seal(
            vault: &Vault,
            journal: &dyn JournalStore,
            session: &str,
            body: &Value,
        ) -> PrivateRef {
            let reference = uuid::Uuid::now_v7();
            let sealed = vault.seal_audit(session, reference, body).unwrap();
            journal
                .append_private(
                    NewRecord {
                        kind: "ops/private".into(),
                        session_id: Some(session.into()),
                        payload: json!({"reference":reference}),
                    },
                    session,
                    &reference.to_string(),
                    &sealed,
                )
                .unwrap();
            PrivateRef::new(session, reference.to_string()).unwrap()
        }

        fn open(vault: &Vault, journal: &dyn JournalStore, reference: &PrivateRef) -> Value {
            let sealed = journal
                .private_body(&reference.session, &reference.reference.to_string())
                .unwrap();
            vault
                .open_audit(&reference.session, reference.reference, &sealed)
                .unwrap()
        }

        async fn scripted_write_model(cwd: &str) -> (String, Arc<AtomicUsize>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let calls = Arc::new(AtomicUsize::new(0));
            let seen = calls.clone();
            let arguments = json!({
                "command":"printf 'applied\\n' >> apply.log; printf 'ssh-write-receipt\\n'",
                "cwd":cwd,
            });
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let (read, mut write) = stream.into_split();
                    let mut reader = BufReader::new(read);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap() == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    let mut raw = vec![0; length];
                    reader.read_exact(&mut raw).await.unwrap();
                    let request: Value = serde_json::from_slice(&raw).unwrap();
                    assert_eq!(request["model"], "scripted");
                    let index = seen.fetch_add(1, Ordering::SeqCst);
                    let delta = if index == 0 {
                        json!({"tool_calls":[{"index":0,"id":"write-1","function":{
                            "name":"run_command","arguments":arguments.to_string(),
                        }}]})
                    } else {
                        json!({"content":"unexpected continuation"})
                    };
                    let chunk = json!({"choices":[{"delta":delta,"finish_reason":"stop"}]});
                    let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
                    let header = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    write.write_all(header.as_bytes()).await.unwrap();
                    write.write_all(body.as_bytes()).await.unwrap();
                }
            });
            (base, calls)
        }

        #[tokio::test]
        async fn lost_checkpoint_after_real_ssh_write_stays_unknown_and_is_not_replayed() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let server = DevServer::start(
                "127.0.0.1:0",
                DevConfig {
                    sftp_root: Some(root.clone()),
                    ..DevConfig::demo()
                },
            )
            .await
            .unwrap();
            let vault =
                Arc::new(Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap());
            assert_eq!(vault.try_unlock(), rove_vault::LockState::Unlocked);
            let events = EventBus::default();
            let prompts = Arc::new(HostKeyPrompts::new(events.clone(), Duration::from_secs(5)));
            let responder = prompts.clone();
            let _listener = events.on::<HostKeyPrompt>(move |prompt| {
                responder.respond(&prompt.request_id, HostKeyAnswer::Trust);
            });
            let connector = SshConnector::new(
                &SshConfig::default(),
                Arc::new(VaultVerifier::new(vault.clone(), prompts)),
            );
            let target = SshTarget {
                hostname: "127.0.0.1".into(),
                port: server.addr().port(),
                username: "demo".into(),
                host_id: None,
                title: None,
                route: String::new(),
            };
            let connection = connector
                .connect(&target, &AuthSpec::Password(Zeroizing::new("demo".into())))
                .await
                .unwrap();
            let host = Arc::new(RemoteHost::new("SSH fixture", Arc::new(Source(connection))));
            let journal: Arc<dyn JournalStore> =
                Arc::new(SqliteJournal::open(&root.join("journal.db")).unwrap());
            let ops = Arc::new(
                OpsSupervisor::new(
                    Arc::new(JournalTaskStore::new(journal.clone()).unwrap()),
                    OpsConfig::default(),
                )
                .unwrap(),
            );
            let session_id = "ai-ssh-recovery";
            let host_id = HostId::new();
            let input = seal(
                &vault,
                journal.as_ref(),
                session_id,
                &json!({"task":"apply once"}),
            );
            let task = ops
                .create(TaskDraft {
                    request_id: OpsRequestId::new("ssh-recovery-request").unwrap(),
                    conversation: session_id.into(),
                    source: TaskSource::User,
                    input,
                    acceptance: None,
                    scope: TaskScope {
                        targets: vec![Target {
                            host_id,
                            fingerprint: "loopback-ssh-fixture".into(),
                            input: None,
                        }],
                        can_modify: true,
                    },
                })
                .unwrap();
            let sub = ops
                .ensure_subtask(task.id, host_id, "loopback-ssh-fixture")
                .unwrap();
            let lease = ops.lease_worker(task.id, sub.id).await.unwrap();
            let auditor = {
                let vault = vault.clone();
                let journal = journal.clone();
                let ops = ops.clone();
                Arc::new(Auditor::new(
                    Arc::new(move |record| {
                        let reference = seal(&vault, journal.as_ref(), session_id, &record.payload);
                        journal
                            .append(NewRecord {
                                payload: json!({"body_ref":reference.reference}),
                                ..record
                            })
                            .unwrap();
                        ops.record_evidence(task.id, sub.id, reference).unwrap();
                    }),
                    session_id,
                    "SSH fixture",
                ))
            };
            auditor.protect_text();
            let (approvals, mut asked) = mpsc::unbounded_channel();
            let session = Arc::new(AiSession::new(
                host.clone(),
                SessionSettings::default(),
                Arc::new(move |event| {
                    if let AiEvent::Approval { request_id, .. } = &event {
                        approvals.send(*request_id).unwrap();
                    }
                    auditor.event(&event);
                }),
            ));
            let weak = Arc::downgrade(&session);
            tokio::spawn(async move {
                while let Some(request_id) = asked.recv().await {
                    let Some(session) = weak.upgrade() else { break };
                    session.answer(request_id, Answer::AllowOnce).await;
                }
            });
            let (base, model_calls) = scripted_write_model(root.to_str().unwrap()).await;
            let client = Client::new(
                Endpoint {
                    format: Format::Chat,
                    base_url: base,
                    model: "scripted".into(),
                    max_tokens: None,
                    effort: None,
                },
                "",
                Duration::from_secs(5),
            )
            .unwrap();
            let lost_result = Arc::new(AtomicBool::new(false));
            let sink = {
                let vault = vault.clone();
                let journal = journal.clone();
                let ops = ops.clone();
                let lost = lost_result.clone();
                Arc::new(move |checkpoint: AssistantCheckpoint| {
                    if matches!(
                        checkpoint.messages.last(),
                        Some(rove_ai::llm::Message::Tool { .. })
                    ) {
                        lost.store(true, Ordering::SeqCst);
                        return Err("injected loss of the result checkpoint".into());
                    }
                    let pending = checkpoint.pending_tool.is_some();
                    let reference = seal(
                        &vault,
                        journal.as_ref(),
                        session_id,
                        &serde_json::to_value(checkpoint).unwrap(),
                    );
                    if pending {
                        ops.mark_effect_started(task.id, sub.id).unwrap();
                    }
                    ops.update_checkpoint(task.id, sub.id, reference).unwrap();
                    Ok(())
                })
            };
            let assistant = Assistant::new(client.clone(), session, &HostMeta::default())
                .with_checkpoint_sink(sink);
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(10), assistant.prompt("apply once"))
                    .await
                    .unwrap(),
                Err(AssistantError::Checkpoint(_))
            ));
            assert!(lost_result.load(Ordering::SeqCst));
            assert_eq!(
                std::fs::read_to_string(root.join("apply.log")).unwrap(),
                "applied\n"
            );
            assert_eq!(model_calls.load(Ordering::SeqCst), 1);
            let before = ops.get(task.id).unwrap();
            let saved = before.subtask(sub.id).unwrap().checkpoint.clone().unwrap();
            let checkpoint: AssistantCheckpoint =
                serde_json::from_value(open(&vault, journal.as_ref(), &saved)).unwrap();
            assert_eq!(checkpoint.pending_tool.as_deref(), Some("write-1"));
            assert!(before.subtask(sub.id).unwrap().uncertain_effect);
            let command = journal
                .read(&JournalQuery {
                    session_id: Some(session_id.into()),
                    kind: Some(KIND_COMMAND.into()),
                    ..JournalQuery::default()
                })
                .unwrap();
            assert_eq!(command.len(), 1);
            let reference =
                PrivateRef::new(session_id, command[0].payload["body_ref"].as_str().unwrap())
                    .unwrap();
            let audit = open(&vault, journal.as_ref(), &reference);
            assert_eq!(audit["outcome"]["stdout"], "ssh-write-receipt\n");
            assert_eq!(audit["approved"], true);
            assert!(
                before
                    .subtask(sub.id)
                    .unwrap()
                    .evidence
                    .contains(&reference)
            );
            let sealed = journal
                .private_body(session_id, &reference.reference.to_string())
                .unwrap();
            assert!(
                !sealed
                    .windows(17)
                    .any(|window| window == b"ssh-write-receipt")
            );
            assert!(!command[0].payload.to_string().contains("printf"));
            drop(assistant);
            drop(lease);
            drop(ops);

            let restarted = Arc::new(
                OpsSupervisor::new(
                    Arc::new(JournalTaskStore::new(journal.clone()).unwrap()),
                    OpsConfig::default(),
                )
                .unwrap(),
            );
            let retry = restarted.retry_failed(task.id).unwrap();
            let worker = retry.subtask(sub.id).unwrap();
            assert_eq!(worker.state, OpsTaskState::Blocked);
            assert_eq!(worker.reason, Some(BlockReason::UnresolvedEffect));
            assert!(worker.uncertain_effect);
            assert_eq!(worker.attempts, 1);
            assert!(worker.evidence.contains(&reference));
            assert!(restarted.lease_worker(task.id, sub.id).await.is_err());
            let read_only = Arc::new(AiSession::new(
                host,
                SessionSettings::default(),
                Arc::new(|event| assert!(!matches!(event, AiEvent::Approval { .. }))),
            ));
            read_only.restrict_read_only();
            read_only.set_mode(Mode::Auto).await;
            let restored = Assistant::new(client, read_only.clone(), &HostMeta::default());
            assert!(matches!(
                restored.restore_checkpoint(checkpoint).await,
                Err(AssistantError::Checkpoint(_))
            ));
            assert!(
                read_only
                    .run_command("printf replayed >> apply.log", root.to_str(), None)
                    .await
                    .is_err()
            );
            assert_eq!(read_only.take_deferred().len(), 1);
            assert_eq!(model_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                std::fs::read_to_string(root.join("apply.log")).unwrap(),
                "applied\n"
            );
        }
    }
}
