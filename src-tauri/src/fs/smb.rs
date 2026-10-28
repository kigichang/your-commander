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
    // Feed the script through stdin: as an argument the URL (password
    // included) would be visible to every local process via `ps`.
    let mut child = Command::new("/usr/bin/osascript")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().ok_or("osascript: no stdin")?;
        stdin.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Never echo the URL (it contains the password); osascript errors don't.
        return Err(if err.is_empty() {
            "Could not mount the share".to_string()
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
    Err("Could not mount the share".to_string())
}

/// List the disk shares a server offers, via `smbutil view`.
/// smbutil only takes credentials inside the URL argument, so they are
/// briefly visible to local `ps` while it runs; macOS offers no stdin path.
pub fn list_shares(server: &str, username: &str, password: &str) -> Result<Vec<String>, String> {
    let mut url = String::from("//");
    if !username.is_empty() {
        url.push_str(&urlencoding::encode(username));
        if !password.is_empty() {
            url.push(':');
            url.push_str(&urlencoding::encode(password));
        }
        url.push('@');
    }
    url.push_str(server);

    let mut cmd = Command::new("/usr/bin/smbutil");
    cmd.arg("view");
    if password.is_empty() {
        // Never try to prompt on a tty we don't have (guest/empty password).
        cmd.arg("-N");
    }
    let output = cmd.arg(&url).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let mut err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            err = "Could not list shares".to_string();
        }
        // Defense in depth: never echo the password, encoded or not.
        if !password.is_empty() {
            err = err
                .replace(&urlencoding::encode(password).to_string(), "•••")
                .replace(password, "•••");
        }
        return Err(err);
    }
    Ok(parse_view(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse `smbutil view` output: a `Share … Type … Comments` header, a dashed
/// rule, fixed-width rows, then a `N shares listed` footer.
fn parse_view(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    let Some(type_col) = lines.find_map(|l| {
        (l.contains("Share") && l.contains("Type")).then(|| l.find("Type"))?
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in lines {
        if line.len() <= type_col || line.starts_with('-') {
            continue;
        }
        // Column offsets are bytes; back off to a char boundary just in case.
        let mut col = type_col;
        while col > 0 && !line.is_char_boundary(col) {
            col -= 1;
        }
        let (name, rest) = line.split_at(col);
        if rest.split_whitespace().next() == Some("Disk") {
            let name = name.trim_end();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::parse_view;

    #[test]
    fn parses_smbutil_view_output() {
        let text = "\
Share                                           Type    Comments
-------------------------------
homes                                           Disk    user home directories
My Photos                                       Disk
IPC$                                            Pipe    IPC Service
LaserJet                                        Printer office

3 shares listed
";
        assert_eq!(parse_view(text), vec!["homes", "My Photos"]);
    }

    #[test]
    fn parse_view_tolerates_garbage() {
        assert!(parse_view("").is_empty());
        assert!(parse_view("no header at all\nDisk\n").is_empty());
    }
}
