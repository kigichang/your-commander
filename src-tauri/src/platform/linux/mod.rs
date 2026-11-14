//! Linux implementations (Ubuntu/GNOME is the reference target): GIO for
//! MIME and app lookup, org.freedesktop.FileManager1 over D-Bus for the
//! file manager, ffprobe for media metadata, libx264 for the repair encode,
//! the freedesktop icon theme for file icons.

use super::{MediaInfo, OpenApp, find_cli};
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod smb;

/// The home trash's files/ folder (freedesktop Trash spec). Created when
/// missing — a fresh account has none until something is trashed.
pub fn trash_dir() -> Result<PathBuf, String> {
    let dir = home_trash_files(
        std::env::var_os("XDG_DATA_HOME"),
        &crate::fs::local::home_dir(),
    );
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// `<Trash>/info/<name>.trashinfo` for an item in `<Trash>/files`.
fn trash_info_file(item: &Path) -> Result<PathBuf, String> {
    let name = item.file_name().ok_or("not in the Trash")?;
    let trash = item.parent().and_then(Path::parent).ok_or("not in the Trash")?;
    let mut info = name.to_os_string();
    info.push(".trashinfo");
    Ok(trash.join("info").join(info))
}

/// The original path from a .trashinfo file's `Path=` key (percent-encoded;
/// absolute for the home trash).
fn parse_trash_info(text: &str) -> Option<PathBuf> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[Trash Info]";
        } else if in_section {
            if let Some(v) = line.strip_prefix("Path=") {
                let bytes = urlencoding::decode_binary(v.as_bytes()).into_owned();
                use std::os::unix::ffi::OsStringExt;
                let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
                return path.is_absolute().then_some(path);
            }
        }
    }
    None
}

/// Where a Trash item came from, from its .trashinfo file.
pub fn trash_origin(item: &Path) -> Result<PathBuf, String> {
    let name = item.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let info = trash_info_file(item)?;
    let text = std::fs::read_to_string(&info)
        .map_err(|_| format!("{name}: original location unknown (no .trashinfo)"))?;
    parse_trash_info(&text).ok_or_else(|| format!("{name}: original location unknown"))
}

/// Remove the .trashinfo of an item that left the Trash.
pub fn trash_forget(item: &Path) {
    if let Ok(info) = trash_info_file(item) {
        let _ = std::fs::remove_file(info);
    }
}

/// Entries of Trash/files are all trashed items.
pub fn trash_is_bookkeeping(_name: &std::ffi::OsStr) -> bool {
    false
}

/// After emptying: clear orphaned .trashinfo files and the size cache.
pub fn trash_after_empty() {
    let Ok(files) = trash_dir() else { return };
    let Some(trash) = files.parent() else { return };
    if let Ok(rd) = std::fs::read_dir(trash.join("info")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "trashinfo") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::remove_file(trash.join("directorysizes"));
}

/// $XDG_DATA_HOME/Trash/files, ~/.local/share/Trash/files when it is unset
/// or empty — the same rule the `trash` crate uses when it moves files
/// there, so the button opens exactly where F8 puts things.
fn home_trash_files(xdg_data_home: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    xdg_data_home
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("Trash/files")
}

/// file:// URI with each path segment percent-encoded (quotes too, so the
/// result is safe to embed in a GVariant literal).
pub(crate) fn file_uri(path: &Path) -> String {
    let s = path.to_string_lossy();
    let encoded: Vec<String> = s
        .split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect();
    format!("file://{}", encoded.join("/"))
}

/// "New … Here" in the blank-space context menu: open a terminal or editor
/// in `dir`. `tool` is "terminal", "vscode", or "zed".
pub fn open_in(tool: &str, dir: &Path) -> Result<(), String> {
    let spawn =
        |mut cmd: Command| cmd.spawn().map(|_| ()).map_err(|e| e.to_string());
    match tool {
        "terminal" => {
            // x-terminal-emulator is Debian's alternatives link; the rest
            // cover the common desktops before the xterm fallback.
            for name in [
                "x-terminal-emulator",
                "gnome-terminal",
                "konsole",
                "xfce4-terminal",
                "xterm",
            ] {
                let Some(bin) = find_cli(name, &[]) else { continue };
                let mut cmd = Command::new(bin);
                match name {
                    "gnome-terminal" => {
                        cmd.arg(format!("--working-directory={}", dir.display()));
                    }
                    "konsole" => {
                        cmd.arg("--workdir").arg(dir);
                    }
                    _ => {
                        cmd.current_dir(dir);
                    }
                }
                return spawn(cmd);
            }
            Err("no terminal emulator found".to_string())
        }
        "vscode" => {
            let cli = find_cli("code", &["/usr/bin/code", "/snap/bin/code", "/usr/local/bin/code"])
                .ok_or("code not found")?;
            let mut cmd = Command::new(cli);
            // -n: always a new window, like the menu item says.
            cmd.arg("-n").arg(dir);
            spawn(cmd)
        }
        "zed" => {
            let cli = find_cli("zed", &["/usr/bin/zed", "/usr/local/bin/zed"])
                .or_else(|| find_cli("zeditor", &[]))
                .ok_or("zed not found")?;
            let mut cmd = Command::new(cli);
            cmd.arg(dir);
            spawn(cmd)
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Installed ffmpeg-based player to hand undecodable files to.
pub fn video_player() -> Option<String> {
    ["vlc", "mpv"]
        .iter()
        .find(|n| find_cli(n, &[]).is_some())
        .map(|n| n.to_string())
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    find_cli("ffmpeg", &["/usr/bin/ffmpeg", "/usr/local/bin/ffmpeg"])
}

/// Video codec arguments for the repair re-encode (software H.264; yuv420p
/// keeps 10-bit sources playable everywhere).
pub fn encode_args() -> &'static [&'static str] {
    &["-c:v", "libx264", "-crf", "23", "-preset", "veryfast", "-pix_fmt", "yuv420p"]
}

/// GIO's content type for the file ("gio info" asks GLib itself, so the
/// answer matches what "gio mime"/the desktop would do).
fn content_type(path: &Path) -> Option<String> {
    let out = Command::new("gio")
        .args(["info", "-a", "standard::content-type"])
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("standard::content-type:")
            .map(|v| v.trim().to_string())
    })
}

/// Apps registered for this file's MIME type, default handler first, the
/// rest alphabetical. Empty on any failure.
pub fn apps_for(path: &Path) -> Vec<OpenApp> {
    let Some(mime) = content_type(path) else {
        return vec![];
    };
    let Ok(out) = Command::new("gio")
        .args(["mime", &mime])
        .env("LC_ALL", "C")
        .output()
    else {
        return vec![];
    };
    let (default_id, ids) = parse_gio_mime(&String::from_utf8_lossy(&out.stdout));
    let mut apps: Vec<OpenApp> = ids
        .iter()
        .filter_map(|id| {
            let desktop = find_desktop_file(id)?;
            let name = desktop_entry_name(&std::fs::read_to_string(&desktop).ok()?)
                .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
            Some(OpenApp {
                name,
                path: desktop.to_string_lossy().to_string(),
                default: Some(id.as_str()) == default_id.as_deref(),
            })
        })
        .collect();
    apps.sort_by(|a, b| {
        (!a.default, a.name.to_lowercase()).cmp(&(!b.default, b.name.to_lowercase()))
    });
    apps
}

/// Parse `gio mime <type>` output: the default handler plus the registered
/// and recommended lists, deduplicated in order.
pub(crate) fn parse_gio_mime(text: &str) -> (Option<String>, Vec<String>) {
    let mut default_id = None;
    let mut ids: Vec<String> = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        if let Some(rest) = line.split_once(':').filter(|(h, _)| {
            h.starts_with("Default application for")
        }) {
            let id = rest.1.trim().to_string();
            if !id.is_empty() && id.ends_with(".desktop") {
                default_id = Some(id.clone());
                ids.push(id);
            }
            in_list = false;
            continue;
        }
        if line.ends_with("applications:") {
            in_list = true;
            continue;
        }
        if in_list {
            let id = line.trim();
            if id.is_empty() {
                in_list = false;
            } else if id.ends_with(".desktop") && !ids.iter().any(|e| e == id) {
                ids.push(id.to_string());
            }
        }
    }
    (default_id, ids)
}

/// Directories .desktop files live in, user first (matching GIO precedence).
fn applications_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data).join("applications"));
    } else if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    // Snap exports aren't always in XDG_DATA_DIRS.
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs.dedup();
    dirs
}

fn find_desktop_file(id: &str) -> Option<PathBuf> {
    applications_dirs()
        .into_iter()
        .map(|d| d.join(id))
        .find(|p| p.is_file())
}

/// Name= from a .desktop file's [Desktop Entry] section, preferring the
/// current locale ("zh_TW" tries Name[zh_TW], Name[zh], then Name).
pub(crate) fn desktop_entry_name(content: &str) -> Option<String> {
    let locale = std::env::var("LC_MESSAGES")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let locale = locale.split('.').next().unwrap_or("").to_string();
    let lang = locale.split('_').next().unwrap_or("").to_string();

    let mut name = None;
    let mut name_lang = None;
    let mut name_locale = None;
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Name" => name = Some(value.trim().to_string()),
            k if !locale.is_empty() && k == format!("Name[{locale}]") => {
                name_locale = Some(value.trim().to_string());
            }
            k if !lang.is_empty() && k == format!("Name[{lang}]") => {
                name_lang = Some(value.trim().to_string());
            }
            _ => {}
        }
    }
    name_locale.or(name_lang).or(name)
}

/// Launch a user-defined external tool. A value ending in ".desktop" (an id
/// or a path) goes through `gio launch`; anything else runs as a command.
pub fn open_with(app: &str, paths: &[PathBuf]) -> Result<(), String> {
    let output = if app.ends_with(".desktop") {
        let desktop = if Path::new(app).is_absolute() {
            PathBuf::from(app)
        } else {
            find_desktop_file(app).ok_or_else(|| format!("{app}: not found"))?
        };
        Command::new("gio")
            .arg("launch")
            .arg(&desktop)
            .args(paths)
            .output()
            .map_err(|e| e.to_string())?
    } else {
        return Command::new(app)
            .args(paths)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{app}: {e}"));
    };
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() { format!("gio launch \"{app}\" failed") } else { err })
    }
}

/// org.freedesktop.FileManager1 call with a list of file URIs (Nautilus,
/// Dolphin, and Nemo all implement it).
fn file_manager_call(method: &str, paths: &[PathBuf]) -> Result<(), String> {
    let uris: Vec<String> = paths.iter().take(10).map(|p| file_uri(p)).collect();
    let list = format!(
        "[{}]",
        uris.iter().map(|u| format!("'{u}'")).collect::<Vec<_>>().join(", ")
    );
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            &format!("org.freedesktop.FileManager1.{method}"),
            &list,
            "",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// The file manager's properties window for each path.
pub fn get_info(paths: &[PathBuf]) -> Result<(), String> {
    file_manager_call("ShowItemProperties", paths)
}

/// Select the file in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    file_manager_call("ShowItems", std::slice::from_ref(&path.to_path_buf()))
}

/// No portable tag store on Linux; tags stay empty.
pub fn file_tags(_path: &Path) -> Vec<String> {
    vec![]
}

/// Video metadata for Quick View via ffprobe; all-None when it's missing.
pub fn media_info(path: &Path) -> Result<MediaInfo, String> {
    let Some(ffprobe) = find_cli("ffprobe", &["/usr/bin/ffprobe", "/usr/local/bin/ffprobe"])
    else {
        return Ok(MediaInfo::default());
    };
    let out = crate::proc::output_bounded(
        Command::new(ffprobe)
            .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams"])
            .arg(path),
        super::PROBE_TIMEOUT,
        super::PROBE_OUTPUT_MAX,
    )?;
    Ok(parse_ffprobe(&String::from_utf8_lossy(&out.stdout)))
}

pub(crate) fn parse_ffprobe(json: &str) -> MediaInfo {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|d| *d > 0.0);
    let stream = v["streams"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["codec_type"] == "video"));
    MediaInfo {
        duration,
        width: stream.and_then(|s| s["width"].as_u64()).map(|w| w as u32),
        height: stream.and_then(|s| s["height"].as_u64()).map(|h| h as u32),
    }
}

/// Icon-theme names to try for a file-list key ("folder", "generic", or a
/// lower-cased extension), most specific first.
pub(crate) fn icon_names(key: &str) -> Vec<String> {
    match key {
        "folder" => vec!["folder".to_string()],
        "generic" | "" => vec!["text-x-generic".to_string()],
        ext => {
            let mut names = Vec::new();
            if let Some(mime) = mime_guess::from_ext(ext).first() {
                names.push(format!("{}-{}", mime.type_(), mime.subtype()).replace('/', "-"));
                // The theme's per-category fallback, e.g. image-x-generic.
                names.push(format!("{}-x-generic", mime.type_()));
            }
            names.push("text-x-generic".to_string());
            names
        }
    }
}

/// The user's icon theme, via gsettings (GNOME); themes fall back to
/// Adwaita/hicolor in the lookup itself. Asked once: icons are cached for
/// the app's lifetime anyway, so a theme switch shows after a restart.
fn icon_theme() -> &'static str {
    static THEME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    THEME.get_or_init(|| {
        Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "icon-theme"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Adwaita".to_string())
    })
}

/// PNG bytes of the theme icon for `key`; None sends the frontend to its
/// built-in icons. SVG results are skipped (the frontend wants raster).
pub fn icon_png(key: &str) -> Option<Vec<u8>> {
    let (key, size) = crate::icons::split_size(key);
    let theme = icon_theme();
    for name in icon_names(key) {
        for t in [theme, "Adwaita", "hicolor"] {
            let Some(p) = freedesktop_icons::lookup(&name)
                .with_theme(t)
                .with_size(size)
                .with_cache()
                .find()
            else {
                continue;
            };
            if p.extension().is_some_and(|e| e == "png")
                && let Ok(bytes) = std::fs::read(&p)
            {
                return Some(bytes);
            }
        }
    }
    None
}

/// No cloud placeholders to fetch on Linux (GVfs mounts read through).
pub fn allow_cloud_downloads() {}

/// See `allow_cloud_downloads`.
pub fn forbid_cloud_downloads_on_this_thread() {}

/// See `allow_cloud_downloads`.
pub fn is_cloud_placeholder(_path: &Path) -> bool {
    false
}

/// See `allow_cloud_downloads`.
pub fn download_cloud_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_trash_info() {
        let text = "[Trash Info]\nPath=/home/u/My%20Docs/%E4%B8%AD.txt\nDeletionDate=2026-11-06T20:30:47\n";
        assert_eq!(parse_trash_info(text), Some(PathBuf::from("/home/u/My Docs/中.txt")));
        assert_eq!(parse_trash_info("[Other]\nPath=/x\n"), None);
        assert_eq!(parse_trash_info("[Trash Info]\nPath=relative/x\n"), None);
        assert_eq!(
            trash_info_file(Path::new("/h/.local/share/Trash/files/a.txt")).unwrap(),
            PathBuf::from("/h/.local/share/Trash/info/a.txt.trashinfo")
        );
    }

    #[test]
    fn home_trash_files_follows_xdg_data_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            home_trash_files(None, home),
            PathBuf::from("/home/u/.local/share/Trash/files")
        );
        assert_eq!(
            home_trash_files(Some("".into()), home),
            PathBuf::from("/home/u/.local/share/Trash/files")
        );
        assert_eq!(
            home_trash_files(Some("/data".into()), home),
            PathBuf::from("/data/Trash/files")
        );
    }

    #[test]
    fn parses_gio_mime_output() {
        let text = "\
Default application for 'text/plain': org.gnome.TextEditor.desktop
Registered applications:
\torg.gnome.TextEditor.desktop
\tvim.desktop
\tcode.desktop
Recommended applications:
\torg.gnome.TextEditor.desktop
";
        let (default, ids) = parse_gio_mime(text);
        assert_eq!(default.as_deref(), Some("org.gnome.TextEditor.desktop"));
        assert_eq!(ids, vec!["org.gnome.TextEditor.desktop", "vim.desktop", "code.desktop"]);
    }

    #[test]
    fn gio_mime_no_handlers() {
        let (default, ids) = parse_gio_mime("No default applications for “application/x-foo”\n");
        assert_eq!(default, None);
        assert!(ids.is_empty());
    }

    #[test]
    fn desktop_name_plain() {
        let content = "[Desktop Entry]\nType=Application\nName=Text Editor\nName[zh_TW]=文字編輯器\n";
        // Result depends on the test environment's locale; both are valid.
        let name = desktop_entry_name(content).unwrap();
        assert!(name == "Text Editor" || name == "文字編輯器");
    }

    #[test]
    fn desktop_name_ignores_other_sections() {
        let content = "[Desktop Action new]\nName=New Window\n[Desktop Entry]\nName=Files\n";
        assert_eq!(desktop_entry_name(content).as_deref(), Some("Files"));
    }

    #[test]
    fn parses_ffprobe_json() {
        let json = r#"{
            "streams": [
                {"codec_type": "audio", "sample_rate": "48000"},
                {"codec_type": "video", "width": 1920, "height": 1080}
            ],
            "format": {"duration": "12.345"}
        }"#;
        let info = parse_ffprobe(json);
        assert_eq!(info.duration, Some(12.345));
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        let empty = parse_ffprobe("not json");
        assert_eq!(empty.duration, None);
    }

    #[test]
    fn file_uris_are_escaped() {
        assert_eq!(
            file_uri(Path::new("/home/kigi/My Files/a'b.txt")),
            "file:///home/kigi/My%20Files/a%27b.txt"
        );
    }

    #[test]
    fn icon_name_chain() {
        assert_eq!(icon_names("folder"), vec!["folder"]);
        let png = icon_names("png");
        assert_eq!(png[0], "image-png");
        assert_eq!(png[1], "image-x-generic");
        assert_eq!(icon_names("zzzunknown"), vec!["text-x-generic"]);
    }
}
