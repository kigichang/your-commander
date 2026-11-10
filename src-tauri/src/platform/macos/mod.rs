//! macOS implementations: LaunchServices for app lookup, `open`/osascript
//! for launching and Finder integration, Spotlight importers for media
//! metadata, VideoToolbox for the repair encode.

use super::{MediaInfo, OpenApp, find_cli};
use std::path::{Path, PathBuf};

pub mod smb;
mod trash;

/// An autoreleased NSString (dock menu titles, icon lookups). Call with an
/// autorelease pool in place, i.e. on the main thread.
pub(crate) fn ns_string(s: &str) -> *mut objc2::runtime::AnyObject {
    use objc2::{class, msg_send};
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

/// The user's Trash folder. Listing it needs Full Disk Access: macOS
/// privacy protection answers EPERM ("Operation not permitted") otherwise.
pub fn trash_dir() -> Result<PathBuf, String> {
    Ok(crate::fs::local::home_dir().join(".Trash"))
}

/// Where a Trash item came from, from Finder's Put Back records.
pub fn trash_origin(item: &Path) -> Result<PathBuf, String> {
    trash::original_location(item)
}

/// Drop the Trash's bookkeeping for a restored or deleted item. Finder
/// prunes its own stale .DS_Store records, so nothing to do here.
pub fn trash_forget(_item: &Path) {}

/// Entries of the Trash folder that are bookkeeping, not trashed items.
pub fn trash_is_bookkeeping(name: &std::ffi::OsStr) -> bool {
    name == ".DS_Store"
}

/// After emptying: nothing beyond the items themselves on macOS.
pub fn trash_after_empty() {}

/// "New … Here" in the blank-space context menu: open a terminal or editor
/// in `dir`. `tool` is "terminal", "vscode", or "zed".
pub fn open_in(tool: &str, dir: &Path) -> Result<(), String> {
    let spawn =
        |mut cmd: std::process::Command| cmd.spawn().map(|_| ()).map_err(|e| e.to_string());
    match tool {
        "terminal" => {
            let mut cmd = std::process::Command::new("/usr/bin/open");
            cmd.arg("-a").arg("Terminal").arg(dir);
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

/// Double-click on a `.dmg`, Finder style: DiskImageMounter attaches it
/// (and handles verification, licence and password prompts itself), then
/// the mounted volume opens in a frontmost Finder window so the app inside
/// can be dragged to Applications. A plain `open` mounts the image but
/// leaves Finder in the background, so nothing visible seems to happen.
pub fn open_disk_image(path: &Path) -> Result<(), String> {
    let image = path.canonicalize().map_err(|e| e.to_string())?;
    let status = std::process::Command::new("/usr/bin/open")
        .arg("--")
        .arg(&image)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("Cannot open {}", path.display()));
    }
    // Mounting (and any prompt the user must answer) happens asynchronously
    // in DiskImageMounter; wait for the volume in the background.
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while std::time::Instant::now() < deadline {
            if let Some(mount) = disk_image_mount_point(&image) {
                let _ = std::process::Command::new("/usr/bin/open")
                    .arg("-a")
                    .arg("Finder")
                    .arg(&mount)
                    .status();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    });
    Ok(())
}

/// Where `image` is mounted, from `hdiutil info` (None while not attached).
fn disk_image_mount_point(image: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("/usr/bin/hdiutil")
        .arg("info")
        .output()
        .ok()?;
    parse_hdiutil_mount(&String::from_utf8_lossy(&out.stdout), image)
}

/// `hdiutil info` prints one `====`-separated block per attached image: an
/// `image-path : …` line, then tab-separated `dev  hint  mount-point` rows.
fn parse_hdiutil_mount(info: &str, image: &Path) -> Option<PathBuf> {
    let same = |p: &str| {
        let p = Path::new(p);
        p == image || p.canonicalize().is_ok_and(|c| c == image)
    };
    for block in info.split("\n====") {
        let mut lines = block.lines();
        let matches = lines.clone().any(|l| {
            l.split_once(':').is_some_and(|(k, v)| k.trim() == "image-path" && same(v.trim()))
        });
        if !matches {
            continue;
        }
        let mount = lines.find_map(|l| {
            let mount = l.split('\t').nth(2)?.trim();
            mount.starts_with('/').then(|| PathBuf::from(mount))
        });
        if mount.is_some() {
            return mount;
        }
    }
    None
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

// <sys/resource.h>; not wrapped by the libc crate.
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT: libc::c_int = 0;
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_ON: libc::c_int = 2;

unsafe extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
}

/// Let reads download cloud placeholders. Google Drive, iCloud Drive,
/// OneDrive and other File Provider folders keep not-yet-downloaded files
/// "dataless"; the system default refuses to fetch them for the reader and
/// fails with EDEADLK ("Resource deadlock avoided"). Called once at
/// startup, so Quick View, copy, move and compress fetch on demand the way
/// Finder does. Child processes (LibreOffice, ffmpeg) inherit it.
pub fn allow_cloud_downloads() {
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        );
    }
}

/// True for a cloud file not downloaded yet (Quick View offers a Download
/// button instead of fetching it just because the cursor passed by).
pub fn is_cloud_placeholder(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && crate::fs::local::is_dataless(&m))
}

/// Download a cloud placeholder: reading it through is what fetches it.
/// Blocks until the provider has the whole file on disk.
pub fn download_cloud_file(path: &Path) -> Result<(), String> {
    /// Downloads on for this thread whatever the process policy, back to
    /// following the process on drop (pool threads are reused).
    struct Allow;
    impl Drop for Allow {
        fn drop(&mut self) {
            unsafe {
                setiopolicy_np(
                    IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                    IOPOL_SCOPE_THREAD,
                    IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT,
                );
            }
        }
    }
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        );
    }
    let _allow = Allow;
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(err)?;
    std::io::copy(&mut file, &mut std::io::sink()).map_err(err)?;
    Ok(())
}

/// Keep the calling thread from downloading cloud placeholders: a content
/// search over a cloud folder must not fetch the whole drive. Reads of a
/// placeholder then fail and the file is skipped. Only for a thread the
/// caller owns (not a shared pool thread): it lasts for the thread's life.
pub fn forbid_cloud_downloads_on_this_thread() {
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
        );
    }
}

#[cfg(test)]
mod cloud_policy_tests {
    use super::*;

    unsafe extern "C" {
        fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
    }

    fn policy(scope: libc::c_int) -> libc::c_int {
        unsafe { getiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, scope) }
    }

    #[test]
    fn process_downloads_while_a_search_thread_does_not() {
        allow_cloud_downloads();
        assert_eq!(policy(IOPOL_SCOPE_PROCESS), IOPOL_MATERIALIZE_DATALESS_FILES_ON);
        let on_search_thread = std::thread::spawn(|| {
            forbid_cloud_downloads_on_this_thread();
            policy(IOPOL_SCOPE_THREAD)
        })
        .join()
        .unwrap();
        assert_eq!(on_search_thread, IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
        // Other threads keep following the process policy.
        assert_eq!(policy(IOPOL_SCOPE_PROCESS), IOPOL_MATERIALIZE_DATALESS_FILES_ON);
    }

    #[test]
    fn a_local_file_is_no_placeholder() {
        let file = std::env::temp_dir().join(format!("yc-cloud-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        assert!(!is_cloud_placeholder(&file));
        assert!(download_cloud_file(&file).is_ok());
        std::fs::remove_file(&file).unwrap();
    }

    /// `YC_CLOUD_FILE=<a not-yet-downloaded Drive/iCloud file> cargo test --
    /// --ignored cloud`: downloads it, even on a thread that forbids it.
    #[test]
    #[ignore = "needs a cloud placeholder file"]
    fn cloud_placeholder_downloads_on_request() {
        let path = PathBuf::from(std::env::var("YC_CLOUD_FILE").expect("YC_CLOUD_FILE"));
        assert!(is_cloud_placeholder(&path), "already downloaded");
        let listed = |path: &Path| {
            crate::fs::local::list_dir(path.parent().unwrap())
                .unwrap()
                .into_iter()
                .find(|e| e.path == path)
                .unwrap()
                .is_cloud
        };
        assert!(listed(&path), "the listing marks it");
        std::thread::spawn(move || {
            forbid_cloud_downloads_on_this_thread();
            assert!(std::fs::read(&path).is_err());
            download_cloud_file(&path).unwrap();
            assert!(!is_cloud_placeholder(&path));
            assert!(!listed(&path));
            // The thread is back to its own policy afterwards.
            assert_eq!(policy(IOPOL_SCOPE_THREAD), IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT);
        })
        .join()
        .unwrap();
    }
}

#[cfg(test)]
mod disk_image_tests {
    use super::parse_hdiutil_mount;
    use std::path::{Path, PathBuf};

    const INFO: &str = "framework       : 710\n\
driver          : 710\n\
================================================\n\
image-path      : /Users/me/Downloads/Other.dmg\n\
image-type      : read-only disk image\n\
/dev/disk4\tGUID_partition_scheme\t\n\
/dev/disk4s1\t48465300-0000-11AA-AA11-00306543ECAC\t/Volumes/Other\n\
================================================\n\
image-path      : /Users/me/Downloads/My App 1.0.dmg\n\
image-type      : read-only disk image\n\
/dev/disk5\tGUID_partition_scheme\t\n\
/dev/disk5s1\t48465300-0000-11AA-AA11-00306543ECAC\t/Volumes/My App\n";

    #[test]
    fn finds_mount_point_of_matching_image() {
        assert_eq!(
            parse_hdiutil_mount(INFO, Path::new("/Users/me/Downloads/My App 1.0.dmg")),
            Some(PathBuf::from("/Volumes/My App"))
        );
    }

    #[test]
    fn none_when_image_not_attached() {
        assert_eq!(parse_hdiutil_mount(INFO, Path::new("/Users/me/Missing.dmg")), None);
    }
}
