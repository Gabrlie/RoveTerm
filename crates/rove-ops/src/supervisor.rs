use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use parking_lot::Mutex;
use rove_vault::HostId;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    BlockReason, OpsConfig, OpsError, OpsSubtask, OpsSubtaskId, OpsTask, OpsTaskId, OpsTaskState,
    PrivateRef, TaskDraft, TaskStore, WorkerOutcome,
};

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

type WorkerKey = (OpsTaskId, OpsSubtaskId);

/// One process-local owner of a task store. All interactive and scheduled Consumers share it.
/// A lease must be acquired before a worker is executed; no worker is run by this metadata service.
pub struct OpsSupervisor {
    store: Arc<dyn TaskStore>,
    config: OpsConfig,
    serial: Mutex<()>,
    workers: Mutex<BTreeMap<WorkerKey, CancellationToken>>,
    runners: Mutex<BTreeSet<OpsTaskId>>,
    permits: Arc<Semaphore>,
    stop: CancellationToken,
    closed: AtomicBool,
    failed: AtomicBool,
    changes: watch::Sender<u64>,
}

impl OpsSupervisor {
    pub fn new(store: Arc<dyn TaskStore>, config: OpsConfig) -> Result<Self, OpsError> {
        config.validate()?;
        let supervisor = Self {
            permits: Arc::new(Semaphore::new(config.max_workers)),
            store,
            config,
            serial: Mutex::new(()),
            workers: Mutex::new(BTreeMap::new()),
            runners: Mutex::new(BTreeSet::new()),
            stop: CancellationToken::new(),
            closed: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            changes: watch::channel(0).0,
        };
        supervisor.recover()?;
        Ok(supervisor)
    }

    fn ensure_open(&self) -> Result<(), OpsError> {
        if self.closed.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire) {
            return Err(OpsError::Closed);
        }
        Ok(())
    }

    /// Subscribe before reading task state, then await changed() to avoid lost wakeups.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire)
    }

    fn notify(&self) {
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    fn persistence_failed(&self, error: &OpsError) {
        if matches!(error, OpsError::Journal(_)) {
            self.halt_on_storage_failure();
        }
    }

    /// Stops all dispatch and cancels active leases after any durable store becomes unavailable.
    /// The application calls this for private-body and checkpoint-index write failures too.
    pub fn halt_on_storage_failure(&self) {
        self.failed.store(true, Ordering::Release);
        self.stop.cancel();
        self.permits.close();
        self.notify();
    }

    fn change_inner(
        &self,
        id: OpsTaskId,
        action: impl FnOnce(&mut OpsTask) -> Result<(), OpsError>,
    ) -> Result<OpsTask, OpsError> {
        let _serial = self.serial.lock();
        let result = (|| {
            let mut task = self.store.get(id)?;
            let original = task.clone();
            let revision = task.revision;
            action(&mut task)?;
            if task == original {
                return Ok(task);
            }
            if task.report == original.report {
                task.report = None;
            }
            task.updated_at = now_millis();
            let task = self.store.save(task, revision)?;
            self.notify();
            Ok(task)
        })();
        if let Err(error) = &result {
            self.persistence_failed(error);
        }
        result
    }

    fn change(
        &self,
        id: OpsTaskId,
        action: impl FnOnce(&mut OpsTask) -> Result<(), OpsError>,
    ) -> Result<OpsTask, OpsError> {
        self.ensure_open()?;
        self.change_inner(id, |task| {
            if task.deleted {
                return Err(OpsError::TaskNotFound(id));
            }
            action(task)
        })
    }

    pub fn create(&self, draft: TaskDraft) -> Result<OpsTask, OpsError> {
        self.ensure_open()?;
        if draft.scope.targets.is_empty() || draft.scope.targets.len() > self.config.max_subtasks {
            return Err(OpsError::Invalid("task target limit exceeded".into()));
        }
        let now = now_millis();
        let task = OpsTask {
            id: OpsTaskId::new(),
            revision: 0,
            request_id: draft.request_id,
            conversation: draft.conversation,
            source: draft.source,
            state: OpsTaskState::Queued,
            deleted: false,
            scope: draft.scope,
            inputs: vec![draft.input],
            acceptance: draft.acceptance,
            report: None,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let result = self.store.create(task);
        if let Err(error) = &result {
            self.persistence_failed(error);
        }
        if result.is_ok() {
            self.notify();
        }
        result
    }

    pub fn get(&self, id: OpsTaskId) -> Result<OpsTask, OpsError> {
        let task = self.store.get(id)?;
        if task.deleted {
            return Err(OpsError::TaskNotFound(id));
        }
        Ok(task)
    }

    pub fn list(&self) -> Result<Vec<OpsTask>, OpsError> {
        let mut tasks = self.store.list()?;
        tasks.retain(|task| !task.deleted);
        tasks.sort_by_key(|task| std::cmp::Reverse((task.updated_at, task.id)));
        Ok(tasks)
    }

    /// Appends explicit input without re-running any host. The Consumer selects hosts with requeue_subtask.
    pub fn append_input(&self, id: OpsTaskId, input: PrivateRef) -> Result<OpsTask, OpsError> {
        input.validate()?;
        self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            if task.inputs.len() >= self.config.max_inputs {
                return Err(OpsError::Invalid("task input limit reached".into()));
            }
            task.inputs.push(input);
            task.aggregate();
            Ok(())
        })
    }

    /// Returns the existing worker identity for a host, or records a new queued subtask.
    /// A changed resolved endpoint never silently replaces the frozen target.
    pub fn ensure_subtask(
        &self,
        id: OpsTaskId,
        host_id: HostId,
        fingerprint: &str,
    ) -> Result<OpsSubtask, OpsError> {
        let task = self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            let target = task
                .scope
                .targets
                .iter()
                .find(|target| target.host_id == host_id)
                .ok_or(OpsError::Forbidden)?;
            if target.fingerprint != fingerprint {
                return Err(OpsError::Forbidden);
            }
            if task.subtasks.iter().any(|sub| sub.host_id == host_id) {
                return Ok(());
            }
            if task.subtasks.len() >= self.config.max_subtasks {
                return Err(OpsError::Invalid("task worker limit reached".into()));
            }
            let input = match &target.input {
                Some(input) => input.clone(),
                None => task.inputs[0].clone(),
            };
            let mut inputs = vec![input];
            inputs.extend(task.inputs.iter().skip(1).cloned());
            task.subtasks.push(OpsSubtask {
                id: OpsSubtaskId::new(),
                host_id,
                fingerprint: fingerprint.to_owned(),
                state: OpsTaskState::Queued,
                session: None,
                checkpoint: None,
                inputs,
                input_version: task.inputs.len(),
                evidence: Vec::new(),
                reason: None,
                error_code: None,
                uncertain_effect: false,
                attempts: 0,
            });
            task.aggregate();
            Ok(())
        })?;
        task.subtasks
            .into_iter()
            .find(|sub| sub.host_id == host_id)
            .ok_or(OpsError::Forbidden)
    }

    pub fn requeue_subtask(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
    ) -> Result<OpsTask, OpsError> {
        if self.workers.lock().contains_key(&(id, subtask)) {
            return Err(OpsError::Busy);
        }
        self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            let inputs = task.inputs.clone();
            let sub = task.subtask_mut(subtask)?;
            if sub.uncertain_effect || sub.reason == Some(BlockReason::UnresolvedEffect) {
                return Err(OpsError::UncertainEffect);
            }
            if sub.state == OpsTaskState::Completed && sub.input_version >= inputs.len() {
                return Err(OpsError::NoNewInput);
            }
            if matches!(
                sub.state,
                OpsTaskState::Running | OpsTaskState::WaitingApproval
            ) {
                return Err(OpsError::Busy);
            }
            sub.inputs
                .extend(inputs.iter().skip(sub.input_version).cloned());
            sub.input_version = inputs.len();
            sub.state = OpsTaskState::Queued;
            sub.reason = None;
            sub.error_code = None;
            sub.evidence.clear();
            task.aggregate();
            Ok(())
        })
    }

    /// Queues only failed, blocked or interrupted work. Completed hosts retain their result.
    pub fn retry_failed(&self, id: OpsTaskId) -> Result<OpsTask, OpsError> {
        self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            for sub in &mut task.subtasks {
                if matches!(
                    sub.state,
                    OpsTaskState::Failed | OpsTaskState::Blocked | OpsTaskState::Interrupted
                ) && !sub.uncertain_effect
                    && sub.reason != Some(BlockReason::UnresolvedEffect)
                {
                    sub.state = OpsTaskState::Queued;
                    sub.reason = None;
                    sub.error_code = None;
                    sub.evidence.clear();
                }
            }
            task.aggregate();
            Ok(())
        })
    }

    pub fn retry_subtask(&self, id: OpsTaskId, subtask: OpsSubtaskId) -> Result<OpsTask, OpsError> {
        if self.workers.lock().contains_key(&(id, subtask)) {
            return Err(OpsError::Busy);
        }
        self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            let sub = task.subtask_mut(subtask)?;
            if sub.uncertain_effect || sub.reason == Some(BlockReason::UnresolvedEffect) {
                return Err(OpsError::UncertainEffect);
            }
            match sub.state {
                OpsTaskState::Running | OpsTaskState::WaitingApproval => {
                    return Err(OpsError::Busy);
                }
                OpsTaskState::Completed => return Err(OpsError::NoNewInput),
                OpsTaskState::Queued => return Ok(()),
                OpsTaskState::Cancelled => return Err(OpsError::Forbidden),
                OpsTaskState::Failed | OpsTaskState::Blocked | OpsTaskState::Interrupted => {
                    sub.state = OpsTaskState::Queued;
                    sub.reason = None;
                    sub.error_code = None;
                    sub.evidence.clear();
                }
            }
            task.aggregate();
            Ok(())
        })
    }

    /// Prevents duplicate task routers while allowing that router to dispatch distinct hosts concurrently.
    pub fn claim_task(self: &Arc<Self>, id: OpsTaskId) -> Result<TaskRunGuard, OpsError> {
        self.ensure_open()?;
        self.get(id)?;
        if !self.runners.lock().insert(id) {
            return Err(OpsError::Busy);
        }
        Ok(TaskRunGuard {
            supervisor: Arc::downgrade(self),
            id,
        })
    }

    /// Acquires the shared worker budget and durably marks this queued worker running.
    /// A queued duplicate for the same subtask is rejected before consuming a global slot.
    pub async fn lease_worker(
        self: &Arc<Self>,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
    ) -> Result<WorkerLease, OpsError> {
        self.ensure_open()?;
        let token = self.stop.child_token();
        let task = self.get(id)?;
        if task.state == OpsTaskState::Cancelled
            || task.subtask(subtask)?.state != OpsTaskState::Queued
        {
            return Err(OpsError::Forbidden);
        }
        if task.subtask(subtask)?.uncertain_effect
            || task.subtask(subtask)?.reason == Some(BlockReason::UnresolvedEffect)
        {
            return Err(OpsError::UncertainEffect);
        }
        {
            let mut workers = self.workers.lock();
            if workers.contains_key(&(id, subtask)) {
                return Err(OpsError::Busy);
            }
            workers.insert((id, subtask), token.clone());
        }
        let mut reservation = WorkerReservation {
            supervisor: self.clone(),
            key: (id, subtask),
            armed: true,
        };
        let task = self.get(id)?;
        if task.state == OpsTaskState::Cancelled
            || task.subtask(subtask)?.state != OpsTaskState::Queued
        {
            return Err(OpsError::Forbidden);
        }
        let permit = tokio::select! {
            _ = token.cancelled() => return Err(OpsError::Closed),
            permit = self.permits.clone().acquire_owned() => permit.map_err(|_| OpsError::Closed)?,
        };
        self.change(id, |task| {
            if task.state == OpsTaskState::Cancelled {
                return Err(OpsError::Forbidden);
            }
            let sub = task.subtask_mut(subtask)?;
            if sub.state != OpsTaskState::Queued
                || sub.uncertain_effect
                || sub.reason == Some(BlockReason::UnresolvedEffect)
            {
                return Err(OpsError::Forbidden);
            }
            sub.state = OpsTaskState::Running;
            sub.attempts = sub
                .attempts
                .checked_add(1)
                .ok_or_else(|| OpsError::Invalid("worker attempt count overflow".into()))?;
            sub.reason = None;
            task.aggregate();
            Ok(())
        })?;
        reservation.armed = false;
        Ok(WorkerLease {
            supervisor: Arc::downgrade(self),
            task_id: id,
            subtask_id: subtask,
            cancellation: token,
            _permit: permit,
        })
    }

    pub fn update_subtask_session(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        session: String,
    ) -> Result<OpsTask, OpsError> {
        if session.is_empty() || session.len() > 256 || session.chars().any(char::is_control) {
            return Err(OpsError::Invalid(
                "invalid worker session identifier".into(),
            ));
        }
        self.change(id, |task| {
            task.subtask_mut(subtask)?.session = Some(session);
            Ok(())
        })
    }

    pub fn update_checkpoint(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        checkpoint: PrivateRef,
    ) -> Result<OpsTask, OpsError> {
        checkpoint.validate()?;
        self.change(id, |task| {
            task.subtask_mut(subtask)?.checkpoint = Some(checkpoint);
            Ok(())
        })
    }

    pub fn set_report(&self, id: OpsTaskId, report: PrivateRef) -> Result<OpsTask, OpsError> {
        report.validate()?;
        self.change(id, |task| {
            task.report = Some(report);
            Ok(())
        })
    }

    /// Publishes a report only for the exact state whose evidence it describes.
    pub fn set_report_if_current(
        &self,
        id: OpsTaskId,
        revision: u64,
        report: PrivateRef,
    ) -> Result<OpsTask, OpsError> {
        report.validate()?;
        self.change(id, |task| {
            if task.revision != revision {
                return Err(OpsError::Conflict);
            }
            task.report = Some(report);
            Ok(())
        })
    }

    pub fn record_evidence(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        evidence: PrivateRef,
    ) -> Result<OpsTask, OpsError> {
        evidence.validate()?;
        self.change(id, |task| {
            let sub = task.subtask_mut(subtask)?;
            if !sub.evidence.contains(&evidence) {
                sub.evidence.push(evidence);
            }
            Ok(())
        })
    }

    pub fn set_waiting_approval(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        waiting: bool,
    ) -> Result<OpsTask, OpsError> {
        self.change(id, |task| {
            let sub = task.subtask_mut(subtask)?;
            match (&sub.state, waiting) {
                (OpsTaskState::Running, true) => sub.state = OpsTaskState::WaitingApproval,
                (OpsTaskState::WaitingApproval, false)
                    if self.workers.lock().contains_key(&(id, subtask)) =>
                {
                    sub.state = OpsTaskState::Running
                }
                (OpsTaskState::WaitingApproval, true) | (OpsTaskState::Running, false) => {
                    return Ok(());
                }
                _ => return Err(OpsError::Forbidden),
            }
            task.aggregate();
            Ok(())
        })
    }

    /// Record the intent before starting a remote modification. The caller still needs Policy approval.
    pub fn mark_effect_started(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
    ) -> Result<OpsTask, OpsError> {
        self.change(id, |task| {
            if !task.scope.can_modify {
                return Err(OpsError::Forbidden);
            }
            let sub = task.subtask_mut(subtask)?;
            if sub.state != OpsTaskState::Running || sub.uncertain_effect {
                return Err(OpsError::Forbidden);
            }
            sub.uncertain_effect = true;
            Ok(())
        })
    }

    /// Clears an in-flight effect only after a durable receipt or explicit verification was saved.
    pub fn resolve_effect(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        evidence: PrivateRef,
    ) -> Result<OpsTask, OpsError> {
        evidence.validate()?;
        self.change(id, |task| {
            let sub = task.subtask_mut(subtask)?;
            sub.uncertain_effect = false;
            sub.evidence.push(evidence);
            if sub.reason == Some(BlockReason::UnresolvedEffect) {
                sub.reason = Some(BlockReason::UserDecision);
            }
            Ok(())
        })
    }

    pub fn finish_subtask(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
        outcome: WorkerOutcome,
    ) -> Result<OpsTask, OpsError> {
        self.change(id, |task| {
            let cancelled = task.state == OpsTaskState::Cancelled;
            let sub = task.subtask_mut(subtask)?;
            if cancelled || sub.state == OpsTaskState::Cancelled {
                if let WorkerOutcome::Completed { evidence } = outcome {
                    evidence.validate()?;
                    if !sub.evidence.contains(&evidence) {
                        sub.evidence.push(evidence);
                    }
                }
                return Ok(());
            }
            if !matches!(
                sub.state,
                OpsTaskState::Running | OpsTaskState::WaitingApproval
            ) {
                return Err(OpsError::Forbidden);
            }
            sub.reason = None;
            sub.error_code = None;
            if sub.uncertain_effect {
                sub.state = OpsTaskState::Blocked;
                sub.reason = Some(BlockReason::UnresolvedEffect);
            } else {
                match outcome {
                    WorkerOutcome::Completed { evidence } => {
                        evidence.validate()?;
                        if !sub.evidence.contains(&evidence) {
                            sub.evidence.push(evidence);
                        }
                        sub.state = OpsTaskState::Completed;
                    }
                    WorkerOutcome::Blocked { reason } => {
                        sub.state = OpsTaskState::Blocked;
                        sub.reason = Some(reason);
                    }
                    WorkerOutcome::Failed { code } => {
                        if code.is_empty()
                            || code.len() > 128
                            || !code.bytes().all(|b| {
                                b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)
                            })
                        {
                            return Err(OpsError::Invalid(
                                "worker error must be a non-secret error code".into(),
                            ));
                        }
                        sub.state = OpsTaskState::Failed;
                        sub.error_code = Some(code);
                    }
                    WorkerOutcome::MaxRequests => {
                        sub.state = OpsTaskState::Blocked;
                        sub.reason = Some(BlockReason::BudgetExhausted);
                    }
                    WorkerOutcome::Cancelled => sub.state = OpsTaskState::Cancelled,
                    WorkerOutcome::Interrupted => {
                        sub.state = OpsTaskState::Interrupted;
                        sub.reason = Some(BlockReason::WorkerInterrupted);
                    }
                }
            }
            task.aggregate();
            Ok(())
        })
    }

    pub fn cancel(&self, id: OpsTaskId) -> Result<OpsTask, OpsError> {
        let task = self.change(id, |task| {
            task.state = OpsTaskState::Cancelled;
            for sub in &mut task.subtasks {
                if sub.state != OpsTaskState::Completed {
                    sub.state = OpsTaskState::Cancelled;
                }
            }
            Ok(())
        })?;
        for ((task_id, _), token) in self.workers.lock().iter() {
            if *task_id == id {
                token.cancel();
            }
        }
        Ok(task)
    }

    pub fn cancel_subtask(
        &self,
        id: OpsTaskId,
        subtask: OpsSubtaskId,
    ) -> Result<OpsTask, OpsError> {
        let task = self.change(id, |task| {
            let sub = task.subtask_mut(subtask)?;
            if !matches!(sub.state, OpsTaskState::Completed | OpsTaskState::Cancelled) {
                sub.state = OpsTaskState::Cancelled;
                sub.reason = None;
                sub.error_code = None;
                task.aggregate();
            }
            Ok(())
        })?;
        if let Some(token) = self.workers.lock().get(&(id, subtask)) {
            token.cancel();
        }
        Ok(task)
    }

    pub fn cancel_conversation(&self, conversation: &str) -> Result<Vec<OpsTask>, OpsError> {
        let mut cancelled = Vec::new();
        for task in self.list()? {
            if task.conversation == conversation
                && !matches!(
                    task.state,
                    OpsTaskState::Completed | OpsTaskState::Failed | OpsTaskState::Cancelled
                )
            {
                cancelled.push(self.cancel(task.id)?);
            }
        }
        Ok(cancelled)
    }

    /// Tombstones task metadata after its parent conversation was deleted by the user.
    pub fn forget_conversation(&self, conversation: &str) -> Result<(), OpsError> {
        self.cancel_conversation(conversation)?;
        if self.conversation_active(conversation)? {
            return Err(OpsError::Busy);
        }
        for task in self.list()? {
            if task.conversation == conversation {
                self.change(task.id, |task| {
                    task.deleted = true;
                    Ok(())
                })?;
            }
        }
        Ok(())
    }

    fn conversation_active(&self, conversation: &str) -> Result<bool, OpsError> {
        let ids: BTreeSet<_> = self
            .store
            .list()?
            .into_iter()
            .filter(|task| task.conversation == conversation)
            .map(|task| task.id)
            .collect();
        Ok(self.workers.lock().keys().any(|(id, _)| ids.contains(id))
            || self.runners.lock().iter().any(|id| ids.contains(id)))
    }

    /// Waits for routing and worker leases, including cancelled workers, to leave the conversation.
    pub async fn wait_conversation_idle(&self, conversation: &str) -> Result<(), OpsError> {
        let mut changes = self.subscribe();
        loop {
            if !self.conversation_active(conversation)? {
                return Ok(());
            }
            changes.changed().await.map_err(|_| OpsError::Closed)?;
        }
    }

    fn recover(&self) -> Result<(), OpsError> {
        for task in self.store.list()? {
            if !task.subtasks.iter().any(|sub| {
                matches!(
                    sub.state,
                    OpsTaskState::Running | OpsTaskState::WaitingApproval
                )
            }) {
                continue;
            }
            self.change_inner(task.id, |task| {
                for sub in &mut task.subtasks {
                    if matches!(
                        sub.state,
                        OpsTaskState::Running | OpsTaskState::WaitingApproval
                    ) {
                        interrupt(sub);
                    }
                }
                task.aggregate();
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), OpsError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.stop.cancel();
        self.permits.close();
        let result = self.recover();
        self.notify();
        result
    }

    fn release_worker(&self, id: OpsTaskId, subtask: OpsSubtaskId) {
        self.workers.lock().remove(&(id, subtask));
        self.notify();
        let result = self.change_inner(id, |task| {
            let sub = task.subtask_mut(subtask)?;
            if matches!(
                sub.state,
                OpsTaskState::Running | OpsTaskState::WaitingApproval
            ) {
                interrupt(sub);
                task.aggregate();
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(%error, "worker release could not be recorded");
        }
    }
}

fn interrupt(sub: &mut OpsSubtask) {
    let waiting = sub.state == OpsTaskState::WaitingApproval;
    sub.state = if sub.uncertain_effect || waiting {
        OpsTaskState::Blocked
    } else {
        OpsTaskState::Interrupted
    };
    sub.reason = Some(if sub.uncertain_effect {
        BlockReason::UnresolvedEffect
    } else if waiting {
        BlockReason::UserDecision
    } else {
        BlockReason::WorkerInterrupted
    });
}

struct WorkerReservation {
    supervisor: Arc<OpsSupervisor>,
    key: WorkerKey,
    armed: bool,
}
impl Drop for WorkerReservation {
    fn drop(&mut self) {
        if self.armed {
            self.supervisor.workers.lock().remove(&self.key);
            self.supervisor.notify();
        }
    }
}

pub struct WorkerLease {
    supervisor: Weak<OpsSupervisor>,
    pub task_id: OpsTaskId,
    pub subtask_id: OpsSubtaskId,
    pub cancellation: CancellationToken,
    _permit: OwnedSemaphorePermit,
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        if let Some(supervisor) = self.supervisor.upgrade() {
            supervisor.release_worker(self.task_id, self.subtask_id);
        }
    }
}

pub struct TaskRunGuard {
    supervisor: Weak<OpsSupervisor>,
    id: OpsTaskId,
}
impl Drop for TaskRunGuard {
    fn drop(&mut self) {
        if let Some(supervisor) = self.supervisor.upgrade() {
            supervisor.runners.lock().remove(&self.id);
            supervisor.notify();
        }
    }
}
