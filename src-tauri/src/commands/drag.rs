//! Drag-out, Finder style: hand the dragged entries to a native drag
//! session (NSDraggingSession on macOS, GTK DnD on Linux) as file URLs, so
//! they can be dropped on Finder, Mail, a browser upload field, a chat app…
//! The operation offered is Copy — the receiver copies, the source files
//! are never moved out from under the panel.

use crate::state::AppState;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use tauri::{AppHandle, Manager, Window};

/// Last resort when no system icon is available: the app's own icon.
const FALLBACK_IMAGE: &[u8] = include_bytes!("../../icons/32x32.png");

/// The GTK backend of `drag` builds each URI as `file://{path}` verbatim,
/// so spaces, `#`, `%` or CJK names would yield invalid URIs; hand it the
/// path already percent-encoded per segment. macOS goes through NSURL,
/// which wants the plain path.
#[cfg(target_os = "linux")]
fn gtk_uri_path(p: PathBuf) -> PathBuf {
    let uri = crate::platform::file_uri(&p);
    PathBuf::from(uri.strip_prefix("file://").unwrap_or(&uri))
}

#[cfg(not(target_os = "linux"))]
fn gtk_uri_path(p: PathBuf) -> PathBuf {
    p
}

/// Must be called while the mouse button is still held: the session
/// takes over the pointer from the current drag gesture.
#[tauri::command]
pub async fn start_drag(
    app: AppHandle,
    window: Window,
    paths: Vec<String>,
    icon_key: String,
) -> Result<(), String> {
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(gtk_uri_path)
        .collect();
    if paths.is_empty() {
        return Err("nothing to drag".into());
    }
    let cached = app
        .state::<AppState>()
        .icons
        .lock()
        .unwrap()
        .get(&icon_key)
        .cloned();

    let (tx, rx) = channel();
    app.run_on_main_thread(move || {
        let image = cached
            .map(|b| b.as_ref().clone())
            .or_else(|| crate::icons::icon_png_on_main_thread(&icon_key))
            .unwrap_or_else(|| FALLBACK_IMAGE.to_vec());

        #[cfg(target_os = "linux")]
        let target = window.gtk_window();
        #[cfg(not(target_os = "linux"))]
        let target = tauri::Result::Ok(window.clone());

        let result = match target {
            Ok(w) => drag::start_drag(
                &w,
                drag::DragItem::Files(paths),
                drag::Image::Raw(image),
                |_, _| {},
                drag::Options::default(),
            )
            .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::gtk_uri_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn gtk_paths_are_percent_encoded_per_segment() {
        let enc = |p: &str| gtk_uri_path(PathBuf::from(p));
        assert_eq!(enc("/home/k/My Files/a#1%.txt"), Path::new("/home/k/My%20Files/a%231%25.txt"));
        assert_eq!(enc("/home/k/中文.md"), Path::new("/home/k/%E4%B8%AD%E6%96%87.md"));
        assert_eq!(enc("/"), Path::new("/"));
    }
}
