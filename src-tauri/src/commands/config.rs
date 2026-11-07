use crate::config::Config;
use crate::state::AppState;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, State};

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

#[tauri::command]
pub fn update_config(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    patch: Map<String, Value>,
) -> Result<(), String> {
    apply_patch(&app, &state, window.label(), patch)
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
    fn invalid_patch_is_refused() {
        let base = Config::default();
        assert!(merge_patch(&base, &patch(json!({ "show_hidden": "yes" }))).is_err());
        assert!(merge_patch(&base, &patch(json!({ "favorites": 3 }))).is_err());
    }
}
