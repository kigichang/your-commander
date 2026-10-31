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
