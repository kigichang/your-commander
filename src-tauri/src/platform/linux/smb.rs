//! SMB over GVfs: `gio mount smb://…` mounts land under
//! $XDG_RUNTIME_DIR/gvfs as FUSE paths shared with GNOME Files, one
//! directory per mount named `smb-share:server=…,share=…`. Share listing
//! goes through smbclient. Credentials travel over stdin — never argv,
//! where every local process could read them from `ps`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// gio and smbclient print a few lines; anything near this is not them.
const OUTPUT_MAX: u64 = 4 * 1024 * 1024;
/// Mounting waits on the server; one that never answers must not hold the
/// worker (and the Connect dialog) for good.
const MOUNT_TIMEOUT: Duration = Duration::from_secs(60);
/// Listing shares.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// The directory GVfs exposes FUSE mounts in (also read by the drives
/// menu, `fs::local::volumes`).
pub fn gvfs_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
        .join("gvfs")
}

/// `smb-share:server=nas,share=media[,user=kigi]` → (server, share), both
/// %XX-decoded. Anything else (sftp:…, mtp:…) is None.
pub fn parse_smb_dir(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("smb-share:")?;
    let mut server = None;
    let mut share = None;
    for kv in rest.split(',') {
        let (key, value) = kv.split_once('=')?;
        let value = urlencoding::decode(value)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| value.to_string());
        match key {
            "server" => server = Some(value),
            "share" => share = Some(value),
            _ => {}
        }
    }
    Some((server?, share?))
}

/// The drives-menu name of a GVfs mount directory: "media on nas" for
/// `smb-share:server=nas,share=media`; None for anything not SMB.
pub fn display_name(dir: &str) -> Option<String> {
    let (server, share) = parse_smb_dir(dir)?;
    Some(format!("{share} on {server}"))
}

/// Where is `//server/share` mounted, if at all?
pub fn find_mount(server: &str, share: &str) -> Option<PathBuf> {
    let server = server.to_lowercase();
    let share = share.to_lowercase();
    for entry in std::fs::read_dir(gvfs_dir()).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some((s, sh)) = parse_smb_dir(&name)
            && s.to_lowercase() == server
            && sh.to_lowercase() == share
        {
            return Some(entry.path());
        }
    }
    None
}

fn smb_url(server: &str, share: &str) -> String {
    format!("smb://{server}/{}", urlencoding::encode(share))
}

/// Strip the password out of an error before it reaches the UI or a log.
fn scrub(mut err: String, password: &str) -> String {
    if !password.is_empty() {
        err = err
            .replace(&urlencoding::encode(password).to_string(), "•••")
            .replace(password, "•••");
    }
    err
}

/// Mount an SMB share via `gio mount`, which shares the mount with GNOME
/// Files. Blocking: run inside an op worker. gio prompts on stdin when the
/// keyring has no credentials: user, domain (blank = default), password.
pub fn mount(
    server: &str,
    share: &str,
    username: &str,
    password: &str,
) -> Result<PathBuf, String> {
    if let Some(existing) = find_mount(server, share) {
        return Ok(existing);
    }

    let mut cmd = Command::new("gio");
    cmd.arg("mount");
    if username.is_empty() {
        cmd.arg("--anonymous");
    }
    let mut child = cmd
        .arg(smb_url(server, share))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if !username.is_empty()
        && let Some(stdin) = child.stdin.as_mut()
    {
        // gio may exit without prompting (keyring hit) — a broken pipe here
        // is fine.
        let _ = write!(stdin, "{username}\n\n{password}\n");
    }
    drop(child.stdin.take());
    let output = crate::proc::wait_bounded(child, MOUNT_TIMEOUT, OUTPUT_MAX)?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "Could not mount the share".to_string()
        } else {
            scrub(err, password)
        });
    }

    // The FUSE path appears under gvfs a moment later.
    for _ in 0..20 {
        if let Some(path) = find_mount(server, share) {
            return Ok(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err("Could not mount the share".to_string())
}

/// List the disk shares a server offers, via `smbclient -L -g` (machine-
/// readable lines). The password goes in on stdin.
pub fn list_shares(server: &str, username: &str, password: &str) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("smbclient");
    cmd.arg("-L").arg(format!("//{server}")).args(["-g", "-m", "SMB3"]);
    if username.is_empty() {
        cmd.arg("-N");
    } else {
        // One argument, so a name can never be read as another option.
        cmd.arg(format!("--user={username}"));
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "smbclient not found — install the smbclient package".to_string()
            } else {
                e.to_string()
            }
        })?;
    if !username.is_empty()
        && let Some(stdin) = child.stdin.as_mut()
    {
        let _ = writeln!(stdin, "{password}");
    }
    drop(child.stdin.take());
    let output = crate::proc::wait_bounded(child, LIST_TIMEOUT, OUTPUT_MAX)?;
    if !output.status.success() {
        let mut err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            err = "Could not list shares".to_string();
        }
        return Err(scrub(err, password));
    }
    Ok(parse_share_list(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `smbclient -L -g` output: one `Type|Name|Comment` line per entry.
fn parse_share_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            if parts.next()? != "Disk" {
                return None;
            }
            let name = parts.next()?.trim();
            // Trailing '$' marks administrative shares (print$, C$) that
            // Windows hides; smbclient lists them anyway.
            (!name.is_empty() && !name.ends_with('$')).then(|| name.to_string())
        })
        .collect()
}

/// Unmount a GVfs SMB path by its smb:// URI; any other path (a USB stick
/// in /media, say) unmounts as a file:// location, which gio routes through
/// udisks2 like the file manager's eject button.
pub fn unmount(path: &Path) -> Result<(), String> {
    let uri = path
        .file_name()
        .and_then(|n| parse_smb_dir(&n.to_string_lossy()))
        .filter(|_| path.parent() == Some(gvfs_dir().as_path()))
        .map(|(server, share)| smb_url(&server, &share))
        .unwrap_or_else(|| super::file_uri(path));
    let output = Command::new("gio")
        .args(["mount", "-u", &uri])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{display_name, parse_share_list, parse_smb_dir};

    #[test]
    fn parses_gvfs_dir_names() {
        assert_eq!(
            parse_smb_dir("smb-share:server=nas,share=media"),
            Some(("nas".into(), "media".into()))
        );
        assert_eq!(
            parse_smb_dir("smb-share:server=nas,share=my%20docs,user=kigi"),
            Some(("nas".into(), "my docs".into()))
        );
        assert_eq!(parse_smb_dir("sftp:host=box,user=kigi"), None);
        assert_eq!(parse_smb_dir("smb-share:server=nas"), None);
    }

    #[test]
    fn names_mounts_for_the_drives_menu() {
        assert_eq!(display_name("smb-share:server=nas,share=media").as_deref(), Some("media on nas"));
        // Decoded like find_mount sees them, not "My%20Photos on nas".
        assert_eq!(
            display_name("smb-share:server=nas,share=My%20Photos,user=kigi").as_deref(),
            Some("My Photos on nas")
        );
        assert_eq!(
            display_name("smb-share:server=nas,share=%E5%BD%B1%E7%89%87").as_deref(),
            Some("影片 on nas")
        );
        assert_eq!(display_name("sftp:host=box"), None);
        assert_eq!(display_name("smb-share:server=nas"), None);
    }

    #[test]
    fn parses_smbclient_list() {
        // smbclient -g prints Type|Name|Comment.
        let text = "\
Disk|homes|user home directories
Disk|My Photos|
Disk|print$|Printer Drivers
IPC|IPC$|IPC Service
Printer|LaserJet|office
";
        assert_eq!(parse_share_list(text), vec!["homes", "My Photos"]);
    }

    #[test]
    fn share_list_tolerates_garbage() {
        assert!(parse_share_list("").is_empty());
        assert!(parse_share_list("no pipes here\n").is_empty());
    }
}
