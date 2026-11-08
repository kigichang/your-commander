//! Saved connection passwords in the platform credential store: the macOS
//! Keychain (generic passwords, same service/account layout as the old
//! security-framework code, so existing entries keep working) or the
//! Secret Service (GNOME Keyring / KWallet) on Linux.

use keyring::Entry;

const SERVICE: &str = "your-commander";

pub fn store(account: &str, password: &str) -> Result<(), String> {
    Entry::new(SERVICE, account)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn load(account: &str) -> Option<String> {
    Entry::new(SERVICE, account).ok()?.get_password().ok()
}

pub fn remove(account: &str) {
    if let Ok(entry) = Entry::new(SERVICE, account) {
        let _ = entry.delete_credential();
    }
}

/// Credential-store account key for an SSH login. Port 22 keeps the old
/// `ssh://user@host` form, so entries saved before ports counted still
/// work; any other port is part of the key, so two servers behind one
/// host (containers, port forwards) never share or swap a password.
/// An IPv6 host is bracketed so its colons cannot run into the port.
pub fn ssh_account(username: &str, host: &str, port: u16) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match port {
        22 => format!("ssh://{username}@{host}"),
        _ => format!("ssh://{username}@{host}:{port}"),
    }
}

/// Credential-store account key for an SMB share.
pub fn smb_account(username: &str, server: &str, share: &str) -> String {
    format!("smb://{username}@{server}/{share}")
}
