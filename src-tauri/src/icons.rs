//! System file icons as PNG, one per extension (plus "folder"/"generic"),
//! cached for the app's lifetime. macOS asks NSWorkspace (AppKit calls run
//! on the main thread); Linux looks the MIME type up in the freedesktop
//! icon theme. Any miss sends the frontend to its built-in (lucide) icons.

use tauri::AppHandle;

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::CString;

    const NS_BITMAP_IMAGE_FILE_TYPE_PNG: usize = 4;

    unsafe fn nsstring(s: &str) -> *mut AnyObject {
        let c = CString::new(s).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }

    /// PNG bytes of the Finder icon for `key`: "folder", "generic", a
    /// lower-cased file extension, or "path:<abs path>" for an item's own
    /// icon (app bundles, favorite folders), optionally suffixed "#<px>" for a larger size.
    /// Must run on the main thread.
    pub unsafe fn icon_png(key: &str) -> Option<Vec<u8>> {
        let (key, min_width) = super::split_size(key);
        let min_width = min_width as isize;
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let image: *mut AnyObject = match key.strip_prefix("path:") {
                Some(path) if std::path::Path::new(path).exists() => {
                    msg_send![workspace, iconForFile: nsstring(path)]
                }
                // Not on the local disk (SFTP, archive, unmounted volume):
                // the generic app or folder icon.
                Some(path) => {
                    let generic = if path.to_lowercase().ends_with(".app") {
                        "com.apple.application-bundle"
                    } else {
                        "public.folder"
                    };
                    msg_send![workspace, iconForFileType: nsstring(generic)]
                }
                None => {
                    let file_type = match key {
                        "folder" => nsstring("public.folder"),
                        "generic" | "" => nsstring("public.data"),
                        ext => nsstring(ext),
                    };
                    msg_send![workspace, iconForFileType: file_type]
                }
            };
            if image.is_null() {
                return None;
            }
            // The icon carries several representations (16/32/128/…); pick the
            // smallest one that still looks crisp at the requested size.
            let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
            if tiff.is_null() {
                return None;
            }
            let reps: *mut AnyObject =
                msg_send![class!(NSBitmapImageRep), imageRepsWithData: tiff];
            if reps.is_null() {
                return None;
            }
            let count: usize = msg_send![reps, count];
            let mut best: *mut AnyObject = std::ptr::null_mut();
            let mut best_width: isize = isize::MAX;
            for i in 0..count {
                let rep: *mut AnyObject = msg_send![reps, objectAtIndex: i];
                let width: isize = msg_send![rep, pixelsWide];
                if width >= min_width && width < best_width {
                    best = rep;
                    best_width = width;
                }
            }
            if best.is_null() && count > 0 {
                // Nothing that large: take the largest there is.
                let mut largest: isize = -1;
                for i in 0..count {
                    let rep: *mut AnyObject = msg_send![reps, objectAtIndex: i];
                    let width: isize = msg_send![rep, pixelsWide];
                    if width > largest {
                        best = rep;
                        largest = width;
                    }
                }
            }
            if best.is_null() {
                return None;
            }
            let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: *mut AnyObject = msg_send![
                best,
                representationUsingType: NS_BITMAP_IMAGE_FILE_TYPE_PNG,
                properties: props
            ];
            if png.is_null() {
                return None;
            }
            let bytes: *const u8 = msg_send![png, bytes];
            let len: usize = msg_send![png, length];
            if bytes.is_null() || len == 0 {
                return None;
            }
            Some(std::slice::from_raw_parts(bytes, len).to_vec())
        }
    }
}

/// Splits an optional "#<px>" size suffix off an icon key; the default is
/// 32 px (16 pt on a retina display, the list rows' size).
pub(crate) fn split_size(key: &str) -> (&str, u16) {
    key.rsplit_once('#')
        .and_then(|(k, px)| px.parse().ok().map(|px| (k, px)))
        .unwrap_or((key, 32))
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn file_icon(app: AppHandle, key: String) -> Result<tauri::ipc::Response, String> {
    use crate::state::AppState;
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use tauri::Manager;

    if let Some(bytes) = app.state::<AppState>().icons.lock().unwrap().get(&key) {
        return Ok(tauri::ipc::Response::new(bytes.as_ref().clone()));
    }
    let (tx, rx) = channel();
    let k = key.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(unsafe { imp::icon_png(&k) });
    })
    .map_err(|e| e.to_string())?;
    let bytes = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no icon for {key}"))?;
    let bytes = Arc::new(bytes);
    app.state::<AppState>()
        .icons
        .lock()
        .unwrap()
        .insert(key, bytes.clone());
    Ok(tauri::ipc::Response::new(bytes.as_ref().clone()))
}

/// Theme lookup is plain file I/O — no main-thread hop needed.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn file_icon(app: AppHandle, key: String) -> Result<tauri::ipc::Response, String> {
    use crate::state::AppState;
    use std::sync::Arc;
    use tauri::Manager;

    if let Some(bytes) = app.state::<AppState>().icons.lock().unwrap().get(&key) {
        return Ok(tauri::ipc::Response::new(bytes.as_ref().clone()));
    }
    let k = key.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || crate::platform::icon_png(&k))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no icon for {key}"))?;
    let bytes = Arc::new(bytes);
    app.state::<AppState>()
        .icons
        .lock()
        .unwrap()
        .insert(key, bytes.clone());
    Ok(tauri::ipc::Response::new(bytes.as_ref().clone()))
}

/// Uncached icon lookup for callers already on the main thread (the drag
/// image when the frontend has not fetched that icon yet).
pub(crate) fn icon_png_on_main_thread(key: &str) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return unsafe { imp::icon_png(key) };
    #[cfg(not(target_os = "macos"))]
    return crate::platform::icon_png(key);
}

#[cfg(test)]
mod tests {
    use super::split_size;

    #[test]
    fn splits_size_suffix() {
        assert_eq!(split_size("txt"), ("txt", 32));
        assert_eq!(split_size("folder#128"), ("folder", 128));
        assert_eq!(split_size("path:/Applications/A.app#128"), ("path:/Applications/A.app", 128));
        assert_eq!(split_size("path:/x/#y.app"), ("path:/x/#y.app", 32));
    }
}
