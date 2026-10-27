use crate::ops::sync::{self, SyncRow};
use crate::state::{AppState, Location};
use std::path::PathBuf;
use tauri::State;

/// Recursive diff of two folders (design screen 1g). `by_content` compares
/// bytes and is honored only when both sides are local.
#[tauri::command]
pub async fn sync_compare(
    state: State<'_, AppState>,
    left: Location,
    left_dir: PathBuf,
    right: Location,
    right_dir: PathBuf,
    by_content: bool,
) -> Result<Vec<SyncRow>, String> {
    let left_conn = state.resolve(&left)?;
    let right_conn = state.resolve(&right)?;
    tauri::async_runtime::spawn_blocking(move || {
        sync::compare(
            left_conn.as_deref(),
            &left_dir,
            right_conn.as_deref(),
            &right_dir,
            by_content,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
