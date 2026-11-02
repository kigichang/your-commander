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

/// "New Herdr Here": a new shell terminal in `dir`, kept alive by herdr-core
/// (the Herdr app shows it). Waits for `--bg` to return so a failure
/// reaches the toast.
fn herdr_shell(dir: &std::path::Path) -> Result<(), String> {
    let local = crate::fs::local::home_dir().join(".local/bin/herdr-core");
    let core = find_cli(
        "herdr-core",
        &[
            local.to_str().unwrap_or_default(),
            "/opt/homebrew/bin/herdr-core",
            "/usr/local/bin/herdr-core",
        ],
    )
    .ok_or("herdr-core not found")?;
    let out = std::process::Command::new(core)
        .args(["run", "--bg", "--no-parent", "--cwd"])
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "herdr-core run failed".to_string() } else { err });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
