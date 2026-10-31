use crate::fs::archive_fs;
use crate::state::{AppState, Location, Resolved};
use serde::Serialize;
use std::io::Read;
use std::path::PathBuf;
use tauri::ipc::Response;
use tauri::State;

#[derive(Serialize, Default)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Whether a path sits on a network mount (SMB/NFS/...). Quick View streams
/// video from local disks through the asset protocol but must fall back to
/// a Blob for network mounts, where chunked range reads starve the decoder.
#[tauri::command]
pub async fn is_network_path(path: PathBuf) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(crate::fs::local::is_network_volume(&path)))
        .await
        .map_err(|e| e.to_string())?
}

/// Video metadata for Quick View, local files only. `mdimport -t -d2` runs
/// the Spotlight importer directly, so it works on unindexed volumes too
/// (`mdls` only reads the index). Missing attributes come back as None.
#[tauri::command]
pub async fn media_info(path: PathBuf) -> Result<MediaInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("/usr/bin/mdimport")
            .args(["-t", "-d2"])
            .arg(&path)
            .output()
            .map_err(|e| e.to_string())?;
        // The attribute dump goes to stderr on some macOS versions.
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let num = |attr: &str| -> Option<f64> {
            text.lines().find_map(|line| {
                let rest = line.trim().strip_prefix(attr)?.trim_start();
                let val = rest.strip_prefix('=')?.trim().trim_end_matches(';').trim();
                val.trim_matches('"').parse::<f64>().ok()
            })
        };
        Ok(MediaInfo {
            duration: num("kMDItemDurationSeconds"),
            width: num("kMDItemPixelWidth").map(|v| v as u32),
            height: num("kMDItemPixelHeight").map(|v| v as u32),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Raw file bytes for the Quick View pane (PDF.js, images, text). Works for
/// local and SFTP paths alike; capped at `max_bytes`.
#[tauri::command]
pub async fn read_preview(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
    max_bytes: u64,
) -> Result<Response, String> {
    let resolved = state.resolve(&location)?;
    let bytes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        match resolved {
            Resolved::Local => {
                let mut file = std::fs::File::open(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let mut buf = Vec::new();
                file.by_ref()
                    .take(max_bytes)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(buf)
            }
            Resolved::Sftp(c) => c.read_limited(&path, max_bytes),
            Resolved::Archive(archive) => {
                archive_fs::read_entry(&archive, &path.to_string_lossy(), max_bytes)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Response::new(bytes))
}
