//! SMB over GVfs (`gio mount smb://…`), sharing mounts with GNOME Files
//! under $XDG_RUNTIME_DIR/gvfs. Implementation lands with the SMB phase;
//! until then every call reports the feature as unavailable.

use std::path::{Path, PathBuf};

const NOT_YET: &str = "SMB is not wired up on Linux yet";

pub fn find_mount(_server: &str, _share: &str) -> Option<PathBuf> {
    None
}

pub fn mount(
    _server: &str,
    _share: &str,
    _username: &str,
    _password: &str,
) -> Result<PathBuf, String> {
    Err(NOT_YET.to_string())
}

pub fn list_shares(_server: &str, _username: &str, _password: &str) -> Result<Vec<String>, String> {
    Err(NOT_YET.to_string())
}

pub fn unmount(_path: &Path) -> Result<(), String> {
    Err(NOT_YET.to_string())
}
