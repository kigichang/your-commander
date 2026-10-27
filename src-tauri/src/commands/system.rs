use crate::fs::local::{self, VolumeInfo};
use crate::ops::archive;
use std::path::PathBuf;

#[tauri::command]
pub async fn volumes() -> Result<Vec<VolumeInfo>, String> {
    tauri::async_runtime::spawn_blocking(local::volumes)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn home_dir() -> PathBuf {
    local::home_dir()
}

/// Open a local file or folder with its default app.
#[tauri::command]
pub async fn open_path(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open::that(&path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// Select the file in a new Finder window.
#[tauri::command]
pub async fn reveal_in_finder(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(&path)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("open -R failed".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn is_archive(name: String) -> bool {
    archive::is_archive(&name)
}

/// macOS Finder tags of a file, via `mdls` (best effort; empty on failure).
#[tauri::command]
pub async fn file_tags(path: PathBuf) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new("/usr/bin/mdls")
            .args(["-raw", "-name", "kMDItemUserTags"])
            .arg(&path)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim() == "(null)" {
            return None;
        }
        // Output shape: (\n    Tag1,\n    Tag2\n)
        let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
        Some(
            inner
                .lines()
                .map(|l| l.trim().trim_end_matches(',').trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        )
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}
