//! macOS implementations: LaunchServices for app lookup, `open`/osascript
//! for launching and Finder integration, Spotlight importers for media
//! metadata, VideoToolbox for the repair encode.

use super::{MediaInfo, OpenApp, find_cli};
use std::path::{Path, PathBuf};

pub mod smb;

/// "New … Here" in the blank-space context menu: open a terminal or editor
/// in `dir`. `tool` is "terminal", "herdr", "vscode", or
/// "zed".
pub fn open_in(tool: &str, dir: &Path) -> Result<(), String> {
    let spawn =
        |mut cmd: std::process::Command| cmd.spawn().map(|_| ()).map_err(|e| e.to_string());
    match tool {
        "terminal" => {
            let mut cmd = std::process::Command::new("/usr/bin/open");
            cmd.arg("-a").arg("Terminal").arg(dir);
            spawn(cmd)
        }
        "herdr" => {
            super::herdr_shell(dir)?;
            let mut cmd = std::process::Command::new("/usr/bin/open");
            cmd.arg("-a").arg("Herdr");
            spawn(cmd)
        }
        "vscode" => {
            let cli = find_cli(
                "code",
                &[
                    "/opt/homebrew/bin/code",
                    "/usr/local/bin/code",
                    "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                ],
            );
            match cli {
                // -n: always a new window, like the menu item says.
                Some(code) => {
                    let mut cmd = std::process::Command::new(code);
                    cmd.arg("-n").arg(dir);
                    spawn(cmd)
                }
                None => {
                    let mut cmd = std::process::Command::new("/usr/bin/open");
                    cmd.arg("-na").arg("Visual Studio Code").arg("--args").arg(dir);
                    spawn(cmd)
                }
            }
        }
        "zed" => {
            let cli = find_cli(
                "zed",
                &[
                    "/opt/homebrew/bin/zed",
                    "/usr/local/bin/zed",
                    "/Applications/Zed.app/Contents/MacOS/cli",
                ],
            );
            match cli {
                Some(zed) => {
                    let mut cmd = std::process::Command::new(zed);
                    cmd.arg(dir);
                    spawn(cmd)
                }
                None => {
                    let mut cmd = std::process::Command::new("/usr/bin/open");
                    cmd.arg("-a").arg("Zed").arg(dir);
                    spawn(cmd)
                }
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// Installed ffmpeg-based player to hand undecodable files to.
pub fn video_player() -> Option<String> {
    let app_exists = |name: &str| {
        PathBuf::from(format!("/Applications/{name}.app")).exists()
            || crate::fs::local::home_dir()
                .join(format!("Applications/{name}.app"))
                .exists()
    };
    ["VLC", "IINA"].iter().find(|n| app_exists(n)).map(|n| n.to_string())
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    find_cli("ffmpeg", &["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"])
}

/// Video codec arguments for the repair re-encode (hardware H.264).
pub fn encode_args() -> &'static [&'static str] {
    &["-c:v", "h264_videotoolbox", "-q:v", "65"]
}

/// Apps LaunchServices knows can open this file, default handler first,
/// the rest alphabetical. Empty on any failure.
pub fn apps_for(path: &Path) -> Vec<OpenApp> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::url::{CFURL, CFURLRef};

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyApplicationURLsForURL(in_url: CFURLRef, roles: u32) -> CFArrayRef;
        fn LSCopyDefaultApplicationURLForURL(
            in_url: CFURLRef,
            roles: u32,
            out_error: *mut std::ffi::c_void,
        ) -> CFURLRef;
    }
    const ROLES_ALL: u32 = 0xFFFF_FFFF;

    let Some(url) = CFURL::from_path(path, false) else {
        return vec![];
    };
    let default_path = unsafe {
        let r = LSCopyDefaultApplicationURLForURL(
            url.as_concrete_TypeRef(),
            ROLES_ALL,
            std::ptr::null_mut(),
        );
        if r.is_null() {
            None
        } else {
            CFURL::wrap_under_create_rule(r).to_path()
        }
    };
    let mut out: Vec<OpenApp> = vec![];
    unsafe {
        let arr = LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), ROLES_ALL);
        if arr.is_null() {
            return vec![];
        }
        let arr: CFArray<CFURL> = CFArray::wrap_under_create_rule(arr);
        for app_url in arr.iter() {
            let Some(p) = app_url.to_path() else { continue };
            let path_str = p.to_string_lossy().to_string();
            if out.iter().any(|a| a.path == path_str) {
                continue;
            }
            let Some(name) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            let default = default_path.as_deref() == Some(p.as_path());
            out.push(OpenApp { name, path: path_str, default });
        }
    }
    out.sort_by(|a, b| {
        (!a.default, a.name.to_lowercase()).cmp(&(!b.default, b.name.to_lowercase()))
    });
    out
}

/// Launch a user-defined external tool: `open -a <app> <paths...>`.
/// Waits for `open` to exit (it returns right after handing the files to
/// LaunchServices) so a missing app surfaces as an error instead of nothing.
pub fn open_with(app: &str, paths: &[PathBuf]) -> Result<(), String> {
    let output = std::process::Command::new("/usr/bin/open")
        .arg("-a")
        .arg(app)
        .args(paths)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() { format!("open -a \"{app}\" failed") } else { err })
    }
}

/// Finder's Get Info window for each path (local only, capped so a huge
/// mark set can't flood the screen with windows).
pub fn get_info(paths: &[PathBuf]) -> Result<(), String> {
    let mut script = String::from("tell application \"Finder\"\nactivate\n");
    for p in paths.iter().take(10) {
        let s = p
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        script.push_str(&format!(
            "open information window of (POSIX file \"{s}\" as alias)\n"
        ));
    }
    script.push_str("end tell\n");
    let mut child = std::process::Command::new("/usr/bin/osascript")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().ok_or("osascript: no stdin")?;
        stdin.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Select the file in a new Finder window.
pub fn reveal(path: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("open -R failed".to_string())
    }
}

/// Finder tags of a file, via `mdls` (best effort; empty on failure).
pub fn file_tags(path: &Path) -> Vec<String> {
    let parse = || -> Option<Vec<String>> {
        let output = std::process::Command::new("/usr/bin/mdls")
            .args(["-raw", "-name", "kMDItemUserTags"])
            .arg(path)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim() == "(null)" {
            return None;
        }
        // Output shape: (\n    Tag1,\n    Tag2\n)
        let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
        Some(
            inner
                .lines()
                .map(|l| l.trim().trim_end_matches(',').trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        )
    };
    parse().unwrap_or_default()
}

/// Video metadata for Quick View, local files only. `mdimport -t -d2` runs
/// the Spotlight importer directly, so it works on unindexed volumes too
/// (`mdls` only reads the index). Missing attributes come back as None.
pub fn media_info(path: &Path) -> Result<MediaInfo, String> {
    let out = std::process::Command::new("/usr/bin/mdimport")
        .args(["-t", "-d2"])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    // The attribute dump goes to stderr on some macOS versions.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let num = |attr: &str| -> Option<f64> {
        text.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(attr)?.trim_start();
            let val = rest.strip_prefix('=')?.trim().trim_end_matches(';').trim();
            val.trim_matches('"').parse::<f64>().ok()
        })
    };
    Ok(MediaInfo {
        duration: num("kMDItemDurationSeconds"),
        width: num("kMDItemPixelWidth").map(|v| v as u32),
        height: num("kMDItemPixelHeight").map(|v| v as u32),
    })
}
