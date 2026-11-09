//! Snippet commands. Sending a snippet to a terminal is `session_write` from the WebView.

use rove_vault::{Snippet, SnippetDraft, SnippetId};
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

fn snippet_id(id: &str) -> Result<SnippetId, CommandError> {
    Ok(id.parse()?)
}

/// Lists snippets.
#[tauri::command]
pub fn snippet_list(state: State<'_, AppState>) -> Result<Vec<Snippet>, CommandError> {
    Ok(state.vault.list_snippets()?)
}

/// Adds a snippet.
#[tauri::command]
pub fn snippet_create(
    state: State<'_, AppState>,
    draft: SnippetDraft,
) -> Result<Snippet, CommandError> {
    Ok(state.vault.create_snippet(draft)?)
}

/// Replaces a snippet.
#[tauri::command]
pub fn snippet_update(
    state: State<'_, AppState>,
    id: String,
    draft: SnippetDraft,
) -> Result<Snippet, CommandError> {
    Ok(state.vault.update_snippet(snippet_id(&id)?, draft)?)
}

/// Deletes a snippet.
#[tauri::command]
pub fn snippet_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.vault.delete_snippet(snippet_id(&id)?)?)
}
