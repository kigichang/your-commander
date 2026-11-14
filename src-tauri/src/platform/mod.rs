//! Platform-specific system integration, one implementation per OS behind
//! a shared set of free functions: "open with" app lookup and launch,
//! reveal/get-info in the system file manager, terminal/editor launchers,
//! media probing and encoding, file tags, and SMB mounting. Command
//! signatures (and the frontend IPC) stay identical across platforms.

use std::path::PathBuf;

/// file:// URI with each path segment percent-encoded: spaces, "#", "%" and
/// CJK names stay valid (soffice's -env:UserInstallation, GTK drags), and
/// quotes are escaped too, so it can sit in a GVariant literal.
pub(crate) fn file_uri(path: &std::path::Path) -> String {
    let encoded: Vec<String> = path
        .to_string_lossy()
        .split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect();
    format!("file://{}", encoded.join("/"))
}

#[derive(serde::Serialize)]
pub struct OpenApp {
    pub name: String,
    pub path: String,
    /// The system's default handler for this file.
    pub default: bool,
}

/// How long a media probe (ffmpeg, ffprobe, mdimport) may take. Quick View
/// probes whatever is under the cursor: a file on a dead network mount, or
/// one crafted to stall the decoder, must not hold a worker for good.
pub(crate) const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// What a probe may print (an ffprobe JSON dump, mdimport's attribute list).
pub(crate) const PROBE_OUTPUT_MAX: u64 = 8 * 1024 * 1024;

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

/// LibreOffice's `soffice` for Office previews: the path set in Settings
/// when it points at a file, else the usual install locations, else PATH.
pub fn find_soffice(override_path: &str) -> Option<PathBuf> {
    let custom = override_path.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        // Only an absolute path: a relative one would resolve against
        // whatever the app's working directory happens to be.
        return (p.is_absolute() && p.is_file()).then_some(p);
    }
    #[cfg(target_os = "macos")]
    {
        let in_apps = |root: PathBuf| root.join("LibreOffice.app/Contents/MacOS/soffice");
        for p in [
            in_apps(PathBuf::from("/Applications")),
            in_apps(crate::fs::local::home_dir().join("Applications")),
        ] {
            if p.is_file() {
                return Some(p);
            }
        }
        find_cli("soffice", &["/opt/homebrew/bin/soffice", "/usr/local/bin/soffice"])
    }
    #[cfg(not(target_os = "macos"))]
    {
        find_cli(
            "soffice",
            &["/usr/bin/soffice", "/usr/bin/libreoffice", "/snap/bin/libreoffice"],
        )
        .or_else(|| find_cli("libreoffice", &[]))
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(test)]
mod tests {
    #[test]
    fn file_uris_are_escaped() {
        assert_eq!(
            super::file_uri(std::path::Path::new("/home/kigi/My Files/a'b#c.txt")),
            "file:///home/kigi/My%20Files/a%27b%23c.txt"
        );
    }
}
