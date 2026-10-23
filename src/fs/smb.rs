use rust_i18n::t;
use std::path::PathBuf;
use std::process::Command;

/// Where is `//server/share` mounted, if at all? Parses `mount` output lines
/// like `//user@server/share on /Volumes/share (smbfs, ...)`.
pub fn find_mount(server: &str, share: &str) -> Option<PathBuf> {
    let output = Command::new("/sbin/mount").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let server_lower = server.to_lowercase();
    let share_lower = share.to_lowercase();
    for line in text.lines() {
        if !line.contains("(smbfs") {
            continue;
        }
        let Some((spec, rest)) = line.split_once(" on ") else {
            continue;
        };
        let spec = spec.to_lowercase();
        // spec: //user@server/share or //server/share (URL-encoded possible)
        let Some(tail) = spec.strip_prefix("//") else {
            continue;
        };
        let Some((host, mount_share)) = tail.rsplit_once('/') else {
            continue;
        };
        let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
        let decoded_share = urlencoding::decode(mount_share)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| mount_share.to_string());
        if host == server_lower && decoded_share == share_lower {
            let mountpoint = rest.split(" (").next().unwrap_or(rest);
            return Some(PathBuf::from(mountpoint));
        }
    }
    None
}

/// Mount an SMB share via NetFS (AppleScript `mount volume`), which lands in
/// /Volumes and is shared with Finder. Blocking: run inside an op worker.
pub fn mount(
    server: &str,
    share: &str,
    username: &str,
    password: &str,
) -> Result<PathBuf, String> {
    if let Some(existing) = find_mount(server, share) {
        return Ok(existing);
    }

    let mut url = String::from("smb://");
    if !username.is_empty() {
        url.push_str(&urlencoding::encode(username));
        if !password.is_empty() {
            url.push(':');
            url.push_str(&urlencoding::encode(password));
        }
        url.push('@');
    }
    url.push_str(server);
    url.push('/');
    url.push_str(&urlencoding::encode(share));

    // AppleScript string literal: escape backslash and double quote.
    let escaped = url.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("mount volume \"{escaped}\"");
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Never echo the URL (it contains the password); osascript errors don't.
        return Err(if err.is_empty() {
            t!("status.mount_failed").to_string()
        } else {
            err
        });
    }

    // The volume appears under /Volumes a moment later.
    for _ in 0..20 {
        if let Some(path) = find_mount(server, share) {
            return Ok(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(t!("status.mount_failed").to_string())
}

pub fn unmount(path: &std::path::Path) -> Result<(), String> {
    let output = Command::new("/usr/sbin/diskutil")
        .arg("unmount")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}
