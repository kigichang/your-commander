use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Favorite {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SmbConnection {
    pub name: String,
    pub server: String,
    pub share: String,
    pub username: String,
    pub remember: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SshConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Private key path; empty means try the default ~/.ssh keys.
    pub identity_file: String,
    /// true: password auth, false: public key auth.
    pub use_password: bool,
    pub remember: bool,
}

impl Default for SshConnection {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            identity_file: String::new(),
            use_password: false,
            remember: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub smb_connections: Vec<SmbConnection>,
    pub ssh_connections: Vec<SshConnection>,
    pub language: Option<String>,
    pub show_hidden: bool,
    pub favorites: Vec<Favorite>,
    pub left_path: Option<PathBuf>,
    pub right_path: Option<PathBuf>,
    pub left_view: Option<String>,
    pub right_view: Option<String>,
    pub window_size: Option<(f32, f32)>,
}

impl Config {
    fn file_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("your-commander").join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::file_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::file_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, text);
        }
    }
}
