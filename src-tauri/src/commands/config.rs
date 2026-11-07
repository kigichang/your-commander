use crate::config::{Config, SshConnection};
use crate::keychain;
use crate::state::AppState;
use serde_json::{Map, Value};
use super::blocking;
use tauri::{AppHandle, Emitter, Manager, State};

/// Event every window hears after the config changes: the changed fields
/// only, and which window changed them (that window already has them).
pub const CONFIG_PATCHED: &str = "config-patched";

#[derive(Clone, serde::Serialize)]
pub struct ConfigPatched {
    pub source: String,
    pub patch: Map<String, Value>,
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

/// `config` with the top-level fields in `patch` replaced. Fails, leaving
/// nothing changed, if the result is not a valid config.
pub fn merge_patch(config: &Config, patch: &Map<String, Value>) -> Result<Config, String> {
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    let fields = value.as_object_mut().ok_or("config is not an object")?;
    for (key, v) in patch {
        fields.insert(key.clone(), v.clone());
    }
    serde_json::from_value(value).map_err(|e| format!("Invalid setting: {e}"))
}

/// Apply `patch` to the shared config, save it, and tell every window.
/// Windows send only the fields they changed, so one window can never
/// write back another's settings from a stale copy. The event goes out
/// under the config lock so every window sees patches in save order.
pub fn apply_patch(
    app: &AppHandle,
    state: &AppState,
    source: &str,
    patch: Map<String, Value>,
) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    let next = merge_patch(&config, &patch)?;
    next.save();
    *config = next;
    let _ = app.emit(
        CONFIG_PATCHED,
        ConfigPatched {
            source: source.to_string(),
            patch,
        },
    );
    Ok(())
}

/// Async, so the file write runs off the main (UI) thread; every
/// navigation saves the panel state through here.
#[tauri::command]
pub async fn update_config(
    app: AppHandle,
    window: tauri::Window,
    patch: Map<String, Value>,
) -> Result<(), String> {
    let source = window.label().to_string();
    blocking(move || apply_patch(&app, &app.state::<AppState>(), &source, patch)).await
}

/// May a password saved under the old port-less key (`ssh://user@host`)
/// be taken as this non-22 connection's? Before ports counted, the saved
/// list held at most one connection per user@host, so the old entry is
/// this one's exactly when it is still the only user@host connection. With
/// any other (a port-22 one now owns the old key), it is not adopted.
fn owns_legacy_key(conns: &[SshConnection], username: &str, host: &str, port: u16) -> bool {
    let mut same = conns.iter().filter(|c| c.username == username && c.host == host);
    port != 22 && matches!((same.next(), same.next()), (Some(c), None) if c.port == port)
}

fn saved_ssh_password(app: &AppHandle, username: &str, host: &str, port: u16) -> Option<String> {
    let account = keychain::ssh_account(username, host, port);
    if let Some(password) = keychain::load(&account) {
        return Some(password);
    }
    // One-time move of a password saved before ports were part of the key.
    let legacy = keychain::ssh_account(username, host, 22);
    let owned = {
        let state = app.state::<AppState>();
        let config = state.config.lock().unwrap();
        owns_legacy_key(&config.ssh_connections, username, host, port)
    };
    if !owned {
        return None;
    }
    let password = keychain::load(&legacy)?;
    if keychain::store(&account, &password).is_ok() {
        keychain::remove(&legacy);
    }
    Some(password)
}

// The credential store can block (the Keychain or Secret Service asking
// to be unlocked), so these run off the main thread like file I/O.

/// SSH password/passphrase storage, keyed by user, host and port (see
/// `keychain::ssh_account`).
#[tauri::command]
pub async fn ssh_saved_password(
    app: AppHandle,
    username: String,
    host: String,
    port: u16,
) -> Option<String> {
    blocking(move || Ok::<_, String>(saved_ssh_password(&app, &username, &host, port)))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
pub async fn ssh_store_password(
    username: String,
    host: String,
    port: u16,
    password: String,
) -> Result<(), String> {
    blocking(move || keychain::store(&keychain::ssh_account(&username, &host, port), &password))
        .await
}

#[tauri::command]
pub async fn ssh_forget_password(username: String, host: String, port: u16) {
    let _ = blocking(move || {
        keychain::remove(&keychain::ssh_account(&username, &host, port));
        Ok::<_, String>(())
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::merge_patch;
    use crate::config::Config;
    use serde_json::json;

    fn patch(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn patch_replaces_only_its_fields() {
        let base = Config {
            theme: "dark".into(),
            show_hidden: true,
            window_size: Some((1200.0, 800.0)),
            ..Default::default()
        };
        let next = merge_patch(&base, &patch(json!({ "show_hidden": false, "language": "zh-TW" })))
            .unwrap();
        assert!(!next.show_hidden);
        assert_eq!(next.language.as_deref(), Some("zh-TW"));
        // Untouched fields survive, including ones only the backend writes.
        assert_eq!(next.theme, "dark");
        assert_eq!(next.window_size, Some((1200.0, 800.0)));
    }

    #[test]
    fn ssh_keys_tell_ports_apart() {
        use crate::keychain::ssh_account;
        assert_eq!(ssh_account("me", "box", 22), "ssh://me@box", "old entries keep working");
        assert_eq!(ssh_account("me", "box", 2222), "ssh://me@box:2222");
        assert_ne!(ssh_account("me", "box", 2222), ssh_account("me", "box", 2223));
        // An IPv6 host cannot pass for another host plus a port.
        assert_eq!(ssh_account("me", "::1", 2222), "ssh://me@[::1]:2222");
        assert_ne!(ssh_account("me", "::1", 2222), ssh_account("me", "::1:2222", 22));
    }

    #[test]
    fn legacy_key_is_adopted_only_by_its_sole_owner() {
        use super::owns_legacy_key;
        use crate::config::SshConnection;
        let conn = |port| SshConnection {
            host: "box".into(),
            username: "me".into(),
            port,
            ..Default::default()
        };
        // The only me@box connection: the old key was saved for it.
        assert!(owns_legacy_key(&[conn(2222)], "me", "box", 2222));
        // Port 22 reads the old key directly; nothing to adopt.
        assert!(!owns_legacy_key(&[conn(22)], "me", "box", 22));
        // A second me@box connection: whose the old key is, is unknown.
        assert!(!owns_legacy_key(&[conn(2222), conn(2223)], "me", "box", 2222));
        assert!(!owns_legacy_key(&[conn(22), conn(2222)], "me", "box", 2222));
        // Not saved at all, or saved under another port.
        assert!(!owns_legacy_key(&[], "me", "box", 2222));
        assert!(!owns_legacy_key(&[conn(2223)], "me", "box", 2222));
    }

    #[test]
    fn invalid_patch_is_refused() {
        let base = Config::default();
        assert!(merge_patch(&base, &patch(json!({ "show_hidden": "yes" }))).is_err());
        assert!(merge_patch(&base, &patch(json!({ "favorites": 3 }))).is_err());
    }
}
