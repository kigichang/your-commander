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
