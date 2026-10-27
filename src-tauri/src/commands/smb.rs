use crate::fs::smb;
use crate::keychain;
use std::path::PathBuf;

/// Where `//server/share` is currently mounted, if anywhere.
#[tauri::command]
pub async fn smb_find_mount(server: String, share: String) -> Option<PathBuf> {
    tauri::async_runtime::spawn_blocking(move || smb::find_mount(&server, &share))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
pub async fn smb_unmount(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || smb::unmount(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// Stored password for a saved SMB connection (Keychain).
#[tauri::command]
pub fn smb_saved_password(username: String, server: String, share: String) -> Option<String> {
    keychain::load(&keychain::smb_account(&username, &server, &share))
}

#[tauri::command]
pub fn smb_store_password(
    username: String,
    server: String,
    share: String,
    password: String,
) -> Result<(), String> {
    keychain::store(&keychain::smb_account(&username, &server, &share), &password)
}

#[tauri::command]
pub fn smb_forget_password(username: String, server: String, share: String) {
    keychain::remove(&keychain::smb_account(&username, &server, &share));
}
