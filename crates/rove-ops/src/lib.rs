//! Durable host-scoped task metadata and a shared, bounded worker scheduler.
//! The application supplies model workers; every remote operation remains behind its Policy.

mod model;
mod store;
mod supervisor;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::Deserialize;
use std::sync::Arc;

pub use model::*;
pub use store::{JournalTaskStore, KIND_TASK_EVENT, TaskStore};
pub use supervisor::{OpsSupervisor, TaskRunGuard, WorkerLease};

#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error(transparent)]
    Journal(#[from] rove_journal::JournalError),
    #[error("invalid operations input: {0}")]
    Invalid(String),
    #[error("task {0} not found")]
    TaskNotFound(OpsTaskId),
    #[error("subtask {0} not found")]
    SubtaskNotFound(OpsSubtaskId),
    #[error("task revision changed")]
    Conflict,
    #[error("source request was reused with a different scope")]
    RequestConflict,
    #[error("operation exceeds the task's authorized scope")]
    Forbidden,
    #[error("task or worker is already running")]
    Busy,
    #[error("operations supervisor is closed")]
    Closed,
    #[error("an uncertain remote effect must be verified first")]
    UncertainEffect,
    #[error("completed work needs a new explicit input before it can run again")]
    NoNewInput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpsConfig {
    pub max_workers: usize,
    pub max_subtasks: usize,
    pub max_inputs: usize,
}

impl Default for OpsConfig {
    fn default() -> Self {
        Self {
            max_workers: 4,
            max_subtasks: 16,
            max_inputs: 1024,
        }
    }
}

impl OpsConfig {
    pub fn validate(&self) -> Result<(), OpsError> {
        if !(1..=64).contains(&self.max_workers)
            || !(1..=1024).contains(&self.max_subtasks)
            || !(1..=16384).contains(&self.max_inputs)
        {
            return Err(OpsError::Invalid(
                "worker, subtask or input limit is out of range".into(),
            ));
        }
        Ok(())
    }
}

pub struct OpsPlugin;

impl Plugin for OpsPlugin {
    fn name(&self) -> &'static str {
        "ops"
    }
    fn requires(&self) -> &'static [&'static str] {
        &["journal"]
    }
    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: OpsConfig = config.parse()?;
        let journal = ctx.require::<dyn rove_journal::JournalStore>()?;
        let store: Arc<dyn TaskStore> = Arc::new(
            JournalTaskStore::new(journal).map_err(|error| KernelError::plugin("ops", error))?,
        );
        let supervisor = Arc::new(
            OpsSupervisor::new(store.clone(), config)
                .map_err(|error| KernelError::plugin("ops", error))?,
        );
        let shutdown = supervisor.clone();
        Ok(vec![
            ctx.provide::<dyn TaskStore>(store)?,
            ctx.provide(supervisor)?,
            Disposer::new(move || {
                if let Err(error) = shutdown.shutdown() {
                    tracing::error!(%error, "operations shutdown could not be recorded");
                }
            }),
        ])
    }
}
