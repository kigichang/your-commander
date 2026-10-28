use crate::state::AppState;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

/// Watchers are per window and per panel side, so several main windows can
/// watch independently.
pub fn watch_key(window_label: &str, watch_id: u32) -> String {
    format!("{window_label}:{watch_id}")
}

/// Watch one directory (non-recursive) and emit a debounced `fs:changed`
/// event carrying `watch_id` to the calling window only. Re-watching the
/// same (window, id) replaces the old watch.
#[tauri::command]
pub fn watch_dir(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    watch_id: u32,
    path: PathBuf,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&path, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    // Dropping the previous watcher drops its sender; its debounce thread
    // then sees a disconnect and exits.
    let label = window.label().to_string();
    state
        .watchers
        .lock()
        .unwrap()
        .insert(watch_key(&label, watch_id), watcher);

    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Collapse bursts: wait until the events go quiet for 300 ms.
            loop {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(()) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            let _ = app.emit_to(label.as_str(), "fs:changed", watch_id);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn unwatch(window: tauri::Window, state: State<'_, AppState>, watch_id: u32) {
    state
        .watchers
        .lock()
        .unwrap()
        .remove(&watch_key(window.label(), watch_id));
}
