use crate::fs::archive_fs;
use crate::state::{AppState, Location, Resolved};
use std::io::Read;
use std::path::PathBuf;
use super::{blocking, require_absolute};
use tauri::State;
use tauri::ipc::Response;

pub use crate::platform::MediaInfo;

/// Video metadata for Quick View, local files only. Missing attributes
/// come back as None.
#[tauri::command]
pub async fn media_info(path: PathBuf) -> Result<MediaInfo, String> {
    require_absolute(&path)?;
    blocking(move || crate::platform::media_info(&path)).await
}

/// One decoded frame as JPEG, for the static fallback when the webview's
/// decoder can't render a video (opt-in setting). A few seconds in to skip
/// black lead-ins; 0 when the file is shorter than that.
#[tauri::command]
pub async fn video_poster(path: PathBuf) -> Result<Response, String> {
    require_absolute(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let ffmpeg = crate::platform::find_ffmpeg().ok_or("ffmpeg not found")?;
        for seek in ["5", "0"] {
            let out = std::process::Command::new(&ffmpeg)
                .args(["-v", "error", "-ss", seek, "-i"])
                .arg(&path)
                .args(["-frames:v", "1", "-f", "image2", "-c:v", "mjpeg", "-q:v", "4", "pipe:1"])
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() && !out.stdout.is_empty() {
                return Ok(out.stdout);
            }
        }
        Err("could not extract a frame".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map(Response::new)
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
    let bytes = blocking(move || -> Result<Vec<u8>, String> {
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
    .await?;
    Ok(Response::new(bytes))
}
