use crate::fs::local::{self, VolumeInfo};
use crate::platform;
use crate::state::AppState;
use std::path::{Path, PathBuf};
use super::{blocking, media_input};
use tauri::Manager;

pub use crate::platform::OpenApp;

/// Open another main window — an independent workspace. Labels run
/// "main", "main-2", "main-3", … and share the "main*" capability.
/// Must run on the main thread (IPC commands and menu actions both do).
pub fn open_new_window(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let mut n = 2;
    while app.get_webview_window(&format!("main-{n}")).is_some() {
        n += 1;
    }
    let label = format!("main-{n}");
    let (w, h) = app
        .state::<AppState>()
        .config
        .lock()
        .unwrap()
        .window_size
        .unwrap_or((1150.0, 720.0));

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::default())
        .title("Your Commander")
        .inner_size(f64::from(w.max(800.0)), f64::from(h.max(500.0)))
        .min_inner_size(800.0, 500.0);
    // Overlay title bar with repositioned traffic lights; elsewhere the
    // window keeps its native decorations.
    #[cfg(target_os = "macos")]
    {
        use tauri::{LogicalPosition, TitleBarStyle};
        builder = builder
            .hidden_title(true)
            .title_bar_style(TitleBarStyle::Overlay)
            .traffic_light_position(LogicalPosition::new(16.0, 18.0));
    }
    // Cascade below the window the user is working in.
    if let Some(focused) = app
        .webview_windows()
        .values()
        .find(|win| win.is_focused().unwrap_or(false))
        && let (Ok(pos), Ok(scale)) = (focused.outer_position(), focused.scale_factor())
    {
        let p = pos.to_logical::<f64>(scale);
        builder = builder.position(p.x + 28.0, p.y + 28.0);
    }
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn new_window(app: tauri::AppHandle) -> Result<(), String> {
    open_new_window(&app)
}

/// Quit by closing every window: each one's CloseRequested handler still
/// runs (window size saved, fn-keys released), then the app exits.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    for window in app.webview_windows().values() {
        let _ = window.close();
    }
}

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

/// The user's Desktop folder (toolbar Desktop button): ~/Desktop on macOS;
/// on Linux the XDG user dir from user-dirs.dirs (localized names such as
/// ~/桌面 included), falling back to ~/Desktop.
#[tauri::command]
pub fn desktop_dir() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| local::home_dir().join("Desktop"))
}

/// Folder whose listing is the user's Trash (toolbar Trash button).
#[tauri::command]
pub fn trash_dir() -> Result<PathBuf, String> {
    platform::trash_dir()
}

/// "New … Here" in the blank-space context menu: open a terminal or editor
/// in `dir`. `tool` is "terminal", "vscode", or "zed".
#[tauri::command]
pub async fn open_in(tool: String, dir: PathBuf) -> Result<(), String> {
    blocking(move || platform::open_in(&tool, &dir)).await
}

#[derive(serde::Serialize)]
pub struct VideoTools {
    /// Installed ffmpeg-based player to hand undecodable files to.
    pub player: Option<String>,
    /// Whether ffmpeg is available for the repair (re-encode) action.
    pub ffmpeg: bool,
}

/// Which local helpers exist for videos the system decoder can't handle.
#[tauri::command]
pub async fn video_tools() -> VideoTools {
    tauri::async_runtime::spawn_blocking(|| VideoTools {
        player: platform::video_player(),
        ffmpeg: platform::find_ffmpeg().is_some(),
    })
    .await
    .unwrap_or(VideoTools { player: None, ffmpeg: false })
}

/// Source duration in seconds, parsed from ffmpeg's `-i` banner (the call
/// exits non-zero without an output file; "Duration:" is on stderr).
fn probe_duration(ffmpeg: &std::path::Path, src: &std::path::Path) -> Option<f64> {
    let out = std::process::Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(src)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let rest = &text[text.find("Duration: ")? + 10..];
    let stamp = rest.split(',').next()?.trim();
    let mut parts = stamp.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s).filter(|d| *d > 0.0)
}

/// The hidden work file `repair_video` writes next to `original`.
fn repair_work_file(original: &Path) -> Result<PathBuf, String> {
    let dir = original.parent().ok_or("no parent dir")?;
    let stem = original.file_stem().ok_or("no file name")?.to_string_lossy();
    Ok(dir.join(format!(".{stem}_fix.part.mp4")))
}

/// Re-encode a video the system decoder can't handle into a playable H.264
/// mp4 (hardware encode on macOS, libx264 elsewhere). Writes a hidden work
/// file next to the original and returns its path; finish_repair decides
/// what to keep. Progress fractions (0..1) stream over `on_progress`.
#[tauri::command]
pub async fn repair_video(
    path: PathBuf,
    on_progress: tauri::ipc::Channel<f32>,
) -> Result<PathBuf, String> {
    media_input(&path)?;
    let ffmpeg = platform::find_ffmpeg().ok_or("ffmpeg not found")?;
    let tmp = repair_work_file(&path)?;
    let src = path.clone();
    let out_path = tmp.clone();
    blocking(move || {
        let total = probe_duration(&ffmpeg, &src);
        let mut command = std::process::Command::new(ffmpeg);
        command
            .arg("-hide_banner")
            .args(["-v", "error", "-y", "-progress", "pipe:1", "-nostats", "-i"])
            .arg(&src)
            .args(platform::encode_args())
            .args(["-vf", "setsar=1"])
            .args(["-c:a", "copy", "-movflags", "+faststart"])
            .arg(&out_path);
        // One key=value block per encoded chunk; out_time_us (and the
        // misnamed out_time_ms) are both microseconds.
        let mut last = -1i32;
        let (status, stderr) = run_with_progress(command, |line| {
            let Some(total) = total else { return };
            let us = line
                .strip_prefix("out_time_us=")
                .or_else(|| line.strip_prefix("out_time_ms="));
            if let Some(us) = us.and_then(|v| v.trim().parse::<f64>().ok()) {
                let pct = ((us / 1_000_000.0) / total * 100.0).clamp(0.0, 99.0) as i32;
                if pct > last {
                    last = pct;
                    let _ = on_progress.send(pct as f32 / 100.0);
                }
            }
        })
        .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(out_path)
        } else {
            let _ = std::fs::remove_file(&out_path);
            let err = String::from_utf8_lossy(&stderr).trim().to_string();
            Err(if err.is_empty() { "ffmpeg failed".into() } else { err })
        }
    })
    .await
}

/// Run `command`, handing each stdout line to `on_line` as it arrives, and
/// return its exit status and stderr. stderr drains on its own thread:
/// read after stdout instead, a chatty child (ffmpeg logging an error per
/// broken frame) fills the stderr pipe, blocks, and never closes stdout.
fn run_with_progress(
    mut command: std::process::Command,
    mut on_line: impl FnMut(&str),
) -> std::io::Result<(std::process::ExitStatus, Vec<u8>)> {
    use std::io::{BufRead, Read};
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().expect("piped stderr");
    let drain = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    if let Some(stdout) = child.stdout.take() {
        // Byte lines, not `lines()`: one non-UTF-8 byte would end the loop
        // and leave stdout undrained, the same deadlock on the other pipe.
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line).unwrap_or(0) > 0 {
            on_line(String::from_utf8_lossy(&line).trim_end());
            line.clear();
        }
    }
    let status = child.wait()?;
    let stderr = drain.join().unwrap_or_default();
    Ok((status, stderr))
}

/// Resolve a finished repair: overwrite the original, save it next to the
/// original with a "_fix" suffix, or discard the work file.
#[tauri::command]
pub async fn finish_repair(
    tmp: PathBuf,
    original: PathBuf,
    action: String,
) -> Result<Option<PathBuf>, String> {
    // Only ever the work file repair_video made for this very original:
    // this renames over `original` or deletes `tmp`, so neither may be an
    // arbitrary pair of paths.
    if tmp != repair_work_file(&original)? {
        return Err(format!("{}: not the repair file for this video", tmp.display()));
    }
    blocking(move || match action.as_str() {
        "discard" => {
            let _ = std::fs::remove_file(&tmp);
            Ok(None)
        }
        "overwrite" => {
            std::fs::rename(&tmp, &original).map_err(|e| e.to_string())?;
            Ok(Some(original))
        }
        "save" => {
            let dir = original.parent().ok_or("no parent dir")?;
            let stem = original
                .file_stem()
                .ok_or("no file name")?
                .to_string_lossy()
                .to_string();
            let mut target = dir.join(format!("{stem}_fix.mp4"));
            let mut n = 2;
            while target.exists() {
                target = dir.join(format!("{stem}_fix {n}.mp4"));
                n += 1;
            }
            std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
            Ok(Some(target))
        }
        other => Err(format!("unknown action: {other}")),
    })
    .await
}

/// Apps the system knows can open this file, default handler first,
/// the rest alphabetical. Empty on any failure.
#[tauri::command]
pub async fn apps_for_path(path: PathBuf) -> Vec<OpenApp> {
    tauri::async_runtime::spawn_blocking(move || platform::apps_for(&path))
        .await
        .unwrap_or_default()
}

/// Launch a user-defined external tool with the given files.
#[tauri::command]
pub async fn open_with(app: String, paths: Vec<PathBuf>) -> Result<(), String> {
    if app.trim().is_empty() || paths.is_empty() {
        return Err("open_with: app and paths required".into());
    }
    blocking(move || platform::open_with(&app, &paths)).await
}

/// The system file manager's info/properties window for each path (local
/// only, capped so a huge mark set can't flood the screen with windows).
#[tauri::command]
pub async fn get_info(paths: Vec<PathBuf>) -> Result<(), String> {
    blocking(move || platform::get_info(&paths)).await
}

/// Open a local file or folder with its default app.
#[tauri::command]
pub async fn open_path(path: PathBuf) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dmg"))
        && path.is_file()
    {
        return blocking(move || platform::open_disk_image(&path)).await;
    }
    blocking(move || open::that(&path).map_err(|e| e.to_string())).await
}

/// Select the file in the system file manager (Finder, Nautilus, …).
#[tauri::command]
pub async fn reveal_in_finder(path: PathBuf) -> Result<(), String> {
    blocking(move || platform::reveal(&path)).await
}

/// File tags (Finder tags on macOS; best effort, empty elsewhere).
#[tauri::command]
pub async fn file_tags(path: PathBuf) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || platform::file_tags(&path))
        .await
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_runs_survive_a_child_flooding_stderr() {
        // 1 MiB on stderr (far past any pipe buffer) between progress lines.
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "echo out_time_us=1; head -c 1048576 /dev/zero | tr '\\0' e >&2; \
             printf 'out_time_us=2\\n\\377\\nout_time_us=3\\n'; exit 3",
        ]);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = run_with_progress(command, |l| lines.push(l.to_string()));
            let _ = tx.send((result.map(|(status, err)| (status.code(), err.len())), lines));
        });
        let (result, lines) = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("deadlocked on a full stderr pipe");
        assert_eq!(result.unwrap(), (Some(3), 1 << 20));
        // A non-UTF-8 line does not end the progress stream.
        assert_eq!(lines.first().map(String::as_str), Some("out_time_us=1"));
        assert_eq!(lines.last().map(String::as_str), Some("out_time_us=3"));
    }

    #[test]
    fn finish_repair_only_touches_its_own_work_file() {
        let dir = std::env::temp_dir().join(format!("yc_repair_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("clip.mkv");
        let other = dir.join("precious.txt");
        std::fs::write(&original, b"orig").unwrap();
        std::fs::write(&other, b"keep").unwrap();
        let run = |tmp: &Path, action: &str| {
            tauri::async_runtime::block_on(finish_repair(
                tmp.to_path_buf(),
                original.clone(),
                action.to_string(),
            ))
        };

        // Any other file is refused, for every action, and left alone.
        for action in ["discard", "overwrite", "save"] {
            assert!(run(&other, action).is_err(), "{action}");
            assert_eq!(std::fs::read(&other).unwrap(), b"keep");
            assert_eq!(std::fs::read(&original).unwrap(), b"orig");
        }

        // The real work file goes through.
        let tmp = repair_work_file(&original).unwrap();
        assert_eq!(tmp, dir.join(".clip_fix.part.mp4"));
        std::fs::write(&tmp, b"fixed").unwrap();
        let saved = run(&tmp, "save").unwrap().unwrap();
        assert_eq!(saved, dir.join("clip_fix.mp4"));
        assert_eq!(std::fs::read(&saved).unwrap(), b"fixed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_tools_get_absolute_paths_only() {
        assert!(media_input(Path::new("/Users/me/clip.mp4")).is_ok());
        assert!(media_input(Path::new("/tmp/concat:a|b")).is_ok(), "a file, never a protocol");
        for bad in ["concat:/a|/b", "http://host/x.mp4", "-version", "pipe:0", "clip.mp4"] {
            assert!(media_input(Path::new(bad)).is_err(), "{bad}");
        }
    }
}
