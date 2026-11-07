use crate::keychain;
use crate::platform::smb;
use std::path::PathBuf;
use super::blocking;

/// A server is a host name, an IPv4 address or a bracketed IPv6 literal,
/// optionally with `:port`. Nothing that could reshape the URL it is
/// pasted into (`/`, `@`, `?`, `#`, spaces) or read as an option (`-`).
fn check_server(server: &str) -> Result<(), String> {
    let ok = !server.is_empty()
        && !server.starts_with('-')
        && server
            .chars()
            .all(|c| c.is_alphanumeric() || ".-_:[]%".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!("Invalid server: \"{}\"", server.escape_debug()))
    }
}

/// Shares, user names and passwords go into URLs, argv and line-based
/// stdin/tty prompts: a control character (a newline above all) would
/// split or end a field, or act as a terminal key on the password prompt.
fn check_text(what: &str, value: &str) -> Result<(), String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{what} contains a control character"));
    }
    Ok(())
}

/// Validate what the connect dialog sends before any of it reaches
/// smbutil, smbclient, gio or osascript. `share` may be empty (pick from
/// the server's list).
pub(crate) fn check_params(
    server: &str,
    share: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    check_server(server)?;
    check_text("Share", share)?;
    if share.contains(['/', '\\']) {
        return Err(format!("Invalid share: \"{}\"", share.escape_debug()));
    }
    check_text("User name", username)?;
    check_text("Password", password)
}

/// The dialog's Test button: is anything listening on the SMB port?
#[tauri::command]
pub async fn smb_test(server: String) -> Result<(), String> {
    check_server(&server)?;
    blocking(move || {
        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Duration;
        let addrs = format!("{server}:445")
            .to_socket_addrs()
            .map_err(|e| format!("{server}: {e}"))?
            .collect::<Vec<_>>();
        let addr = addrs.first().ok_or_else(|| format!("{server}: no address"))?;
        TcpStream::connect_timeout(addr, Duration::from_secs(4))
            .map(|_| ())
            .map_err(|e| format!("{server}: {e}"))
    })
    .await
}

/// Disk shares the server offers (share picker when none was given).
#[tauri::command]
pub async fn smb_shares(
    server: String,
    username: String,
    password: String,
) -> Result<Vec<String>, String> {
    check_params(&server, "", &username, &password)?;
    blocking(move || smb::list_shares(&server, &username, &password)).await
}

#[tauri::command]
pub async fn smb_unmount(path: PathBuf) -> Result<(), String> {
    blocking(move || smb::unmount(&path)).await
}

/// Stored password for a saved SMB connection (Keychain).
#[tauri::command]
pub fn smb_saved_password(username: String, server: String, share: String) -> Option<String> {
    keychain::load(&keychain::smb_account(&username, &server, &share))
}

#[tauri::command]
pub fn smb_store_password(
    username: String,
    server: String,
    share: String,
    password: String,
) -> Result<(), String> {
    keychain::store(&keychain::smb_account(&username, &server, &share), &password)
}

#[tauri::command]
pub fn smb_forget_password(username: String, server: String, share: String) {
    keychain::remove(&keychain::smb_account(&username, &server, &share));
}

#[cfg(test)]
mod tests {
    use super::check_params;

    #[test]
    fn accepts_ordinary_targets() {
        for server in ["nas", "nas.local", "192.168.1.5", "[fe80::1%en0]", "nas:445", "伺服器"] {
            assert!(check_params(server, "", "", "").is_ok(), "{server}");
        }
        assert!(check_params("nas", "My Docs", "DOMAIN\\kigi", "p@ss w0rd;%").is_ok());
        assert!(check_params("nas", "", "kigi@corp", "").is_ok());
    }

    #[test]
    fn refuses_reshaping_or_control_characters() {
        for server in ["", "-oProxy", "u@nas", "nas/share", "nas share", "nas?x", "nas#x", "nas\n"] {
            assert!(check_params(server, "", "", "").is_err(), "{server:?}");
        }
        assert!(check_params("nas", "a/b", "", "").is_err());
        assert!(check_params("nas", "a\\b", "", "").is_err());
        assert!(check_params("nas", "", "kigi\nWORKGROUP", "").is_err());
        assert!(check_params("nas", "", "kigi", "pw\nmore").is_err());
        assert!(check_params("nas", "", "kigi", "pw\u{3}").is_err());
    }
}
