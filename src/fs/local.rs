use super::provider::{FsEntry, FsProvider};
use std::io;
use std::path::{Path, PathBuf};

pub struct LocalFs;

impl FsProvider for LocalFs {
    fn scheme(&self) -> &'static str {
        "file"
    }

    fn display_prefix(&self) -> String {
        String::new()
    }

    fn list_dir(&mut self, path: &Path) -> io::Result<Vec<FsEntry>> {
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
                Ok(m) => (path.is_dir(), m.len(), m.modified().ok()),
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

    fn initial_dir(&self) -> PathBuf {
        home_dir()
    }
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Mounted volumes shown in the panel "drives" menu (includes SMB mounts).
pub fn volumes() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir("/Volumes") {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            out.push((name, entry.path()));
        }
    }
    out.sort_by_key(|v| v.0.to_lowercase());
    out
}
