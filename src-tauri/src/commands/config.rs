use crate::config::Config;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, config: Config) {
    config.save();
    *state.config.lock().unwrap() = config;
}

/// SSH password/passphrase storage, keyed like the old app: `ssh://user@host`.
#[tauri::command]
pub fn ssh_saved_password(username: String, host: String) -> Option<String> {
    crate::keychain::load(&format!("ssh://{username}@{host}"))
}

#[tauri::command]
pub fn ssh_store_password(username: String, host: String, password: String) -> Result<(), String> {
    crate::keychain::store(&format!("ssh://{username}@{host}"), &password)
}

#[tauri::command]
pub fn ssh_forget_password(username: String, host: String) {
    crate::keychain::remove(&format!("ssh://{username}@{host}"));
}
