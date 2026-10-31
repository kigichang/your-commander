use crate::fs::local::{self, VolumeInfo};
use crate::ops::archive;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::Manager;

/// Open another main window — an independent workspace. Labels run
/// "main", "main-2", "main-3", … and share the "main*" capability.
/// Must run on the main thread (IPC commands and menu actions both do).
pub fn open_new_window(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{LogicalPosition, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};
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
        .min_inner_size(800.0, 500.0)
        .hidden_title(true)
        .title_bar_style(TitleBarStyle::Overlay)
        .traffic_light_position(LogicalPosition::new(16.0, 18.0));
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

/// First existing path among `candidates`, else a PATH lookup via `which`
/// (the app's own PATH lacks Homebrew when launched from Finder).
fn find_cli(name: &str, candidates: &[&str]) -> Option<PathBuf> {
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    let out = std::process::Command::new("/usr/bin/which")
        .arg(name)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// "New … Here" in the blank-space context menu: open a terminal or editor
/// in `dir`. `tool` is "terminal", "vscode", or "zed".
#[tauri::command]
pub async fn open_in(tool: String, dir: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let spawn = |mut cmd: std::process::Command| {
            cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
        };
        match tool.as_str() {
            "terminal" => {
                let mut cmd = std::process::Command::new("/usr/bin/open");
                cmd.arg("-a").arg("Terminal").arg(&dir);
                spawn(cmd)
            }
            "vscode" => {
                let cli = find_cli(
                    "code",
                    &[
                        "/opt/homebrew/bin/code",
                        "/usr/local/bin/code",
                        "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                    ],
                );
                match cli {
                    // -n: always a new window, like the menu item says.
                    Some(code) => {
                        let mut cmd = std::process::Command::new(code);
                        cmd.arg("-n").arg(&dir);
                        spawn(cmd)
                    }
                    None => {
                        let mut cmd = std::process::Command::new("/usr/bin/open");
                        cmd.arg("-na").arg("Visual Studio Code").arg("--args").arg(&dir);
                        spawn(cmd)
                    }
                }
            }
            "zed" => {
                let cli = find_cli(
                    "zed",
                    &[
                        "/opt/homebrew/bin/zed",
                        "/usr/local/bin/zed",
                        "/Applications/Zed.app/Contents/MacOS/cli",
                    ],
                );
                match cli {
                    Some(zed) => {
                        let mut cmd = std::process::Command::new(zed);
                        cmd.arg(&dir);
                        spawn(cmd)
                    }
                    None => {
                        let mut cmd = std::process::Command::new("/usr/bin/open");
                        cmd.arg("-a").arg("Zed").arg(&dir);
                        spawn(cmd)
                    }
                }
            }
            other => Err(format!("unknown tool: {other}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
pub struct VideoTools {
    /// Installed ffmpeg-based player to hand undecodable files to.
    pub player: Option<String>,
    /// Whether ffmpeg is available for the repair (re-encode) action.
    pub ffmpeg: bool,
}

/// Which local helpers exist for videos AVFoundation can't decode.
#[tauri::command]
pub async fn video_tools() -> VideoTools {
    tauri::async_runtime::spawn_blocking(|| {
        let app_exists = |name: &str| {
            PathBuf::from(format!("/Applications/{name}.app")).exists()
                || local::home_dir().join(format!("Applications/{name}.app")).exists()
        };
        let player = ["VLC", "IINA"].iter().find(|n| app_exists(n)).map(|n| n.to_string());
        let ffmpeg = find_ffmpeg().is_some();
        VideoTools { player, ffmpeg }
    })
    .await
    .unwrap_or(VideoTools { player: None, ffmpeg: false })
}

fn find_ffmpeg() -> Option<PathBuf> {
    find_cli("ffmpeg", &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"])
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

/// Re-encode a video AVFoundation can't decode into a playable H.264 mp4
/// (VideoToolbox hardware encode). Writes a hidden work file next to the
/// original and returns its path; finish_repair decides what to keep.
/// Progress fractions (0..1) stream over `on_progress` while it encodes.
#[tauri::command]
pub async fn repair_video(
    path: PathBuf,
    on_progress: tauri::ipc::Channel<f32>,
) -> Result<PathBuf, String> {
    let ffmpeg = find_ffmpeg().ok_or("ffmpeg not found")?;
    let dir = path.parent().ok_or("no parent dir")?.to_path_buf();
    let stem = path
        .file_stem()
        .ok_or("no file name")?
        .to_string_lossy()
        .to_string();
    let tmp = dir.join(format!(".{stem}_fix.part.mp4"));
    let src = path.clone();
    let out_path = tmp.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::BufRead;
        let total = probe_duration(&ffmpeg, &src);
        let mut child = std::process::Command::new(ffmpeg)
            .arg("-hide_banner")
            .args(["-v", "error", "-y", "-progress", "pipe:1", "-nostats", "-i"])
            .arg(&src)
            .args(["-c:v", "h264_videotoolbox", "-q:v", "65", "-vf", "setsar=1"])
            .args(["-c:a", "copy", "-movflags", "+faststart"])
            .arg(&out_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let (Some(stdout), Some(total)) = (child.stdout.take(), total) {
            // One key=value block per encoded chunk; out_time_us (and the
            // misnamed out_time_ms) are both microseconds.
            let mut last = -1i32;
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
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
            }
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(out_path)
        } else {
            let _ = std::fs::remove_file(&out_path);
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if err.is_empty() { "ffmpeg failed".into() } else { err })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Resolve a finished repair: overwrite the original, save it next to the
/// original with a "_fix" suffix, or discard the work file.
#[tauri::command]
pub async fn finish_repair(
    tmp: PathBuf,
    original: PathBuf,
    action: String,
) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
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
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
pub struct OpenApp {
    pub name: String,
    pub path: String,
    /// LaunchServices' default handler for this file.
    pub default: bool,
}

/// Apps LaunchServices knows can open this file, default handler first,
/// the rest alphabetical. Empty on any failure.
#[tauri::command]
pub async fn apps_for_path(path: PathBuf) -> Vec<OpenApp> {
    tauri::async_runtime::spawn_blocking(move || apps_for(&path))
        .await
        .unwrap_or_default()
}

fn apps_for(path: &std::path::Path) -> Vec<OpenApp> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::url::{CFURL, CFURLRef};

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyApplicationURLsForURL(in_url: CFURLRef, roles: u32) -> CFArrayRef;
        fn LSCopyDefaultApplicationURLForURL(
            in_url: CFURLRef,
            roles: u32,
            out_error: *mut std::ffi::c_void,
        ) -> CFURLRef;
    }
    const ROLES_ALL: u32 = 0xFFFF_FFFF;

    let Some(url) = CFURL::from_path(path, false) else {
        return vec![];
    };
    let default_path = unsafe {
        let r = LSCopyDefaultApplicationURLForURL(
            url.as_concrete_TypeRef(),
            ROLES_ALL,
            std::ptr::null_mut(),
        );
        if r.is_null() {
            None
        } else {
            CFURL::wrap_under_create_rule(r).to_path()
        }
    };
    let mut out: Vec<OpenApp> = vec![];
    unsafe {
        let arr = LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), ROLES_ALL);
        if arr.is_null() {
            return vec![];
        }
        let arr: CFArray<CFURL> = CFArray::wrap_under_create_rule(arr);
        for app_url in arr.iter() {
            let Some(p) = app_url.to_path() else { continue };
            let path_str = p.to_string_lossy().to_string();
            if out.iter().any(|a| a.path == path_str) {
                continue;
            }
            let Some(name) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            let default = default_path.as_deref() == Some(p.as_path());
            out.push(OpenApp { name, path: path_str, default });
        }
    }
    out.sort_by(|a, b| {
        (!a.default, a.name.to_lowercase()).cmp(&(!b.default, b.name.to_lowercase()))
    });
    out
}

/// Launch a user-defined external tool: `open -a <app> <paths...>`.
/// Waits for `open` to exit (it returns right after handing the files to
/// LaunchServices) so a missing app surfaces as an error instead of nothing.
#[tauri::command]
pub async fn open_with(app: String, paths: Vec<PathBuf>) -> Result<(), String> {
    if app.trim().is_empty() || paths.is_empty() {
        return Err("open_with: app and paths required".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new("/usr/bin/open")
            .arg("-a")
            .arg(&app)
            .args(&paths)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if err.is_empty() {
                format!("open -a \"{app}\" failed")
            } else {
                err
            })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Finder's Get Info window for each path (local only, capped so a huge
/// mark set can't flood the screen with windows).
#[tauri::command]
pub async fn get_info(paths: Vec<PathBuf>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut script = String::from("tell application \"Finder\"\nactivate\n");
        for p in paths.iter().take(10) {
            let s = p
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            script.push_str(&format!(
                "open information window of (POSIX file \"{s}\" as alias)\n"
            ));
        }
        script.push_str("end tell\n");
        let mut child = std::process::Command::new("/usr/bin/osascript")
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().ok_or("osascript: no stdin")?;
            stdin.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Can the panel step into this archive like a folder (2d)?
#[tauri::command]
pub fn can_browse_archive(name: String) -> bool {
    crate::fs::archive_fs::can_browse(&name)
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
