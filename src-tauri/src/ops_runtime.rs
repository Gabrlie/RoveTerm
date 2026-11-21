//! Host-bound model workers for the durable operations service.

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
    pub task: String,
    pub session: String,
    pub checkpoint: Option<AssistantCheckpoint>,
    pub persist: CheckpointSink,
    pub can_modify: bool,
    pub on_event: Arc<dyn Fn(&AiEvent) + Send + Sync>,
    pub acceptance: Vec<String>,
}

#[derive(Clone)]
pub struct OpsWorker {
    pub id: String,
    pub assistant: Arc<Assistant>,
    pub verification: Arc<crate::ops_verifier::Verification>,
}

struct WorkerEnd {
    app: AppHandle,
    id: Option<String>,
}
impl WorkerEnd {
    async fn close(&mut self) {
        if let Some(id) = self.id.take() {
            let state = self.app.state::<AppState>();
            ai::end_subagent(&self.app, &state, &id).await;
        }
    }
}
impl Drop for WorkerEnd {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                ai::end_subagent(&app, &state, &id).await;
            });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchInput {
    pub host_id: HostId,
    pub task: String,
    pub acceptance: Vec<String>,
}

pub struct DispatchRequest {
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

fn conservative_mode(initial: Mode, current: Mode) -> Mode {
    if matches!(initial, Mode::AskAll) || matches!(current, Mode::AskAll) {
        Mode::AskAll
    } else if matches!(initial, Mode::AskByRisk) || matches!(current, Mode::AskByRisk) {
        Mode::AskByRisk
    } else {
        Mode::Auto
    }
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

fn emit(app: &AppHandle, task: &OpsTask) {
    if let Err(error) = app.emit("ops-update", task) {
        tracing::debug!(%error, "operations update unavailable");
    }
}

pub async fn dispatch(
    app: &AppHandle,
    parent: &str,
    request: DispatchRequest,
) -> Result<OpsTask, CommandError> {
    let DispatchRequest {
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
            || task.acceptance.is_empty()
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
            scope: TaskScope {
                targets,
                can_modify: allow_changes && unattended != Some(false),
            },
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
        if let Ok(task) = ops.get(id)
            && task.state != OpsTaskState::Cancelled
            && task
                .subtasks
                .iter()
                .any(|sub| sub.state == OpsTaskState::Queued)
        {
            kick(&app, id);
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
        "read_file" | "list_dir" | "read_notes" | "terminal_snapshot" | "read_terminal"
        | "submit_result" => false,
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
    let result = execute_worker(&app, id, sub_id, lease.cancellation.clone()).await;
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
    match ops.finish_subtask(id, sub_id, outcome) {
        Ok(task) => emit(&app, &task),
        Err(error) => tracing::warn!(%error, "operations worker outcome was not recorded"),
    }
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
        format!(
            "{}\n\nAcceptance checks:\n{}\n\nVerify these checks with host tools before reporting success. State any unresolved checks explicitly.",
            initial.task,
            initial
                .acceptance
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let mut acceptance = initial.acceptance.clone();
    let mut followup_index = 0;
    for reference in sub.inputs.iter().skip(consumed) {
        let followup: Followup = read_private(&state, reference)?;
        if followup.host_ids.contains(&sub.host_id) {
            followup_index += 1;
            text.push_str(&format!(
                "\n\nUser follow-up {followup_index}:\n{}",
                followup.text
            ));
            acceptance.push(format!(
                "Address user follow-up {followup_index} shown in this turn."
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
    let current_mode = match ai::ops_live_mode(app, &task.conversation).await {
        Some(mode) => mode,
        None => ai::mode(state.vault.preferences()?.ai.mode),
    };
    let mode = conservative_mode(bundle.mode, current_mode);
    let worker = ai::start_ops_worker(
        app,
        &task.conversation,
        sub.host_id,
        &bundle.provider,
        mode,
        bundle.unattended,
        WorkerSpec {
            task: text.clone(),
            session,
            checkpoint,
            persist,
            can_modify,
            on_event: observe(app.clone(), id, sub_id, counts.clone()),
            acceptance: acceptance.clone(),
        },
    )
    .await?;
    let mut end = WorkerEnd {
        app: app.clone(),
        id: Some(worker.id.clone()),
    };
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => { worker.assistant.cancel().await; Ok("cancelled".to_owned()) },
        result = worker.assistant.prompt(&text) => result,
    };
    let reply = worker.assistant.last_reply().await;
    let reason = result.as_deref().unwrap_or("failed");
    let report = save_private(
        &state,
        &task.conversation,
        &json!({
            "kind":"host_report", "host_id":sub.host_id, "worker_session":worker.id,
            "acceptance":acceptance, "reply":reply, "stop_reason":reason,
            "verification":worker.verification.evidence(),
            "successful_operations":counts.successful.load(Ordering::Acquire),
            "failed_operations":counts.failed.load(Ordering::Acquire), "denied_operations":counts.denied.load(Ordering::Acquire),
        }),
    )?;
    state
        .ops
        .record_evidence(id, sub_id, report.clone())
        .map_err(error)?;
    end.close().await;
    if counts.storage_failed.load(Ordering::Acquire) {
        return Ok(WorkerOutcome::Blocked {
            reason: BlockReason::NeedsVerification,
        });
    }
    if reason == "end_turn"
        && (!worker.verification.is_verified() || counts.successful.load(Ordering::Acquire) == 0)
    {
        return Ok(WorkerOutcome::Blocked {
            reason: BlockReason::NeedsVerification,
        });
    }
    Ok(WorkerOutcome::from_stop_reason(reason, Some(report)))
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
            if let Some(saved) = &sub.checkpoint {
                let mut checkpoint: SavedCheckpoint = read_private(&state, saved)?;
                checkpoint.checkpoint.resolve_interrupted();
                let updated = save_private(
                    &state,
                    &saved.session,
                    &serde_json::to_value(checkpoint).map_err(|_| store_error())?,
                )?;
                state
                    .ops
                    .update_checkpoint(id, sub.id, updated)
                    .map_err(error)?;
            }
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
    fn pending_tools_distinguish_read_only_work_and_submission_from_modifications() {
        for name in [
            "read_file",
            "list_dir",
            "read_notes",
            "read_terminal",
            "terminal_snapshot",
            "submit_result",
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
    fn task_mode_cannot_widen_either_the_original_or_current_interactive_policy() {
        assert_eq!(conservative_mode(Mode::AskAll, Mode::Auto), Mode::AskAll);
        assert_eq!(conservative_mode(Mode::Auto, Mode::AskAll), Mode::AskAll);
        assert_eq!(
            conservative_mode(Mode::Auto, Mode::AskByRisk),
            Mode::AskByRisk
        );
        assert_eq!(
            conservative_mode(Mode::AskByRisk, Mode::Auto),
            Mode::AskByRisk
        );
        assert_eq!(conservative_mode(Mode::Auto, Mode::Auto), Mode::Auto);
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
}
