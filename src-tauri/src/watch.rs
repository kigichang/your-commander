use crate::state::AppState;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

/// Watch one directory (non-recursive) and emit a debounced `fs:changed`
/// event carrying `watch_id`. Re-watching the same id replaces the old watch.
#[tauri::command]
pub fn watch_dir(
    app: AppHandle,
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
    state.watchers.lock().unwrap().insert(watch_id, watcher);

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
            let _ = app.emit("fs:changed", watch_id);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn unwatch(state: State<'_, AppState>, watch_id: u32) {
    state.watchers.lock().unwrap().remove(&watch_id);
}
