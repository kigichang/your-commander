//! System file icons: NSWorkspace's icon for a file type, encoded as PNG.
//! One icon per extension (plus "folder"/"generic"), cached for the app's
//! lifetime. AppKit image calls run on the main thread.

use crate::state::AppState;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CString;
use std::sync::mpsc::channel;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

const NS_BITMAP_IMAGE_FILE_TYPE_PNG: usize = 4;

unsafe fn nsstring(s: &str) -> *mut AnyObject {
    let c = CString::new(s).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

/// PNG bytes of the Finder icon for `key`: "folder", "generic", or a
/// lower-cased file extension. Must run on the main thread.
unsafe fn icon_png(key: &str) -> Option<Vec<u8>> {
    unsafe {
        let file_type = match key {
            "folder" => nsstring("public.folder"),
            "generic" | "" => nsstring("public.data"),
            ext => nsstring(ext),
        };
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let image: *mut AnyObject = msg_send![workspace, iconForFileType: file_type];
        if image.is_null() {
            return None;
        }
        // The icon carries several representations (16/32/128/…); pick the
        // smallest one that still looks crisp at 16 pt on a retina display.
        let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
        if tiff.is_null() {
            return None;
        }
        let reps: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepsWithData: tiff];
        if reps.is_null() {
            return None;
        }
        let count: usize = msg_send![reps, count];
        let mut best: *mut AnyObject = std::ptr::null_mut();
        let mut best_width: isize = isize::MAX;
        for i in 0..count {
            let rep: *mut AnyObject = msg_send![reps, objectAtIndex: i];
            let width: isize = msg_send![rep, pixelsWide];
            if width >= 32 && width < best_width {
                best = rep;
                best_width = width;
            }
        }
        if best.is_null() && count > 0 {
            // Nothing at 32px or above: take the largest there is.
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

#[tauri::command]
pub async fn file_icon(app: AppHandle, key: String) -> Result<tauri::ipc::Response, String> {
    if let Some(bytes) = app.state::<AppState>().icons.lock().unwrap().get(&key) {
        return Ok(tauri::ipc::Response::new(bytes.as_ref().clone()));
    }
    let (tx, rx) = channel();
    let k = key.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(unsafe { icon_png(&k) });
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
