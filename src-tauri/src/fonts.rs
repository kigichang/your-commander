//! Installed font families for the Appearance settings. macOS asks
//! NSFontManager (AppKit, so on the main thread); Linux asks fontconfig.
//! Any failure yields an empty list and the menu offers just the defaults.

use tauri::AppHandle;

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::{CStr, c_char};

    /// Must run on the main thread.
    pub unsafe fn families() -> Vec<String> {
        unsafe {
            let manager: *mut AnyObject = msg_send![class!(NSFontManager), sharedFontManager];
            if manager.is_null() {
                return Vec::new();
            }
            let list: *mut AnyObject = msg_send![manager, availableFontFamilies];
            if list.is_null() {
                return Vec::new();
            }
            let count: usize = msg_send![list, count];
            let mut out = Vec::with_capacity(count);
            for i in 0..count {
                let name: *mut AnyObject = msg_send![list, objectAtIndex: i];
                let utf8: *const c_char = msg_send![name, UTF8String];
                if !utf8.is_null() {
                    out.push(CStr::from_ptr(utf8).to_string_lossy().into_owned());
                }
            }
            out
        }
    }
}

/// Families from `fc-list : family` output: one font per line, aliases
/// comma-separated (the first is the canonical name), escaped `\-`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn parse_fc_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split(',').next())
        .map(|f| f.trim().replace('\\', ""))
        .filter(|f| !f.is_empty())
        .collect()
}

/// Sorted, de-duplicated, without macOS's hidden ".SF…" system families.
fn tidy(mut families: Vec<String>) -> Vec<String> {
    families.retain(|f| !f.starts_with('.'));
    families.sort_by_key(|f| f.to_lowercase());
    families.dedup();
    families
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn list_font_families(app: AppHandle) -> Result<Vec<String>, String> {
    use std::sync::mpsc::channel;
    let (tx, rx) = channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(unsafe { imp::families() });
    })
    .map_err(|e| e.to_string())?;
    let families = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(tidy(families))
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn list_font_families(_app: AppHandle) -> Result<Vec<String>, String> {
    let out = tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("fc-list").args([":", "family"]).output()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(match out {
        Ok(o) if o.status.success() => tidy(parse_fc_list(&String::from_utf8_lossy(&o.stdout))),
        _ => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fc_list_output() {
        let text = "\
DejaVu Sans
Noto Sans CJK TC,Noto Sans CJK TC Regular
Ubuntu Mono
DejaVu Sans
Noto Sans Mono\\-Bold,Noto Sans Mono
";
        assert_eq!(
            tidy(parse_fc_list(text)),
            ["DejaVu Sans", "Noto Sans CJK TC", "Noto Sans Mono-Bold", "Ubuntu Mono"]
        );
    }

    #[test]
    fn drops_hidden_families() {
        assert_eq!(tidy(vec![".SF NS".into(), "Menlo".into()]), ["Menlo"]);
    }
}
