//! Workspace layout snapshots for session restore. The WebView owns the snapshot's shape; the
//! backend stores it as the latest `ui/workspace` journal record.

use rove_journal::{JournalQuery, NewRecord};
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

/// Journal kind of workspace snapshots.
const KIND: &str = "ui/workspace";
/// Largest accepted snapshot, serialized.
const MAX_SNAPSHOT: usize = 256 * 1024;

fn journal_error(error: rove_journal::JournalError) -> CommandError {
    CommandError::new("journal.error", error.to_string())
}

/// Records `snapshot`, an object describing the open tabs and their panes. It holds no secrets:
/// panes refer to saved hosts by id.
#[tauri::command]
pub fn workspace_save(
    state: State<'_, AppState>,
    snapshot: serde_json::Value,
) -> Result<(), CommandError> {
    if !snapshot.is_object() {
        return Err(CommandError::new(
            "input.invalid",
            "snapshot must be an object",
        ));
    }
    if snapshot.to_string().len() > MAX_SNAPSHOT {
        return Err(CommandError::new("input.invalid", "snapshot too large"));
    }
    state
        .journal
        .append(NewRecord {
            kind: KIND.into(),
            session_id: None,
            payload: snapshot,
        })
        .map_err(journal_error)?;
    Ok(())
}

/// Returns the latest snapshot, or `None` when none was recorded.
#[tauri::command]
pub fn workspace_load(
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, CommandError> {
    let latest = state
        .journal
        .read(&JournalQuery {
            kind: Some(KIND.into()),
            limit: Some(1),
            newest_first: true,
            ..Default::default()
        })
        .map_err(journal_error)?;
    Ok(latest.into_iter().next().map(|record| record.payload))
}
