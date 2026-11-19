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
/// One JPEG frame; an 8K one at -q:v 4 stays far below this.
const POSTER_MAX: u64 = 64 * 1024 * 1024;

#[tauri::command]
pub async fn video_poster(path: PathBuf) -> Result<Response, String> {
    require_absolute(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let ffmpeg = crate::platform::find_ffmpeg().ok_or("ffmpeg not found")?;
        for seek in ["5", "0"] {
            let out = crate::proc::output_bounded(
                std::process::Command::new(&ffmpeg)
                    .args(["-v", "error"])
                    .args(crate::platform::FFMPEG_LOCAL_INPUT)
                    .args(["-ss", seek, "-i"])
                    .arg(&path)
                    .args(["-frames:v", "1", "-f", "image2", "-c:v", "mjpeg", "-q:v", "4", "pipe:1"]),
                crate::platform::PROBE_TIMEOUT,
                POSTER_MAX,
            )?;
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

/// The most `read_preview` returns whatever the caller asks for: the
/// frontend's largest request (audio from SFTP or an archive, played from
/// memory). The bytes are held in memory here and again in the webview.
const PREVIEW_MAX: u64 = 256 * 1024 * 1024;

/// Raw file bytes for the Quick View pane (PDF.js, images, text). Works for
/// local and SFTP paths alike; capped at `max_bytes`, and at `PREVIEW_MAX`.
#[tauri::command]
pub async fn read_preview(
    state: State<'_, AppState>,
    location: Location,
    path: PathBuf,
    max_bytes: u64,
) -> Result<Response, String> {
    let resolved = state.resolve(&location)?;
    let max_bytes = max_bytes.min(PREVIEW_MAX);
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
            require_absolute(path)?;
            // Regular files only: a FIFO would block this thread until
            // someone wrote to it, and a device never ends.
            let (mut file, _) = crate::ops::safe_path::open_regular(path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
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
/// canonical form ever matches.
fn asset_dir(dir: &Path) -> Result<PathBuf, String> {
    let home = dirs::home_dir().and_then(|h| std::fs::canonicalize(h).ok());
    asset_dir_with_home(dir, home.as_deref())
}

/// `asset_dir` with the home folder given. A grant is recursive, so some
/// folders are refused outright: the root and top-level folders (/Users,
/// /Volumes: every account or every mounted disk), and the home folder or
/// anything above it (~/.ssh, ~/Library and the rest would come along),
/// and inside the home folder its hidden top-level folders and ~/Library
/// themselves (keys, credentials, app data, keychains). A page there
/// renders without its neighbouring files.
fn asset_dir_with_home(dir: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    require_absolute(dir)?;
    let real = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !real.is_dir() {
        return Err(format!("{}: not a directory", dir.display()));
    }
    let refuse = || Err(format!("{}: too broad to open to asset://", real.display()));
    if real.parent().is_none_or(|p| p.parent().is_none()) {
        return refuse();
    }
    if home.is_some_and(|h| h.starts_with(&real)) {
        return refuse();
    }
    let private = home
        .and_then(|h| real.strip_prefix(h).ok())
        .and_then(|rel| rel.components().next())
        .is_some_and(|first| {
            let name = first.as_os_str().to_string_lossy();
            name.starts_with('.') || name == "Library"
        });
    if private {
        return refuse();
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::{asset_dir, asset_dir_with_home};
    use std::path::{Path, PathBuf};

    /// A fresh folder two levels or more below the root on every platform
    /// (Linux's temp dir, /tmp, is itself top-level).
    fn work(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("yc_asset_{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    #[test]
    fn local_preview_reads_only_regular_files_by_absolute_path() {
        use crate::state::Resolved;
        let dir = work("read");
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let read = |p: &Path| super::read_bytes(Resolved::Local, p, 3);
        assert_eq!(read(&dir.join("a.txt")).unwrap(), b"hel");
        assert!(read(Path::new("a.txt")).is_err());
        assert!(read(Path::new("/dev/zero")).is_err());
        let fifo = dir.join("fifo");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(read(&fifo).is_err());
    }

    #[test]
    fn asset_dir_refuses_private_folders_of_the_home() {
        let home = work("private");
        for dir in [".ssh", ".config/app", "Library/Keychains", "Documents/site"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        let allowed = |dir: &str| asset_dir_with_home(&home.join(dir), Some(&home)).is_ok();
        assert!(!allowed(".ssh"));
        assert!(!allowed(".config/app"));
        assert!(!allowed("Library/Keychains"));
        assert!(allowed("Documents/site"));
    }

    #[test]
    fn asset_dir_is_canonical() {
        let dir = work("canon");
        assert_eq!(asset_dir_with_home(&dir, None).unwrap(), dir);
        assert_eq!(asset_dir_with_home(&dir.join("."), None).unwrap(), dir);
    }

    #[test]
    fn asset_dir_refuses_root_files_and_relative_paths() {
        assert!(asset_dir(Path::new("/")).is_err());
        assert!(asset_dir(Path::new("/usr/..")).is_err());
        assert!(asset_dir(Path::new("relative/dir")).is_err());
        assert!(asset_dir(Path::new("/nonexistent/asset_dir")).is_err());
        let file = work("file").join("page.html");
        std::fs::write(&file, "x").unwrap();
        assert!(asset_dir(&file).is_err());
    }

    #[test]
    fn asset_dir_refuses_top_level_folders_and_home_and_above() {
        assert!(asset_dir_with_home(Path::new("/usr"), None).is_err());
        let home = work("home");
        let docs = home.join("Documents");
        std::fs::create_dir_all(&docs).unwrap();
        assert!(asset_dir_with_home(&home, Some(&home)).is_err(), "home itself");
        assert!(asset_dir_with_home(home.parent().unwrap(), Some(&home)).is_err(), "above home");
        assert_eq!(asset_dir_with_home(&docs, Some(&home)).unwrap(), docs, "inside home");
        // The real home folder, through the public entry point.
        if let Some(real_home) = dirs::home_dir() {
            assert!(asset_dir(&real_home).is_err());
        }
    }
}
