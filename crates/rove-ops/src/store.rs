use std::{collections::BTreeMap, sync::Arc};

use parking_lot::Mutex;
use rove_journal::{JournalError, JournalQuery, JournalStore, NewRecord};
use serde::{Deserialize, Serialize};

use crate::{OpsError, OpsTask, OpsTaskId};

pub const KIND_TASK_EVENT: &str = "ops/task_event";
const EVENT_VERSION: u32 = 1;
const PAGE_SIZE: u32 = 512;

/// Task metadata storage. Implementations enforce request uniqueness and revision CAS.
pub trait TaskStore: Send + Sync {
    fn list(&self) -> Result<Vec<OpsTask>, OpsError>;
    fn get(&self, id: OpsTaskId) -> Result<OpsTask, OpsError>;
    fn create(&self, task: OpsTask) -> Result<OpsTask, OpsError>;
    fn save(&self, task: OpsTask, expected_revision: u64) -> Result<OpsTask, OpsError>;
}

#[derive(Serialize, Deserialize)]
struct Event {
    version: u32,
    task: OpsTask,
}

/// Rebuilds task indexes from local Journal events. Bodies remain in the private audit store.
/// No execution authority is imported by the conversation synchronization projection.
pub struct JournalTaskStore {
    journal: Arc<dyn JournalStore>,
    serial: Mutex<()>,
}

impl JournalTaskStore {
    pub fn new(journal: Arc<dyn JournalStore>) -> Result<Self, OpsError> {
        let store = Self {
            journal,
            serial: Mutex::new(()),
        };
        store.read_all()?;
        Ok(store)
    }

    fn read_all(&self) -> Result<BTreeMap<OpsTaskId, OpsTask>, JournalError> {
        let mut tasks = BTreeMap::<OpsTaskId, OpsTask>::new();
        let mut after = None;
        loop {
            let records = self.journal.read(&JournalQuery {
                kind: Some(KIND_TASK_EVENT.into()),
                after_seq: after,
                limit: Some(PAGE_SIZE),
                ..JournalQuery::default()
            })?;
            if records.is_empty() {
                break;
            }
            for record in &records {
                let event: Event = serde_json::from_value(record.payload.clone())?;
                if event.version != EVENT_VERSION {
                    return Err(JournalError::Io(
                        "unsupported operations event version".into(),
                    ));
                }
                event
                    .task
                    .validate()
                    .map_err(|error| JournalError::Io(error.to_string()))?;
                if let Some(old) = tasks.get(&event.task.id) {
                    if event.task.revision != old.revision + 1
                        || old.scope != event.task.scope
                        || old.request_id != event.task.request_id
                        || old.conversation != event.task.conversation
                        || old.source != event.task.source
                    {
                        return Err(JournalError::Io(
                            "inconsistent operations event revision or scope".into(),
                        ));
                    }
                } else if event.task.revision != 1 {
                    return Err(JournalError::Io(
                        "operations task starts at an invalid revision".into(),
                    ));
                }
                tasks.insert(event.task.id, event.task);
                after = Some(record.seq);
            }
            if records.len() < PAGE_SIZE as usize {
                break;
            }
        }
        Ok(tasks)
    }

    fn append(&self, task: &OpsTask) -> Result<(), JournalError> {
        self.journal.append(NewRecord {
            kind: KIND_TASK_EVENT.into(),
            session_id: Some(task.conversation.clone()),
            payload: serde_json::to_value(Event {
                version: EVENT_VERSION,
                task: task.clone(),
            })?,
        })?;
        Ok(())
    }
}

impl TaskStore for JournalTaskStore {
    fn list(&self) -> Result<Vec<OpsTask>, OpsError> {
        Ok(self.read_all()?.into_values().collect())
    }
    fn get(&self, id: OpsTaskId) -> Result<OpsTask, OpsError> {
        self.read_all()?
            .remove(&id)
            .ok_or(OpsError::TaskNotFound(id))
    }
    fn create(&self, mut task: OpsTask) -> Result<OpsTask, OpsError> {
        task.validate()?;
        let _serial = self.serial.lock();
        let mut result = None;
        self.journal.transaction(&mut || {
            let tasks = self.read_all()?;
            if let Some(existing) = tasks.values().find(|old| old.request_id == task.request_id) {
                result = Some(
                    if existing.scope.same_authority(&task.scope)
                        && existing.conversation == task.conversation
                        && existing.source == task.source
                    {
                        Ok(existing.clone())
                    } else {
                        Err(OpsError::RequestConflict)
                    },
                );
                return Ok(());
            }
            if tasks.contains_key(&task.id) {
                result = Some(Err(OpsError::Conflict));
                return Ok(());
            }
            task.revision = 1;
            self.append(&task)?;
            result = Some(Ok(task.clone()));
            Ok(())
        })?;
        result.ok_or_else(|| OpsError::Invalid("task transaction returned no result".into()))?
    }
    fn save(&self, mut task: OpsTask, expected_revision: u64) -> Result<OpsTask, OpsError> {
        task.validate()?;
        let _serial = self.serial.lock();
        let mut result = None;
        self.journal.transaction(&mut || {
            let Some(old) = self.read_all()?.remove(&task.id) else {
                result = Some(Err(OpsError::TaskNotFound(task.id)));
                return Ok(());
            };
            if old.revision != expected_revision {
                result = Some(Err(OpsError::Conflict));
            } else if old.scope != task.scope
                || old.request_id != task.request_id
                || old.conversation != task.conversation
                || old.source != task.source
            {
                result = Some(Err(OpsError::Forbidden));
            } else {
                task.revision = old
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| JournalError::Io("operations revision overflow".into()))?;
                self.append(&task)?;
                result = Some(Ok(task.clone()));
            }
            Ok(())
        })?;
        result.ok_or_else(|| OpsError::Invalid("task transaction returned no result".into()))?
    }
}
