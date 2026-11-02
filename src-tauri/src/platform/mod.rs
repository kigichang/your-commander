//! Platform-specific system integration, one implementation per OS behind
//! a shared set of free functions: "open with" app lookup and launch,
//! reveal/get-info in the system file manager, terminal/editor launchers,
//! media probing and encoding, file tags, and SMB mounting. Command
//! signatures (and the frontend IPC) stay identical across platforms.

use std::path::PathBuf;

#[derive(serde::Serialize)]
pub struct OpenApp {
    pub name: String,
    pub path: String,
    /// The system's default handler for this file.
    pub default: bool,
}

#[derive(serde::Serialize, Default)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// First existing path among `candidates`, else a PATH lookup via `which`
/// (the app's own PATH lacks Homebrew when launched from Finder).
pub(crate) fn find_cli(name: &str, candidates: &[&str]) -> Option<PathBuf> {
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

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
