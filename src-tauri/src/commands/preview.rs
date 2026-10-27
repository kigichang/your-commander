use crate::state::{AppState, Location};
use std::io::Read;
use std::path::PathBuf;
use tauri::ipc::Response;
use tauri::State;

/// Raw file bytes for the Quick View pane (PDF.js, images, text). Works for
/// local and SFTP paths alike; capped at `max_bytes`.
#[tauri::command]
pub async fn read_preview(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
    max_bytes: u64,
) -> Result<Response, String> {
    let conn = state.resolve(&location)?;
    let bytes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        match conn {
            None => {
                let mut file = std::fs::File::open(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let mut buf = Vec::new();
                file.by_ref()
                    .take(max_bytes)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(buf)
            }
            Some(c) => c.read_limited(&path, max_bytes),
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Response::new(bytes))
}
