//! Linux implementations (Ubuntu/GNOME is the reference target): GIO and
//! the freedesktop ecosystem for app lookup and file-manager integration,
//! ffprobe for media metadata, libx264 for the repair encode.
//!
//! Still to come: apps_for/open_with (gio mime + .desktop), reveal/get_info
//! (org.freedesktop.FileManager1), open_in, media_info (ffprobe). Until
//! then these return empty results or a clear error; the frontend already
//! tolerates both.

use super::{MediaInfo, OpenApp, find_cli};
use std::path::{Path, PathBuf};

pub mod smb;

pub fn open_in(tool: &str, _dir: &Path) -> Result<(), String> {
    Err(format!("\"{tool}\" is not wired up on Linux yet"))
}

/// Installed ffmpeg-based player to hand undecodable files to.
pub fn video_player() -> Option<String> {
    ["vlc", "mpv"]
        .iter()
        .find(|n| find_cli(n, &[]).is_some())
        .map(|n| n.to_string())
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    find_cli("ffmpeg", &["/usr/bin/ffmpeg", "/usr/local/bin/ffmpeg"])
}

/// Video codec arguments for the repair re-encode (software H.264; yuv420p
/// keeps 10-bit sources playable everywhere).
pub fn encode_args() -> &'static [&'static str] {
    &["-c:v", "libx264", "-crf", "23", "-preset", "veryfast", "-pix_fmt", "yuv420p"]
}

pub fn apps_for(_path: &Path) -> Vec<OpenApp> {
    vec![]
}

pub fn open_with(app: &str, _paths: &[PathBuf]) -> Result<(), String> {
    Err(format!("\"{app}\": Open With is not wired up on Linux yet"))
}

pub fn get_info(_paths: &[PathBuf]) -> Result<(), String> {
    Err("Get Info is not wired up on Linux yet".to_string())
}

pub fn reveal(_path: &Path) -> Result<(), String> {
    Err("Reveal is not wired up on Linux yet".to_string())
}

/// No portable tag store on Linux; tags stay empty.
pub fn file_tags(_path: &Path) -> Vec<String> {
    vec![]
}

pub fn media_info(_path: &Path) -> Result<MediaInfo, String> {
    Ok(MediaInfo::default())
}
