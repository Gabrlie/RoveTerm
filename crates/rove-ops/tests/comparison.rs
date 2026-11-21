//! Deterministic scheduler comparisons with fixed model calls and token units.
//! Elapsed time measures this scripted workload, not real LLM performance.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use rove_journal::{JournalStore, SqliteJournal};
use rove_ops::*;
use rove_vault::HostId;
use serde::Serialize;
use tokio::{sync::Semaphore, task::JoinHandle};

const WORKERS: usize = 4;
const MODEL_CALLS_PER_EXECUTION: usize = 2;
const TOKEN_UNITS_PER_MODEL_CALL: usize = 120;

struct ScriptedWork {
    host_executions: Vec<AtomicUsize>,
    fail_once: Option<usize>,
    dispatched: AtomicUsize,
    started: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    model_calls: AtomicUsize,
    human_goal_restatements: AtomicUsize,
    changed: tokio::sync::Notify,
    release: Semaphore,
}

impl ScriptedWork {
    fn new(hosts: usize, fail_once: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            host_executions: (0..hosts).map(|_| AtomicUsize::new(0)).collect(),
            fail_once,
            dispatched: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            model_calls: AtomicUsize::new(0),
            human_goal_restatements: AtomicUsize::new(0),
            changed: tokio::sync::Notify::new(),
            release: Semaphore::new(0),
        })
    }

    fn dispatch(&self) {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }

    fn restate_goal(&self) {
        self.human_goal_restatements.fetch_add(1, Ordering::SeqCst);
    }

    async fn wait_for(&self, counter: &AtomicUsize, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.changed.notified();
                if counter.load(Ordering::SeqCst) >= count {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("scripted workers did not reach their scheduling gate");
    }

    async fn execute(&self, host: usize, gated: bool) -> bool {
        let attempt = self.host_executions[host].fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        self.started.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
        if gated {
            self.release.acquire().await.unwrap().forget();
        }
        for _ in 0..MODEL_CALLS_PER_EXECUTION {
            self.model_calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        !(self.fail_once == Some(host) && attempt == 0)
    }

    fn metrics(&self, elapsed: Duration) -> Metrics {
        let model_calls = self.model_calls.load(Ordering::SeqCst);
        Metrics {
            host_executions: self
                .host_executions
                .iter()
                .map(|counter| counter.load(Ordering::SeqCst))
                .collect(),
            model_calls,
            fixed_token_units: model_calls * TOKEN_UNITS_PER_MODEL_CALL,
            human_goal_restatements: self.human_goal_restatements.load(Ordering::SeqCst),
            concurrency_peak: self.peak.load(Ordering::SeqCst),
            elapsed_micros: elapsed.as_micros(),
        }
    }
}

#[derive(Debug, Serialize)]
struct Metrics {
    host_executions: Vec<usize>,
    model_calls: usize,
    fixed_token_units: usize,
    human_goal_restatements: usize,
    concurrency_peak: usize,
    elapsed_micros: u128,
}

type Job = (OpsTaskId, OpsSubtaskId, usize);
type HostRun = JoinHandle<(usize, bool)>;

fn private_ref() -> PrivateRef {
    PrivateRef::new("ai-comparison", uuid::Uuid::now_v7().to_string()).unwrap()
}

fn supervisor() -> Arc<OpsSupervisor> {
    let journal: Arc<dyn JournalStore> = Arc::new(SqliteJournal::open_in_memory().unwrap());
    let store: Arc<dyn TaskStore> = Arc::new(JournalTaskStore::new(journal).unwrap());
    Arc::new(
        OpsSupervisor::new(
            store,
            OpsConfig {
                max_workers: WORKERS,
                ..OpsConfig::default()
            },
        )
        .unwrap(),
    )
}

fn task(supervisor: &OpsSupervisor, hosts: &[HostId], offset: usize) -> (OpsTask, Vec<Job>) {
    let task = supervisor
        .create(TaskDraft {
            request_id: OpsRequestId::new(uuid::Uuid::now_v7().to_string()).unwrap(),
            conversation: "ai-comparison".into(),
            source: TaskSource::User,
            input: private_ref(),
            acceptance: Some(private_ref()),
            scope: TaskScope {
                targets: hosts
                    .iter()
                    .map(|host| Target {
                        host_id: *host,
                        fingerprint: format!("target:{host}"),
                        input: None,
                    })
                    .collect(),
                can_modify: false,
            },
        })
        .unwrap();
    let jobs = hosts
        .iter()
        .enumerate()
        .map(|(index, host)| {
            let worker = supervisor
                .ensure_subtask(task.id, *host, &format!("target:{host}"))
                .unwrap();
            (task.id, worker.id, offset + index)
        })
        .collect();
    (task, jobs)
}

// Each delegate owns a separate budget; retrying a delegate repeats its whole host list.
fn legacy_delegate(work: Arc<ScriptedWork>, hosts: &[usize], gated: bool) -> Vec<HostRun> {
    let permits = Arc::new(Semaphore::new(WORKERS));
    hosts
        .iter()
        .copied()
        .map(|host| {
            let work = work.clone();
            let permits = permits.clone();
            tokio::spawn(async move {
                work.dispatch();
                let _permit = permits.acquire().await.unwrap();
                (host, work.execute(host, gated).await)
            })
        })
        .collect()
}

fn supervised(
    supervisor: Arc<OpsSupervisor>,
    work: Arc<ScriptedWork>,
    jobs: &[Job],
    gated: bool,
) -> Vec<HostRun> {
    jobs.iter()
        .copied()
        .map(|(task, subtask, host)| {
            let supervisor = supervisor.clone();
            let work = work.clone();
            tokio::spawn(async move {
                work.dispatch();
                let lease = supervisor.lease_worker(task, subtask).await.unwrap();
                let success = work.execute(host, gated).await;
                let outcome = if success {
                    WorkerOutcome::Completed {
                        evidence: private_ref(),
                    }
                } else {
                    WorkerOutcome::Failed {
                        code: "host.unreachable".into(),
                    }
                };
                supervisor.finish_subtask(task, subtask, outcome).unwrap();
                drop(lease);
                (host, success)
            })
        })
        .collect()
}

async fn collect(runs: Vec<HostRun>) -> Vec<(usize, bool)> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut results = Vec::with_capacity(runs.len());
        for run in runs {
            results.push(run.await.unwrap());
        }
        results
    })
    .await
    .expect("scripted host work did not finish")
}

fn report(comparison: &str, legacy: &Metrics, supervisor: &Metrics) {
    println!(
        "{}",
        serde_json::json!({
            "comparison": comparison,
            "workload": "scripted_mock",
            "measurement": "scheduler behavior; not real LLM performance",
            "model_calls_per_host_execution": MODEL_CALLS_PER_EXECUTION,
            "fixed_token_units_per_model_call": TOKEN_UNITS_PER_MODEL_CALL,
            "legacy": legacy,
            "supervisor": supervisor,
        })
    );
}

#[tokio::test]
async fn global_budget_bounds_two_delegates_to_four_workers() {
    let legacy = ScriptedWork::new(8, None);
    let start = Instant::now();
    let mut runs = legacy_delegate(legacy.clone(), &[0, 1, 2, 3], true);
    runs.extend(legacy_delegate(legacy.clone(), &[4, 5, 6, 7], true));
    legacy.wait_for(&legacy.started, 8).await;
    assert_eq!(legacy.active.load(Ordering::SeqCst), 8);
    legacy.release.add_permits(8);
    assert!(collect(runs).await.iter().all(|(_, success)| *success));
    let legacy_metrics = legacy.metrics(start.elapsed());

    let supervisor = supervisor();
    let work = ScriptedWork::new(8, None);
    let hosts: Vec<_> = (0..8).map(|_| HostId::new()).collect();
    let (first, mut jobs) = task(&supervisor, &hosts[..4], 0);
    let (second, more_jobs) = task(&supervisor, &hosts[4..], 4);
    jobs.extend(more_jobs);
    let start = Instant::now();
    let runs = supervised(supervisor.clone(), work.clone(), &jobs, true);
    work.wait_for(&work.dispatched, 8).await;
    work.wait_for(&work.started, WORKERS).await;
    assert_eq!(work.active.load(Ordering::SeqCst), WORKERS);
    assert_eq!(work.started.load(Ordering::SeqCst), WORKERS);
    work.release.add_permits(8);
    assert!(collect(runs).await.iter().all(|(_, success)| *success));
    let supervisor_metrics = work.metrics(start.elapsed());

    assert_eq!(
        supervisor.get(first.id).unwrap().state,
        OpsTaskState::Completed
    );
    assert_eq!(
        supervisor.get(second.id).unwrap().state,
        OpsTaskState::Completed
    );
    assert_eq!(legacy_metrics.host_executions, vec![1; 8]);
    assert_eq!(supervisor_metrics.host_executions, vec![1; 8]);
    assert_eq!(legacy_metrics.concurrency_peak, 8);
    assert_eq!(supervisor_metrics.concurrency_peak, WORKERS);
    assert_eq!(legacy_metrics.model_calls, 16);
    assert_eq!(supervisor_metrics.model_calls, 16);
    assert_eq!(legacy_metrics.fixed_token_units, 1920);
    assert_eq!(supervisor_metrics.fixed_token_units, 1920);
    report("global_worker_budget", &legacy_metrics, &supervisor_metrics);
}

#[tokio::test]
async fn selective_retry_preserves_successful_hosts_and_their_evidence() {
    let legacy = ScriptedWork::new(3, Some(2));
    let start = Instant::now();
    let first = collect(legacy_delegate(legacy.clone(), &[0, 1, 2], false)).await;
    assert_eq!(first, vec![(0, true), (1, true), (2, false)]);
    legacy.restate_goal();
    let retry = collect(legacy_delegate(legacy.clone(), &[0, 1, 2], false)).await;
    assert!(retry.iter().all(|(_, success)| *success));
    let legacy_metrics = legacy.metrics(start.elapsed());

    let supervisor = supervisor();
    let work = ScriptedWork::new(3, Some(2));
    let hosts: Vec<_> = (0..3).map(|_| HostId::new()).collect();
    let (task, jobs) = task(&supervisor, &hosts, 0);
    let start = Instant::now();
    let first = collect(supervised(supervisor.clone(), work.clone(), &jobs, false)).await;
    assert_eq!(first, vec![(0, true), (1, true), (2, false)]);
    let failed = supervisor.get(task.id).unwrap();
    assert_eq!(failed.state, OpsTaskState::Failed);
    let retried = supervisor.retry_failed(task.id).unwrap();
    for (_, subtask, _) in &jobs[..2] {
        assert_eq!(
            retried.subtask(*subtask).unwrap(),
            failed.subtask(*subtask).unwrap()
        );
    }
    let queued: Vec<_> = jobs
        .iter()
        .copied()
        .filter(|(_, subtask, _)| retried.subtask(*subtask).unwrap().state == OpsTaskState::Queued)
        .collect();
    assert_eq!(queued, vec![jobs[2]]);
    assert_eq!(
        collect(supervised(supervisor.clone(), work.clone(), &queued, false)).await,
        vec![(2, true)]
    );
    let supervisor_metrics = work.metrics(start.elapsed());
    let completed = supervisor.get(task.id).unwrap();

    assert_eq!(completed.state, OpsTaskState::Completed);
    assert_eq!(legacy_metrics.host_executions, vec![2, 2, 2]);
    assert_eq!(supervisor_metrics.host_executions, vec![1, 1, 2]);
    assert_eq!(legacy_metrics.model_calls, 12);
    assert_eq!(supervisor_metrics.model_calls, 8);
    assert_eq!(legacy_metrics.fixed_token_units, 1440);
    assert_eq!(supervisor_metrics.fixed_token_units, 960);
    assert_eq!(legacy_metrics.human_goal_restatements, 1);
    assert_eq!(supervisor_metrics.human_goal_restatements, 0);
    assert_eq!(legacy_metrics.concurrency_peak, 3);
    assert_eq!(supervisor_metrics.concurrency_peak, 3);
    for (_, subtask, host) in &jobs {
        assert_eq!(
            completed.subtask(*subtask).unwrap().attempts,
            supervisor_metrics.host_executions[*host] as u32
        );
    }
    report(
        "selective_failed_host_retry",
        &legacy_metrics,
        &supervisor_metrics,
    );
}
