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
/// smbutil reads a password only from the URL argument or from its
/// controlling terminal. The URL would show it to every local user through
/// `ps`, so it runs on a pseudo-terminal and the password is typed at its
/// prompt instead.
pub fn list_shares(server: &str, username: &str, password: &str) -> Result<Vec<String>, String> {
    let mut url = String::from("//");
    if !username.is_empty() {
        url.push_str(&urlencoding::encode(username));
        url.push('@');
    }
    url.push_str(server);

    let output = if password.is_empty() {
        // Never try to prompt on a tty we don't have (guest/empty password).
        Command::new("/usr/bin/smbutil")
            .args(["view", "-N"])
            .arg(&url)
            .output()
            .map_err(|e| e.to_string())?
    } else {
        let mut cmd = Command::new("/usr/bin/smbutil");
        cmd.arg("view").arg(&url);
        run_with_password_prompt(cmd, password, std::time::Duration::from_secs(30))?
    };
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

/// Run `cmd` with a new pseudo-terminal as its controlling terminal (and
/// stdin), answer the first password prompt that appears there, and
/// collect its piped stdout/stderr. The password never touches argv or
/// the environment. If no prompt comes (keychain hit, early failure) the
/// command simply runs to completion; `timeout` bounds the wait for one.
fn run_with_password_prompt(
    mut cmd: Command,
    password: &str,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::sync::mpsc;

    let (mut master_fd, mut slave_fd) = (-1, -1);
    // SAFETY: openpty fills two descriptors we take ownership of below.
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(format!("openpty: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: both fds are fresh from openpty and owned only here.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master_fd), OwnedFd::from_raw_fd(slave_fd)) };
    // The child must not inherit the master side.
    // SAFETY: plain fcntl on a valid descriptor.
    unsafe { libc::fcntl(master_fd, libc::F_SETFD, libc::FD_CLOEXEC) };

    cmd.stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe calls between fork and exec. stdin is
    // already the pty slave here (std sets up stdio before pre_exec), so
    // a new session plus TIOCSCTTY makes it the controlling terminal that
    // /dev/tty — and so the password prompt — resolves to.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // Close our copy of the slave, or the master never sees EOF/EIO.
    drop(cmd);

    let mut writer = File::from(master.try_clone().map_err(|e| e.to_string())?);
    let mut reader = File::from(master);
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        // Ends with EIO once the child and its tty are gone.
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    // Wait for the prompt; readpassphrase flushes earlier input, so the
    // password cannot be typed ahead.
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left.min(std::time::Duration::from_millis(200))) {
            Ok(chunk) => {
                seen.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&seen).to_lowercase().contains("password") {
                    writer
                        .write_all(format!("{password}\n").as_bytes())
                        .map_err(|e| e.to_string())?;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                if left.is_zero() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Timed out waiting for the server".to_string());
                }
            }
        }
    }
    child.wait_with_output().map_err(|e| e.to_string())
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
    use super::{parse_view, run_with_password_prompt};
    use std::process::Command;
    use std::time::Duration;

    /// A stand-in for smbutil: prompts on /dev/tty like readpassphrase and
    /// prints what it read on stdout.
    fn prompting_cmd() -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.args([
            "-c",
            "exec </dev/tty; stty -echo; printf 'Password for nas: ' >/dev/tty; \
             IFS= read -r pw; echo \"got[$pw]\"; echo err >&2",
        ]);
        cmd
    }

    #[test]
    fn password_goes_in_at_the_tty_prompt() {
        let out = run_with_password_prompt(prompting_cmd(), "s3cr et%", Duration::from_secs(10))
            .unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "got[s3cr et%]\n");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
    }

    /// Against a real SMB server: `YC_TEST_SMB` = "server user password",
    /// offering a disk share named "media".
    #[test]
    fn lists_shares_from_live_server() {
        let Ok(spec) = std::env::var("YC_TEST_SMB") else {
            eprintln!("skipped: set YC_TEST_SMB=\"server user password\"");
            return;
        };
        let mut parts = spec.splitn(3, ' ');
        let (server, user, pw) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let shares = super::list_shares(server, user, pw).unwrap();
        assert!(shares.iter().any(|s| s == "media"), "{shares:?}");
        let err = super::list_shares(server, user, "wrong").unwrap_err();
        assert!(!err.contains("wrong"), "{err}");
    }

    #[test]
    fn no_prompt_runs_to_completion() {
        let mut cmd = Command::new("/bin/echo");
        cmd.arg("hi");
        let out = run_with_password_prompt(cmd, "pw", Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    }

    #[test]
    fn silent_command_times_out() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        let t = std::time::Instant::now();
        let err = run_with_password_prompt(cmd, "pw", Duration::from_millis(500)).unwrap_err();
        assert!(err.contains("Timed out"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

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
