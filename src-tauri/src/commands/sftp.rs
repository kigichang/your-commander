use crate::fs::sftp::{self, SshAuth};
use crate::state::AppState;
use std::path::PathBuf;
use tauri::State;

/// Usable (non-wildcard) Host entry from ~/.ssh/config.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: u16,
    pub identity: String,
}

#[tauri::command]
pub async fn ssh_hosts() -> Vec<SshConfigHost> {
    tauri::async_runtime::spawn_blocking(parse_ssh_config_hosts)
        .await
        .unwrap_or_default()
}

fn parse_ssh_config_hosts() -> Vec<SshConfigHost> {
    use ssh2_config::{ParseRule, SshConfig};
    let Ok(config) = SshConfig::parse_default_file(ParseRule::ALLOW_UNKNOWN_FIELDS) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for host in config.get_hosts() {
        let Some(clause) = host.pattern.first() else {
            continue;
        };
        let alias = clause.pattern.clone();
        if alias.contains('*') || alias.contains('?') {
            continue;
        }
        let params = config.query(&alias);
        out.push(SshConfigHost {
            host_name: params.host_name.clone().unwrap_or_else(|| alias.clone()),
            user: params.user.clone().unwrap_or_default(),
            port: params.port.unwrap_or(22),
            identity: params
                .identity_file
                .as_ref()
                .and_then(|v| v.first())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            alias,
        });
    }
    out.sort_by_key(|h| h.alias.to_lowercase());
    out
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// true: password auth, false: public key auth.
    pub use_password: bool,
    #[serde(default)]
    pub use_agent: bool,
    pub password: Option<String>,
    /// Private key path; empty means try the default ~/.ssh keys.
    pub identity_file: Option<String>,
    pub passphrase: Option<String>,
    /// Remote folder to land in; falls back to home when missing.
    #[serde(default)]
    pub start_in: Option<String>,
}

fn auth_of(params: &SftpConnectParams) -> SshAuth {
    if params.use_agent {
        SshAuth::Agent
    } else if params.use_password {
        SshAuth::Password(params.password.clone().unwrap_or_default())
    } else {
        SshAuth::Key {
            path: params
                .identity_file
                .clone()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
            passphrase: params.passphrase.clone().filter(|p| !p.is_empty()),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpConnected {
    pub conn_id: u32,
    pub label: String,
    pub home: PathBuf,
}

/// Blocking SSH connect in a worker; returns the registered connection.
#[tauri::command]
pub async fn sftp_connect(
    state: State<'_, AppState>,
    params: SftpConnectParams,
) -> Result<SftpConnected, String> {
    let (conn, home) = tauri::async_runtime::spawn_blocking(move || {
        let auth = auth_of(&params);
        let conn = sftp::connect(&params.host, params.port, &params.username, &auth)?;
        // Land in start_in when it exists as a folder, else in home.
        let home = match params.start_in.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) if conn.exists(std::path::Path::new(p)) == Some(true) => PathBuf::from(p),
            _ => conn.home.clone(),
        };
        Ok::<_, String>((conn, home))
    })
    .await
    .map_err(|e| e.to_string())??;
    let label = conn.label.clone();
    let conn_id = state.add_connection(conn);
    Ok(SftpConnected {
        conn_id,
        label,
        home,
    })
}

/// Dial, authenticate, and hang up — the dialog's Test button.
#[tauri::command]
pub async fn sftp_test(params: SftpConnectParams) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let auth = auth_of(&params);
        sftp::connect(&params.host, params.port, &params.username, &auth).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn sftp_disconnect(state: State<'_, AppState>, conn_id: u32) {
    state.connections.lock().unwrap().remove(&conn_id);
}
