use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// One entry in a directory listing, independent of where it lives
/// (local disk, mounted SMB volume, or a remote SFTP session).
/// Serialized straight to the frontend; keep fields lean.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// Modification time as Unix epoch milliseconds.
    pub modified: Option<u64>,
    /// Lower-cased extension for files, empty for folders.
    pub ext: String,
    pub is_hidden: bool,
}

pub fn epoch_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

