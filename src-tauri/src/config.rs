use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Favorite {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ExternalTool {
    /// Menu label, e.g. "Extract with Keka".
    pub name: String,
    /// App name or .app path passed to `open -a`, e.g. "Keka".
    pub app: String,
    /// Comma-separated extensions ("zip,7z,rar"); empty = any file.
    pub extensions: String,
    /// Also offer the tool when folders are selected.
    pub dirs: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SmbConnection {
    pub name: String,
    pub server: String,
    pub share: String,
    pub username: String,
    /// Folder (relative to the mount) to land in after connecting.
    pub start_in: String,
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
    /// Authenticate through the running ssh-agent instead of key files.
    pub use_agent: bool,
    /// Remote folder to land in after connecting; empty = home.
    pub start_in: String,
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
            use_agent: false,
            start_in: String::new(),
            remember: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub smb_connections: Vec<SmbConnection>,
    pub ssh_connections: Vec<SshConnection>,
    pub language: Option<String>,
    /// UI theme: "system" (default), "dark", or "light".
    pub theme: String,
    pub show_hidden: bool,
    /// Flip the IOKit HIDFKeyMode while the app is focused so F1–F12 work
    /// without holding fn.
    pub fkeys_without_fn: bool,
    pub favorites: Vec<Favorite>,
    /// Remember open tabs across launches; off = both panes start at HOME.
    pub save_tabs: bool,
    pub left_path: Option<PathBuf>,
    pub right_path: Option<PathBuf>,
    pub left_view: Option<String>,
    pub right_view: Option<String>,
    /// Restored local tab paths and the active tab per side.
    pub left_tabs: Vec<PathBuf>,
    pub right_tabs: Vec<PathBuf>,
    pub left_active: usize,
    pub right_active: usize,
    /// Sort of the active tab, as "key:asc" / "key:desc".
    pub left_sort: Option<String>,
    pub right_sort: Option<String>,
    /// Hotkey preset name ("commander" / "finder") plus the user's overlay:
    /// chord → action id, "" meaning the chord is unbound.
    pub hotkey_preset: Option<String>,
    pub hotkeys: std::collections::HashMap<String, String>,
    /// Recently visited local folders, newest first (command palette).
    pub recent_dirs: Vec<PathBuf>,
    /// PlantUML rendering: "off" (default, nothing leaves the machine),
    /// "server" (encode + GET {server}/svg/…), or "jar" (local java).
    pub plantuml_mode: String,
    pub plantuml_server: String,
    pub plantuml_jar: String,
    /// Explicit java binary; empty = auto-detect (java_home, Homebrew, PATH).
    pub plantuml_java: String,
    /// Show "New VSCode Here" / "New Zed Here" in the context menu.
    pub menu_vscode: bool,
    pub menu_zed: bool,
    /// User-defined tools shown in the file context menu (`open -a`).
    pub external_tools: Vec<ExternalTool>,
    /// Quick View video: start playing automatically / without sound.
    pub qv_autoplay: bool,
    pub qv_mute: bool,
    /// Column widths [ext, size, date] in px; Name takes the rest.
    pub left_cols: Vec<u32>,
    pub right_cols: Vec<u32>,
    pub window_size: Option<(f32, f32)>,
    /// Left panel's share of the pane area (0.2–0.8); 0.5 = even split.
    pub split_ratio: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            smb_connections: Vec::new(),
            ssh_connections: Vec::new(),
            language: None,
            theme: "system".to_string(),
            show_hidden: false,
            fkeys_without_fn: true,
            favorites: Vec::new(),
            save_tabs: true,
            left_path: None,
            right_path: None,
            left_view: None,
            right_view: None,
            left_tabs: Vec::new(),
            right_tabs: Vec::new(),
            left_active: 0,
            right_active: 0,
            left_sort: None,
            right_sort: None,
            hotkey_preset: None,
            hotkeys: std::collections::HashMap::new(),
            recent_dirs: Vec::new(),
            plantuml_mode: "off".to_string(),
            plantuml_server: "https://www.plantuml.com/plantuml".to_string(),
            plantuml_jar: String::new(),
            plantuml_java: String::new(),
            menu_vscode: false,
            menu_zed: false,
            external_tools: Vec::new(),
            qv_autoplay: false,
            qv_mute: false,
            left_cols: Vec::new(),
            right_cols: Vec::new(),
            window_size: None,
            split_ratio: 0.5,
        }
    }
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
