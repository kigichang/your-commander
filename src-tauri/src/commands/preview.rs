use crate::fs::archive_fs;
use crate::state::{AppState, Location, Resolved};
use std::io::Read;
use std::path::{Path, PathBuf};
use super::{blocking, require_absolute};
use tauri::{AppHandle, Manager, State};
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

/// Whether a local file is a cloud placeholder not downloaded yet (Google
/// Drive, iCloud…); Quick View then waits for the Download button.
#[tauri::command]
pub async fn cloud_placeholder(path: PathBuf) -> Result<bool, String> {
    require_absolute(&path)?;
    blocking(move || Ok(crate::platform::is_cloud_placeholder(&path))).await
}

/// Fetch a cloud placeholder's contents to disk; resolves once it's there.
#[tauri::command]
pub async fn cloud_download(path: PathBuf) -> Result<(), String> {
    require_absolute(&path)?;
    blocking(move || crate::platform::download_cloud_file(&path)).await
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
    let bytes = blocking(move || read_bytes(resolved, &path, max_bytes)).await?;
    Ok(Response::new(bytes))
}

/// Up to `max_bytes` of a file wherever it lives: local disk, SFTP or
/// inside an archive.
pub(crate) fn read_bytes(
    resolved: Resolved,
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    match resolved {
        Resolved::Local => {
            let mut file =
                std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut buf = Vec::new();
            file.by_ref()
                .take(max_bytes)
                .read_to_end(&mut buf)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(buf)
        }
        Resolved::Sftp(c) => c.read_limited(path, max_bytes),
        Resolved::Archive(archive) => {
            archive_fs::read_entry(&archive, &path.to_string_lossy(), max_bytes)
        }
    }
}

/// The whole file, or an error when it is bigger than `max_bytes` (a
/// truncated workbook or document would not parse anyway).
pub(crate) fn read_whole(
    resolved: Resolved,
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let buf = read_bytes(resolved, path, max_bytes + 1)?;
    if buf.len() as u64 > max_bytes {
        return Err(format!(
            "{}: larger than {} MB, too big to preview",
            path.display(),
            max_bytes / (1024 * 1024)
        ));
    }
    Ok(buf)
}

/// Opens `dir` and everything under it to the asset protocol, whose static
/// scope is empty: the HTML preview loads the files next to the page it
/// shows (images, stylesheets, fonts), and the rest of the disk stays out
/// of reach of asset:// URLs. Grants last for the session.
#[tauri::command]
pub async fn allow_asset_dir(app: AppHandle, dir: PathBuf) -> Result<(), String> {
    let real = blocking(move || asset_dir(&dir)).await?;
    app.asset_protocol_scope()
        .allow_directory(&real, true)
        .map_err(|e| e.to_string())
}

/// The directory as the asset protocol matches it. Requests are resolved
/// (symlinks, /tmp → /private/tmp) before the scope check, so only the
/// canonical form ever matches. The root is refused: granting it would
/// undo the empty scope.
fn asset_dir(dir: &Path) -> Result<PathBuf, String> {
    require_absolute(dir)?;
    let real = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !real.is_dir() {
        return Err(format!("{}: not a directory", dir.display()));
    }
    if real.parent().is_none() {
        return Err("refusing to open the filesystem root to asset://".into());
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::asset_dir;
    use std::path::Path;

    #[test]
    fn asset_dir_is_canonical() {
        let tmp = std::env::temp_dir();
        assert_eq!(asset_dir(&tmp).unwrap(), std::fs::canonicalize(&tmp).unwrap());
        let dotted = tmp.join(".");
        assert_eq!(asset_dir(&dotted).unwrap(), std::fs::canonicalize(&tmp).unwrap());
    }

    #[test]
    fn asset_dir_refuses_root_files_and_relative_paths() {
        assert!(asset_dir(Path::new("/")).is_err());
        assert!(asset_dir(Path::new("/usr/..")).is_err());
        assert!(asset_dir(Path::new("relative/dir")).is_err());
        assert!(asset_dir(Path::new("/nonexistent/asset_dir")).is_err());
        let file = std::env::temp_dir().join("asset_dir_test_file.html");
        std::fs::write(&file, "x").unwrap();
        assert!(asset_dir(&file).is_err());
        let _ = std::fs::remove_file(&file);
    }
}
