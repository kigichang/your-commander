use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One entry in a directory listing, independent of where it lives
/// (local disk, mounted SMB volume, or a remote SFTP session).
#[derive(Clone, Debug)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Lower-cased extension for files, empty for folders.
    pub ext: String,
    pub is_hidden: bool,
}

impl FsEntry {
    pub fn kind_label(&self) -> String {
        if self.is_dir {
            rust_i18n::t!("list.folder").to_string()
        } else if self.ext.is_empty() {
            rust_i18n::t!("list.file").to_string()
        } else {
            self.ext.to_uppercase()
        }
    }
}

/// Abstraction over a browsable file system. The local disk implements it
/// directly; SFTP will implement it in a later milestone. SMB shares are
/// mounted under /Volumes and browsed through the local provider.
pub trait FsProvider: Send {
    /// Short scheme tag, e.g. "file" or "sftp".
    fn scheme(&self) -> &'static str;
    /// Human-readable location prefix shown in the path bar ("" for local).
    fn display_prefix(&self) -> String;
    fn list_dir(&mut self, path: &Path) -> io::Result<Vec<FsEntry>>;
    /// Default directory to land in when the provider is first opened.
    fn initial_dir(&self) -> PathBuf;
    fn is_local(&self) -> bool {
        self.scheme() == "file"
    }
    /// The live SFTP connection behind this provider, when there is one.
    fn sftp_conn(&self) -> Option<std::sync::Arc<super::sftp::SftpConnection>> {
        None
    }
}
