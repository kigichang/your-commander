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

#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RenameItem {
    pub path: PathBuf,
    pub new_name: String,
}

/// Multi-rename (screen 1f): validated up front, then applied in two phases
/// through temporary names so renames that swap or shift names (A→B, B→A)
/// cannot trample each other. Returns the number of entries renamed.
#[tauri::command]
pub async fn batch_rename(
    state: State<'_, AppState>,
    location: Location,
    items: Vec<RenameItem>,
) -> Result<usize, String> {
    let conn = state.resolve(&location)?;
    blocking(move || {
        let items: Vec<RenameItem> = items
            .into_iter()
            .filter(|i| {
                i.path.file_name().map(|n| n.to_string_lossy() != i.new_name.as_str())
                    != Some(false)
            })
            .collect();
        let mut finals = std::collections::HashSet::new();
        for item in &items {
            if item.new_name.is_empty() || item.new_name.contains('/') {
                return Err(format!("Invalid name: \"{}\"", item.new_name));
            }
            let parent = item
                .path
                .parent()
                .ok_or_else(|| format!("{}: no parent", item.path.display()))?;
            if !finals.insert(parent.join(&item.new_name)) {
                return Err(format!("Duplicate name: \"{}\"", item.new_name));
            }
        }
        // A final name may only exist on disk if that entry is itself being
        // renamed away (swap/shift); anything else is a collision.
        let sources: std::collections::HashSet<&PathBuf> =
            items.iter().map(|i| &i.path).collect();
        let occupied = |p: &PathBuf| -> bool {
            match &conn {
                None => p.exists() || std::fs::symlink_metadata(p).is_ok(),
                Some(c) => c.exists(p).is_some(),
            }
        };
        for target in &finals {
            if !sources.contains(target) && occupied(target) {
                return Err(format!(
                    "\"{}\" already exists",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
        let do_rename = |from: &PathBuf, to: &PathBuf| -> Result<(), String> {
            match &conn {
                None => std::fs::rename(from, to)
                    .map_err(|e| format!("{}: {e}", from.display())),
                Some(c) => c.rename(from, to),
            }
        };
        // Phase 1: everything out of the way under unique temp names.
        let mut temps = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let parent = item.path.parent().unwrap();
            let temp = parent.join(format!(".ycrn_{}_{i}", std::process::id()));
            do_rename(&item.path, &temp)?;
            temps.push(temp);
        }
        // Phase 2: temp names to the final ones.
        for (item, temp) in items.iter().zip(&temps) {
            let target = item.path.parent().unwrap().join(&item.new_name);
            do_rename(temp, &target)?;
        }
        Ok(items.len())
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
