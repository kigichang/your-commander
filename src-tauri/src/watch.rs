use crate::state::AppState;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Watchers are per window and per panel side, so several main windows can
/// watch independently.
pub fn watch_key(window_label: &str, watch_id: u32) -> String {
    format!("{window_label}:{watch_id}")
}

/// Watch one directory (non-recursive) and emit a debounced `fs:changed`
/// event carrying `watch_id` to the calling window only. Re-watching the
/// same (window, id) replaces the old watch.
///
/// Everything that can touch the disk — creating the watch AND dropping a
/// replaced one — stays off the main thread: an unresponsive external or
/// network volume must stall only this call, never the event loop.
#[tauri::command]
pub async fn watch_dir(
    app: AppHandle,
    window: tauri::Window,
    watch_id: u32,
    path: PathBuf,
) -> Result<(), String> {
    let label = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            })
            .map_err(|e| e.to_string())?;
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;

        // Dropping the previous watcher drops its sender; its debounce thread
        // then sees a disconnect and exits. The drop happens on this thread.
        let previous = app
            .state::<AppState>()
            .watchers
            .lock()
            .unwrap()
            .insert(watch_key(&label, watch_id), watcher);
        drop(previous);

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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn unwatch(
    app: AppHandle,
    window: tauri::Window,
    watch_id: u32,
) -> Result<(), String> {
    let removed = app
        .state::<AppState>()
        .watchers
        .lock()
        .unwrap()
        .remove(&watch_key(window.label(), watch_id));
    // FSEvents teardown can block on a dead volume too.
    if let Some(watcher) = removed {
        tauri::async_runtime::spawn_blocking(move || drop(watcher));
    }
    Ok(())
}
