use crate::fs::local;
use crate::fs::provider::FsEntry;
use crate::ops::transfer;
use crate::state::{AppState, Location};
use std::path::PathBuf;
use tauri::State;

/// Run blocking file-system work off the async runtime's core threads.
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_dir(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
) -> Result<Vec<FsEntry>, String> {
    let conn = state.resolve(&location)?;
    blocking(move || match conn {
        None => local::list_dir(&path).map_err(|e| format!("{}: {e}", path.display())),
        Some(c) => c.list(&path).map_err(|e| format!("{}: {e}", path.display())),
    })
    .await
}

#[tauri::command]
pub async fn mkdir(
    state: State<'_, AppState>,
    location: Location,
    dir: PathBuf,
    name: String,
) -> Result<(), String> {
    let conn = state.resolve(&location)?;
    blocking(move || match conn {
        None => transfer::new_folder(&dir, &name),
        Some(c) => {
            let dest = dir.join(&name);
            if c.exists(&dest).is_some() {
                return Err(format!("\"{name}\" already exists"));
            }
            c.mkdir(&dest)
        }
    })
    .await
}

#[tauri::command]
pub async fn rename(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
    new_name: String,
) -> Result<(), String> {
    let conn = state.resolve(&location)?;
    blocking(move || match conn {
        None => transfer::rename(&path, &new_name),
        Some(c) => {
            let Some(parent) = path.parent() else {
                return Err("no parent".to_string());
            };
            let dest = parent.join(&new_name);
            if c.exists(&dest).is_some() {
                return Err(format!("\"{new_name}\" already exists"));
            }
            c.rename(&path, &dest)
        }
    })
    .await
}

#[tauri::command]
pub async fn exists(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
) -> Result<bool, String> {
    let conn = state.resolve(&location)?;
    blocking(move || {
        Ok(match conn {
            None => path.exists(),
            Some(c) => c.exists(&path).is_some(),
        })
    })
    .await
}
