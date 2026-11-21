use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rove_journal::{
    JournalError, JournalQuery, JournalRecord, JournalStore, NewRecord, SqliteJournal,
};
use rove_ops::*;
use rove_vault::HostId;

fn body() -> PrivateRef {
    PrivateRef::new("ai-test", uuid::Uuid::now_v7().to_string()).unwrap()
}

fn draft(hosts: &[HostId]) -> TaskDraft {
    TaskDraft {
        request_id: OpsRequestId::new(uuid::Uuid::now_v7().to_string()).unwrap(),
        conversation: "ai-test".into(),
        source: TaskSource::User,
        input: body(),
        acceptance: Some(body()),
        scope: TaskScope {
            targets: hosts
                .iter()
                .map(|id| Target {
                    host_id: *id,
                    fingerprint: format!("target:{id}"),
                    input: None,
                })
                .collect(),
            can_modify: true,
        },
    }
}

fn setup(
    workers: usize,
) -> (
    Arc<dyn JournalStore>,
    Arc<dyn TaskStore>,
    Arc<OpsSupervisor>,
) {
    let journal: Arc<dyn JournalStore> = Arc::new(SqliteJournal::open_in_memory().unwrap());
    let store: Arc<dyn TaskStore> = Arc::new(JournalTaskStore::new(journal.clone()).unwrap());
    let supervisor = Arc::new(
        OpsSupervisor::new(
            store.clone(),
            OpsConfig {
                max_workers: workers,
                ..OpsConfig::default()
            },
        )
        .unwrap(),
    );
    (journal, store, supervisor)
}

fn sub(supervisor: &OpsSupervisor, task: &OpsTask, host: HostId) -> OpsSubtask {
    supervisor
        .ensure_subtask(task.id, host, &format!("target:{host}"))
        .unwrap()
}

#[test]
fn request_identity_is_opaque_bounded_and_serializable() {
    let id: OpsRequestId = "schedule:123:456:0".parse().unwrap();
    assert_eq!(
        serde_json::from_str::<OpsRequestId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
    for bad in ["".to_owned(), "a\nb".into(), "x".repeat(513)] {
        assert!(OpsRequestId::new(bad).is_err());
    }
    assert!("bad".parse::<OpsTaskId>().is_err());
    assert!(PrivateRef::new("", uuid::Uuid::now_v7().to_string()).is_err());
    assert!(PrivateRef::new("ai-test", "not-a-reference").is_err());
}

#[test]
fn validates_configuration_and_source_scope_before_persisting() {
    let (journal, _, supervisor) = setup(2);
    assert!(
        OpsConfig {
            max_workers: 0,
            ..OpsConfig::default()
        }
        .validate()
        .is_err()
    );
    let host = HostId::new();
    let mut request = draft(&[host, host]);
    assert!(supervisor.create(request.clone()).is_err());
    assert!(supervisor.create(draft(&[])).is_err());
    assert!(
        supervisor
            .create(draft(&(0..17).map(|_| HostId::new()).collect::<Vec<_>>()))
            .is_err()
    );
    request.scope.targets.pop();
    request.source = TaskSource::Schedule {
        schedule: String::new(),
        due: 1,
    };
    assert!(supervisor.create(request).is_err());
    assert!(journal.read(&JournalQuery::default()).unwrap().is_empty());
}

#[test]
fn deduplicates_source_requests_without_widening_scope() {
    let (_, _, supervisor) = setup(2);
    let request = draft(&[HostId::new()]);
    let first = supervisor.create(request.clone()).unwrap();
    let duplicate = supervisor.create(request.clone()).unwrap();
    assert_eq!(first, duplicate);
    let mut changed = request;
    changed.scope.can_modify = false;
    assert!(matches!(
        supervisor.create(changed),
        Err(OpsError::RequestConflict)
    ));
    assert_eq!(supervisor.list().unwrap().len(), 1);
}

#[test]
fn source_dedup_ignores_fresh_encrypted_references_and_target_order() {
    let (_, _, supervisor) = setup(2);
    let mut request = draft(&[HostId::new(), HostId::new()]);
    for target in &mut request.scope.targets {
        target.input = Some(body());
    }
    let first = supervisor.create(request.clone()).unwrap();
    request.input = body();
    request.scope.targets.reverse();
    for target in &mut request.scope.targets {
        target.input = Some(body());
    }
    assert_eq!(supervisor.create(request).unwrap().id, first.id);
}

#[test]
fn a_missing_scoped_subtask_retains_followups_after_partial_initialization() {
    let (_, _, supervisor) = setup(2);
    let hosts = [HostId::new(), HostId::new()];
    let mut request = draft(&hosts);
    let target_input = body();
    request.scope.targets[1].input = Some(target_input.clone());
    let task = supervisor.create(request).unwrap();
    sub(&supervisor, &task, hosts[0]);
    let followup = body();
    supervisor.append_input(task.id, followup.clone()).unwrap();
    let recovered = sub(&supervisor, &task, hosts[1]);
    assert_eq!(recovered.inputs, vec![target_input, followup]);
    assert_eq!(recovered.input_version, 2);
    assert_eq!(recovered.state, OpsTaskState::Queued);
    let current = supervisor.get(task.id).unwrap();
    assert_eq!(current.scope, task.scope);
    assert_eq!(current.subtasks.len(), 2);
    assert_eq!(sub(&supervisor, &task, hosts[1]), recovered);
}

#[tokio::test]
async fn abandoned_approval_is_blocked_and_explicit_input_can_resume_it() {
    let (_, store, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor
        .set_waiting_approval(task.id, worker.id, true)
        .unwrap();
    drop(lease);
    let blocked = supervisor.get(task.id).unwrap();
    assert_eq!(
        blocked.subtask(worker.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert_eq!(
        blocked.subtask(worker.id).unwrap().reason,
        Some(BlockReason::UserDecision)
    );
    drop(supervisor);
    let reopened = Arc::new(OpsSupervisor::new(store, OpsConfig::default()).unwrap());
    reopened.append_input(task.id, body()).unwrap();
    reopened.requeue_subtask(task.id, worker.id).unwrap();
    let resumed = reopened.lease_worker(task.id, worker.id).await.unwrap();
    assert_eq!(
        reopened
            .get(task.id)
            .unwrap()
            .subtask(worker.id)
            .unwrap()
            .attempts,
        2
    );
    drop(resumed);
}

#[tokio::test]
async fn change_subscription_keeps_notifications_and_deleted_tasks_stay_hidden() {
    let (_, store, supervisor) = setup(1);
    let mut changes = supervisor.subscribe();
    let task = supervisor.create(draft(&[HostId::new()])).unwrap();
    changes.changed().await.unwrap();
    supervisor.append_input(task.id, body()).unwrap();
    assert!(changes.has_changed().unwrap());
    supervisor.forget_conversation(&task.conversation).unwrap();
    assert!(supervisor.list().unwrap().is_empty());
    assert!(matches!(
        supervisor.get(task.id),
        Err(OpsError::TaskNotFound(_))
    ));
    let reopened = OpsSupervisor::new(store, OpsConfig::default()).unwrap();
    assert!(reopened.list().unwrap().is_empty());
}

#[test]
fn report_is_invalidated_by_new_state_and_cannot_publish_a_stale_snapshot() {
    let (_, _, supervisor) = setup(1);
    let task = supervisor.create(draft(&[HostId::new()])).unwrap();
    let reported = supervisor
        .set_report_if_current(task.id, task.revision, body())
        .unwrap();
    assert!(reported.report.is_some());
    let changed = supervisor.append_input(task.id, body()).unwrap();
    assert!(changed.report.is_none());
    assert!(matches!(
        supervisor.set_report_if_current(task.id, reported.revision, body()),
        Err(OpsError::Conflict)
    ));
}

#[tokio::test]
async fn conversation_idle_waits_for_both_worker_and_router_leases() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let router = supervisor.claim_task(task.id).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor.cancel_conversation(&task.conversation).unwrap();
    let idle = supervisor.wait_conversation_idle(&task.conversation);
    tokio::pin!(idle);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut idle)
            .await
            .is_err()
    );
    drop(lease);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut idle)
            .await
            .is_err()
    );
    assert!(matches!(
        supervisor.forget_conversation(&task.conversation),
        Err(OpsError::Busy)
    ));
    drop(router);
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut idle)
        .await
        .unwrap()
        .unwrap();
    supervisor.forget_conversation(&task.conversation).unwrap();
    assert!(supervisor.list().unwrap().is_empty());
}

#[test]
fn rejects_unapproved_hosts_and_changed_targets() {
    let (_, store, supervisor) = setup(2);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    assert!(matches!(
        supervisor.ensure_subtask(task.id, HostId::new(), "somewhere"),
        Err(OpsError::Forbidden)
    ));
    assert!(matches!(
        supervisor.ensure_subtask(task.id, host, "changed"),
        Err(OpsError::Forbidden)
    ));
    let mut changed = task.clone();
    changed.scope.targets.push(Target {
        host_id: HostId::new(),
        fingerprint: "other".into(),
        input: None,
    });
    assert!(matches!(
        store.save(changed, task.revision),
        Err(OpsError::Forbidden)
    ));
}

#[test]
fn revision_conflicts_do_not_overwrite_newer_state() {
    let (_, store, supervisor) = setup(2);
    let task = supervisor.create(draft(&[HostId::new()])).unwrap();
    supervisor.append_input(task.id, body()).unwrap();
    assert!(matches!(
        store.save(task.clone(), task.revision),
        Err(OpsError::Conflict)
    ));
    assert_eq!(supervisor.get(task.id).unwrap().inputs.len(), 2);
}

#[tokio::test]
async fn completion_needs_evidence_and_all_scoped_hosts() {
    let (_, _, supervisor) = setup(2);
    let hosts = [HostId::new(), HostId::new()];
    let task = supervisor.create(draft(&hosts)).unwrap();
    let first = sub(&supervisor, &task, hosts[0]);
    let lease = supervisor.lease_worker(task.id, first.id).await.unwrap();
    let blocked = supervisor
        .finish_subtask(
            task.id,
            first.id,
            WorkerOutcome::from_stop_reason("end_turn", None),
        )
        .unwrap();
    assert_eq!(
        blocked.subtask(first.id).unwrap().state,
        OpsTaskState::Blocked
    );
    drop(lease);
    supervisor.retry_failed(task.id).unwrap();
    let lease = supervisor.lease_worker(task.id, first.id).await.unwrap();
    let result = supervisor
        .finish_subtask(
            task.id,
            first.id,
            WorkerOutcome::Completed { evidence: body() },
        )
        .unwrap();
    assert_eq!(result.state, OpsTaskState::Queued);
    drop(lease);
    let second = sub(&supervisor, &task, hosts[1]);
    let lease = supervisor.lease_worker(task.id, second.id).await.unwrap();
    assert_eq!(
        supervisor
            .finish_subtask(
                task.id,
                second.id,
                WorkerOutcome::Completed { evidence: body() }
            )
            .unwrap()
            .state,
        OpsTaskState::Completed
    );
    drop(lease);
}

#[tokio::test]
async fn retries_only_failed_hosts_and_followups_reuse_the_worker() {
    let (_, _, supervisor) = setup(2);
    let hosts = [HostId::new(), HostId::new()];
    let task = supervisor.create(draft(&hosts)).unwrap();
    let first = sub(&supervisor, &task, hosts[0]);
    let second = sub(&supervisor, &task, hosts[1]);
    supervisor
        .update_subtask_session(task.id, first.id, "ai-worker".into())
        .unwrap();
    let checkpoint = body();
    supervisor
        .update_checkpoint(task.id, first.id, checkpoint.clone())
        .unwrap();
    let lease = supervisor.lease_worker(task.id, first.id).await.unwrap();
    supervisor
        .finish_subtask(
            task.id,
            first.id,
            WorkerOutcome::Completed { evidence: body() },
        )
        .unwrap();
    drop(lease);
    let lease = supervisor.lease_worker(task.id, second.id).await.unwrap();
    supervisor
        .finish_subtask(
            task.id,
            second.id,
            WorkerOutcome::Failed {
                code: "host.unreachable".into(),
            },
        )
        .unwrap();
    drop(lease);
    let retry = supervisor.retry_failed(task.id).unwrap();
    assert_eq!(
        retry.subtask(first.id).unwrap().state,
        OpsTaskState::Completed
    );
    assert_eq!(
        retry.subtask(second.id).unwrap().state,
        OpsTaskState::Queued
    );
    assert!(matches!(
        supervisor.requeue_subtask(task.id, first.id),
        Err(OpsError::NoNewInput)
    ));
    supervisor.append_input(task.id, body()).unwrap();
    assert_eq!(
        supervisor
            .get(task.id)
            .unwrap()
            .subtask(first.id)
            .unwrap()
            .state,
        OpsTaskState::Completed
    );
    let continued = supervisor.requeue_subtask(task.id, first.id).unwrap();
    let continued = continued.subtask(first.id).unwrap();
    assert_eq!(continued.id, first.id);
    assert_eq!(continued.session.as_deref(), Some("ai-worker"));
    assert_eq!(continued.checkpoint, Some(checkpoint));
    assert_eq!(continued.inputs.len(), 2);
}

#[tokio::test]
async fn a_single_global_budget_spans_tasks_and_releases_on_drop() {
    let (_, _, supervisor) = setup(1);
    let a = HostId::new();
    let b = HostId::new();
    let first = supervisor.create(draft(&[a])).unwrap();
    let second = supervisor.create(draft(&[b])).unwrap();
    let first_sub = sub(&supervisor, &first, a);
    let second_sub = sub(&supervisor, &second, b);
    let first_lease = supervisor
        .lease_worker(first.id, first_sub.id)
        .await
        .unwrap();
    let wait = supervisor.lease_worker(second.id, second_sub.id);
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut wait)
            .await
            .is_err()
    );
    assert_eq!(
        supervisor
            .get(second.id)
            .unwrap()
            .subtask(second_sub.id)
            .unwrap()
            .attempts,
        0
    );
    assert!(
        supervisor
            .lease_worker(first.id, first_sub.id)
            .await
            .is_err()
    );
    drop(first_lease);
    let second_lease = tokio::time::timeout(std::time::Duration::from_secs(1), &mut wait)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        supervisor.get(first.id).unwrap().state,
        OpsTaskState::Interrupted
    );
    drop(second_lease);
}

#[tokio::test]
async fn followup_is_retained_while_a_worker_waits_for_the_global_permit() {
    let (_, _, supervisor) = setup(1);
    let hosts = [HostId::new(), HostId::new()];
    let first = supervisor.create(draft(&[hosts[0]])).unwrap();
    let second = supervisor.create(draft(&[hosts[1]])).unwrap();
    let first_sub = sub(&supervisor, &first, hosts[0]);
    let second_sub = sub(&supervisor, &second, hosts[1]);
    let first_lease = supervisor
        .lease_worker(first.id, first_sub.id)
        .await
        .unwrap();
    let pending = supervisor.lease_worker(second.id, second_sub.id);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    let followup = body();
    let updated = supervisor
        .append_input(second.id, followup.clone())
        .unwrap();
    assert_eq!(updated.inputs.last(), Some(&followup));
    assert!(matches!(
        supervisor.requeue_subtask(second.id, second_sub.id),
        Err(OpsError::Busy)
    ));
    drop(first_lease);
    let lease = pending.await.unwrap();
    supervisor
        .finish_subtask(
            second.id,
            second_sub.id,
            WorkerOutcome::Completed { evidence: body() },
        )
        .unwrap();
    drop(lease);
    let continued = supervisor
        .requeue_subtask(second.id, second_sub.id)
        .unwrap();
    let continued = continued.subtask(second_sub.id).unwrap();
    assert_eq!(continued.state, OpsTaskState::Queued);
    assert_eq!(continued.inputs.last(), Some(&followup));
    assert_eq!(continued.attempts, 1);
}

#[tokio::test]
async fn router_claim_is_exclusive_but_distinct_hosts_can_run() {
    let (_, _, supervisor) = setup(2);
    let hosts = [HostId::new(), HostId::new()];
    let task = supervisor.create(draft(&hosts)).unwrap();
    let claim = supervisor.claim_task(task.id).unwrap();
    assert!(matches!(
        supervisor.claim_task(task.id),
        Err(OpsError::Busy)
    ));
    let a = sub(&supervisor, &task, hosts[0]);
    let b = sub(&supervisor, &task, hosts[1]);
    let first = supervisor.lease_worker(task.id, a.id).await.unwrap();
    let second = supervisor.lease_worker(task.id, b.id).await.unwrap();
    drop(first);
    drop(second);
    drop(claim);
    assert!(supervisor.claim_task(task.id).is_ok());
}

#[tokio::test]
async fn unknown_effects_require_verified_receipts_before_retry() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor.mark_effect_started(task.id, worker.id).unwrap();
    let finished = supervisor
        .finish_subtask(
            task.id,
            worker.id,
            WorkerOutcome::Completed { evidence: body() },
        )
        .unwrap();
    assert_eq!(
        finished.subtask(worker.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert_eq!(
        finished.subtask(worker.id).unwrap().reason,
        Some(BlockReason::UnresolvedEffect)
    );
    drop(lease);
    let retry = supervisor.retry_failed(task.id).unwrap();
    assert_eq!(
        retry.subtask(worker.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert!(matches!(
        supervisor.requeue_subtask(task.id, worker.id),
        Err(OpsError::UncertainEffect)
    ));
    supervisor
        .resolve_effect(task.id, worker.id, body())
        .unwrap();
    assert_eq!(
        supervisor
            .retry_failed(task.id)
            .unwrap()
            .subtask(worker.id)
            .unwrap()
            .state,
        OpsTaskState::Queued
    );
}

#[tokio::test]
async fn reported_unknown_effect_requires_resolution_without_an_intent_marker() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor
        .finish_subtask(
            task.id,
            worker.id,
            WorkerOutcome::Blocked {
                reason: BlockReason::UnresolvedEffect,
            },
        )
        .unwrap();
    drop(lease);
    let task = supervisor.retry_failed(task.id).unwrap();
    assert_eq!(
        task.subtask(worker.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert!(matches!(
        supervisor.requeue_subtask(task.id, worker.id),
        Err(OpsError::UncertainEffect)
    ));
    assert!(supervisor.lease_worker(task.id, worker.id).await.is_err());
    supervisor
        .resolve_effect(task.id, worker.id, body())
        .unwrap();
    supervisor.retry_failed(task.id).unwrap();
    assert!(supervisor.lease_worker(task.id, worker.id).await.is_ok());
}

#[tokio::test]
async fn an_unconfirmed_followup_keeps_an_unknown_effect_blocked() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor.mark_effect_started(task.id, worker.id).unwrap();
    supervisor
        .finish_subtask(
            task.id,
            worker.id,
            WorkerOutcome::Blocked {
                reason: BlockReason::UnresolvedEffect,
            },
        )
        .unwrap();
    drop(lease);
    let updated = supervisor.append_input(task.id, body()).unwrap();
    assert_eq!(updated.state, OpsTaskState::Blocked);
    assert_eq!(
        updated.subtask(worker.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert_eq!(
        updated.subtask(worker.id).unwrap().reason,
        Some(BlockReason::UnresolvedEffect)
    );
    assert_eq!(
        supervisor.retry_failed(task.id).unwrap().state,
        OpsTaskState::Blocked
    );
}

#[tokio::test]
async fn read_only_tasks_cannot_record_a_modification_intent() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let mut request = draft(&[host]);
    request.scope.can_modify = false;
    let task = supervisor.create(request).unwrap();
    let worker = sub(&supervisor, &task, host);
    let _lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    assert!(matches!(
        supervisor.mark_effect_started(task.id, worker.id),
        Err(OpsError::Forbidden)
    ));
}

#[tokio::test]
async fn startup_recovers_running_work_and_requires_a_fresh_approval_decision() {
    let (_, store, supervisor) = setup(2);
    let hosts = [HostId::new(), HostId::new()];
    let task = supervisor.create(draft(&hosts)).unwrap();
    let a = sub(&supervisor, &task, hosts[0]);
    let b = sub(&supervisor, &task, hosts[1]);
    let mut persisted = supervisor.get(task.id).unwrap();
    persisted.subtasks[0].state = OpsTaskState::Running;
    persisted.subtasks[0].uncertain_effect = true;
    persisted.subtasks[1].state = OpsTaskState::WaitingApproval;
    persisted.state = OpsTaskState::Running;
    let revision = persisted.revision;
    store.save(persisted, revision).unwrap();
    let recovered = OpsSupervisor::new(store, OpsConfig::default())
        .unwrap()
        .get(task.id)
        .unwrap();
    assert_eq!(
        recovered.subtask(a.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert!(recovered.subtask(a.id).unwrap().uncertain_effect);
    assert_eq!(
        recovered.subtask(b.id).unwrap().state,
        OpsTaskState::Blocked
    );
    assert_eq!(
        recovered.subtask(b.id).unwrap().reason,
        Some(BlockReason::UserDecision)
    );
}

#[tokio::test]
async fn shutdown_and_cancel_signal_workers_and_persist_the_state() {
    let (_, _, supervisor) = setup(1);
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    let lease = supervisor.lease_worker(task.id, worker.id).await.unwrap();
    supervisor
        .set_waiting_approval(task.id, worker.id, true)
        .unwrap();
    supervisor
        .set_waiting_approval(task.id, worker.id, false)
        .unwrap();
    supervisor.cancel(task.id).unwrap();
    assert!(lease.cancellation.is_cancelled());
    assert_eq!(
        supervisor.get(task.id).unwrap().state,
        OpsTaskState::Cancelled
    );
    drop(lease);
    assert_eq!(
        supervisor.get(task.id).unwrap().state,
        OpsTaskState::Cancelled
    );
    supervisor.shutdown().unwrap();
    assert!(matches!(
        supervisor.create(draft(&[host])),
        Err(OpsError::Closed)
    ));
}

#[test]
fn pages_replay_without_losing_older_tasks_or_revisions() {
    let (journal, _, supervisor) = setup(1);
    let task = supervisor.create(draft(&[HostId::new()])).unwrap();
    let mut snapshot = task.clone();
    for revision in 2..=520 {
        snapshot.revision = revision;
        journal
            .append(NewRecord {
                kind: KIND_TASK_EVENT.into(),
                session_id: Some(task.conversation.clone()),
                payload: serde_json::json!({"version":1,"task":snapshot}),
            })
            .unwrap();
    }
    let rebuilt = JournalTaskStore::new(journal).unwrap();
    assert_eq!(rebuilt.get(task.id).unwrap().revision, 520);
    assert_eq!(rebuilt.list().unwrap().len(), 1);
}

struct FailingJournal {
    inner: SqliteJournal,
    fail: AtomicBool,
}
impl JournalStore for FailingJournal {
    fn append_private(
        &self,
        record: NewRecord,
        session: &str,
        reference: &str,
        sealed: &[u8],
    ) -> Result<(), JournalError> {
        self.inner
            .append_private(record, session, reference, sealed)
    }
    fn private_body(&self, session: &str, reference: &str) -> Result<Vec<u8>, JournalError> {
        self.inner.private_body(session, reference)
    }
    fn transaction(
        &self,
        action: &mut dyn FnMut() -> Result<(), JournalError>,
    ) -> Result<(), JournalError> {
        self.inner.transaction(action)
    }
    fn sync_receipt(&self, space: &str) -> Result<serde_json::Value, JournalError> {
        self.inner.sync_receipt(space)
    }
    fn set_sync_receipt(&self, space: &str, value: &serde_json::Value) -> Result<(), JournalError> {
        self.inner.set_sync_receipt(space, value)
    }
    fn append(&self, record: NewRecord) -> Result<i64, JournalError> {
        if self.fail.load(Ordering::Acquire) {
            Err(JournalError::Io("storage unavailable".into()))
        } else {
            self.inner.append(record)
        }
    }
    fn import(&self, ts: i64, record: NewRecord) -> Result<i64, JournalError> {
        self.inner.import(ts, record)
    }
    fn read(&self, query: &JournalQuery) -> Result<Vec<JournalRecord>, JournalError> {
        self.inner.read(query)
    }
    fn delete_session(&self, session: &str, prefix: &str) -> Result<usize, JournalError> {
        self.inner.delete_session(session, prefix)
    }
}

#[tokio::test]
async fn storage_failure_prevents_dispatch_and_closes_the_supervisor() {
    let journal = Arc::new(FailingJournal {
        inner: SqliteJournal::open_in_memory().unwrap(),
        fail: AtomicBool::new(false),
    });
    let store: Arc<dyn TaskStore> = Arc::new(JournalTaskStore::new(journal.clone()).unwrap());
    let supervisor = Arc::new(OpsSupervisor::new(store.clone(), OpsConfig::default()).unwrap());
    let host = HostId::new();
    let task = supervisor.create(draft(&[host])).unwrap();
    let worker = sub(&supervisor, &task, host);
    journal.fail.store(true, Ordering::Release);
    assert!(matches!(
        supervisor.lease_worker(task.id, worker.id).await,
        Err(OpsError::Journal(_))
    ));
    assert_eq!(store.get(task.id).unwrap().state, OpsTaskState::Queued);
    assert!(matches!(
        supervisor.create(draft(&[host])),
        Err(OpsError::Closed)
    ));
}

#[tokio::test]
async fn private_storage_failure_cancels_active_and_waiting_workers_globally() {
    let (_, _, supervisor) = setup(1);
    let hosts = [HostId::new(), HostId::new()];
    let first = supervisor.create(draft(&[hosts[0]])).unwrap();
    let second = supervisor.create(draft(&[hosts[1]])).unwrap();
    let first_sub = sub(&supervisor, &first, hosts[0]);
    let second_sub = sub(&supervisor, &second, hosts[1]);
    let lease = supervisor
        .lease_worker(first.id, first_sub.id)
        .await
        .unwrap();
    let pending = supervisor.lease_worker(second.id, second_sub.id);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    let mut changes = supervisor.subscribe();
    supervisor.halt_on_storage_failure();
    assert!(supervisor.is_closed());
    assert!(lease.cancellation.is_cancelled());
    assert!(changes.has_changed().unwrap());
    changes.changed().await.unwrap();
    assert!(matches!(pending.await, Err(OpsError::Closed)));
    assert!(matches!(
        supervisor.create(draft(&[HostId::new()])),
        Err(OpsError::Closed)
    ));
    supervisor.halt_on_storage_failure();
    assert!(changes.has_changed().unwrap());
    drop(lease);
    assert_eq!(
        supervisor
            .get(first.id)
            .unwrap()
            .subtask(first_sub.id)
            .unwrap()
            .state,
        OpsTaskState::Interrupted
    );
    assert_eq!(
        supervisor
            .get(second.id)
            .unwrap()
            .subtask(second_sub.id)
            .unwrap()
            .attempts,
        0
    );
}

#[test]
fn task_metadata_contains_references_and_no_private_message_text() {
    let (journal, _, supervisor) = setup(1);
    let task = supervisor.create(draft(&[HostId::new()])).unwrap();
    let events = journal
        .read(&JournalQuery {
            kind: Some(KIND_TASK_EVENT.into()),
            ..JournalQuery::default()
        })
        .unwrap();
    let payload = serde_json::to_string(&events[0].payload).unwrap();
    assert!(payload.contains(&task.inputs[0].reference.to_string()));
    assert!(!payload.contains("\"prompt\""));
    assert!(!payload.contains("\"text\""));
    assert!(!payload.contains("\"checkpoint_body\""));
}
