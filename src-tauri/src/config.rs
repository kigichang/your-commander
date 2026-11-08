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
    /// When the webview's decoder can't render a video, show a still frame
    /// extracted with ffmpeg instead of just the failure message.
    pub qv_ffmpeg_poster: bool,
    /// Column widths [ext, size, date] in px; Name takes the rest.
    pub left_cols: Vec<u32>,
    pub right_cols: Vec<u32>,
    pub window_size: Option<(f32, f32)>,
    /// Left panel's share of the pane area (0.2–0.8); 0.5 = even split.
    pub split_ratio: f32,
    /// UI font family; empty = the bundled Inter.
    pub font_family: String,
    /// Monospace font family (columns, paths, code); empty = JetBrains Mono.
    pub font_mono: String,
    /// Multiplier on every UI font size (and the list row heights).
    pub font_scale: f32,
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
            qv_ffmpeg_poster: false,
            left_cols: Vec::new(),
            right_cols: Vec::new(),
            window_size: None,
            split_ratio: 0.5,
            font_family: String::new(),
            font_mono: String::new(),
            font_scale: 1.0,
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
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = write_replacing(&path, text.as_bytes());
        }
    }
}

/// Write `bytes` to `path` all at once: into a sibling temp file, then
/// renamed over `path`. A crash or full disk mid-write leaves the old file
/// whole instead of a truncated one, which `load` would read as defaults.
fn write_replacing(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        // A leftover goes first, so create_new can hold: it never follows a
        // link at this name, and the mode applies to a file it made. The
        // config names servers and users: for this user's eyes only.
        let _ = std::fs::remove_file(&tmp);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_writes_replace_the_file_whole() {
        let dir = std::env::temp_dir().join(format!("yc_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        super::write_replacing(&path, b"first = 1\n").unwrap();
        super::write_replacing(&path, b"second = 2\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second = 2\n");
        assert!(!dir.join("config.toml.tmp").exists(), "no temp file left behind");

        // When the rename cannot happen (here: a folder holds the name),
        // the temp file is cleaned up and the error comes back.
        let blocked = dir.join("blocked.toml");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(super::write_replacing(&blocked, b"x").is_err());
        assert!(!dir.join("blocked.toml.tmp").exists());
        assert!(blocked.join("inside").is_dir(), "what was there is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_temp_file_is_private_and_never_followed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yc_cfg_link_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let victim = dir.join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("config.toml.tmp")).unwrap();

        super::write_replacing(&path, b"a = 1\n").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(std::fs::read(&path).unwrap(), b"a = 1\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
