//! System file icons as PNG, one per extension (plus "folder"/"generic"),
//! cached for the app's lifetime. macOS asks NSWorkspace (AppKit calls run
//! on the main thread); Linux looks the MIME type up in the freedesktop
//! icon theme. Any miss sends the frontend to its built-in (lucide) icons.

use tauri::AppHandle;

#[cfg(target_os = "macos")]
mod imp {
    use crate::platform::ns_string;
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    const NS_BITMAP_IMAGE_FILE_TYPE_PNG: usize = 4;
    const NS_COMPOSITING_OPERATION_COPY: usize = 1;

    /// NSRect, for `drawInRect:fromRect:…`.
    #[repr(C)]
    struct Rect {
        origin: [f64; 2],
        size: [f64; 2],
    }

    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct(
            "CGRect",
            &[
                Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]),
                Encoding::Struct("CGSize", &[Encoding::Double, Encoding::Double]),
            ],
        );
    }

    /// An owned (alloc/init) object, released on drop.
    struct Rep(*mut AnyObject);

    impl Drop for Rep {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.0, release];
            }
        }
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
                    msg_send![workspace, iconForFile: ns_string(path)]
                }
                // Not on the local disk (SFTP, archive, unmounted volume):
                // the generic app or folder icon.
                Some(path) => {
                    let generic = if path.to_lowercase().ends_with(".app") {
                        "com.apple.application-bundle"
                    } else {
                        "public.folder"
                    };
                    msg_send![workspace, iconForFileType: ns_string(generic)]
                }
                None => {
                    let file_type = match key {
                        "folder" => ns_string("public.folder"),
                        "generic" | "" => ns_string("public.data"),
                        ext => ns_string(ext),
                    };
                    msg_send![workspace, iconForFileType: file_type]
                }
            };
            if image.is_null() {
                return None;
            }
            // Draw the icon straight into a bitmap of the requested size.
            // (Going through TIFFRepresentation instead encodes every
            // representation up to 1024 px @2x — tens of MB and 30–350 ms
            // per icon, all of it blocking the main thread.)
            let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
            let rep: *mut AnyObject = msg_send![
                rep,
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
                pixelsWide: min_width,
                pixelsHigh: min_width,
                bitsPerSample: 8isize,
                samplesPerPixel: 4isize,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: ns_string("NSDeviceRGBColorSpace"),
                bytesPerRow: 0isize,
                bitsPerPixel: 0isize
            ];
            if rep.is_null() {
                return None;
            }
            let best = Rep(rep);
            let context: *mut AnyObject = msg_send![
                class!(NSGraphicsContext),
                graphicsContextWithBitmapImageRep: best.0
            ];
            if context.is_null() {
                return None;
            }
            let side = min_width as f64;
            let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
            let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: context];
            let _: () = msg_send![
                image,
                drawInRect: Rect { origin: [0.0, 0.0], size: [side, side] },
                fromRect: Rect { origin: [0.0, 0.0], size: [0.0, 0.0] },
                operation: NS_COMPOSITING_OPERATION_COPY,
                fraction: 1.0f64
            ];
            let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
            let props: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: *mut AnyObject = msg_send![
                best.0,
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

    /// Icons come back as a PNG of exactly the requested pixel size, fast
    /// enough not to stall the main thread while a listing scrolls.
    #[cfg(target_os = "macos")]
    #[test]
    fn renders_icons_at_requested_size_quickly() {
        for (key, px) in [("folder", 32u32), ("txt", 32), ("pdf#128", 128)] {
            let start = std::time::Instant::now();
            let png = unsafe { super::imp::icon_png(key) }.expect(key);
            let elapsed = start.elapsed();
            assert_eq!(&png[1..4], b"PNG");
            let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
            assert_eq!((width, height), (px, px), "{key}");
            eprintln!("{key}: {elapsed:?}, {} bytes", png.len());
        }
        // Warm (the first call above loads AppKit): well under a frame.
        let start = std::time::Instant::now();
        unsafe { super::imp::icon_png("json") }.unwrap();
        assert!(start.elapsed().as_millis() < 20, "{:?}", start.elapsed());
    }
}
