use super::provider::{epoch_millis, FsEntry};
use std::io;
use std::path::{Path, PathBuf};

/// Stateless local directory listing.
pub fn list_dir(path: &Path) -> io::Result<Vec<FsEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let symlink_meta = entry.metadata();
        // For symlinks, follow the link for type/size but remember it is one.
        let is_symlink = path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let (is_dir, size, modified) = match symlink_meta {
            Ok(m) => (path.is_dir(), m.len(), m.modified().ok().map(epoch_millis)),
            Err(_) => (false, 0, None),
        };
        let ext = if is_dir {
            String::new()
        } else {
            path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        let is_hidden = name.starts_with('.');
        entries.push(FsEntry {
            name,
            path,
            is_dir,
            is_symlink,
            size,
            modified,
            ext,
            is_hidden,
        });
    }
    Ok(entries)
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// A mounted volume shown in the panel "drives" menu (includes SMB mounts).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    pub name: String,
    pub path: PathBuf,
    pub free: u64,
    pub total: u64,
}

fn statfs(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let block = stat.f_bsize as u64;
    Some((stat.f_bavail * block, stat.f_blocks * block))
}

pub fn volumes() -> Vec<VolumeInfo> {
    let mut out = Vec::new();
    // The boot volume itself ("/") is Macintosh HD; /Volumes holds the rest
    // (and a firmlink to the boot volume on most systems).
    if let Ok(read) = std::fs::read_dir("/Volumes") {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let (free, total) = statfs(&path).unwrap_or((0, 0));
            out.push(VolumeInfo {
                name,
                path,
                free,
                total,
            });
        }
    }
    out.sort_by_key(|v| v.name.to_lowercase());
    out
}
