use security_framework::passwords;

const SERVICE: &str = "your-commander";

pub fn store(account: &str, password: &str) -> Result<(), String> {
    passwords::set_generic_password(SERVICE, account, password.as_bytes())
        .map_err(|e| e.to_string())
}

pub fn load(account: &str) -> Option<String> {
    passwords::get_generic_password(SERVICE, account)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

pub fn remove(account: &str) {
    let _ = passwords::delete_generic_password(SERVICE, account);
}

/// Keychain account key for an SMB share.
pub fn smb_account(username: &str, server: &str, share: &str) -> String {
    format!("smb://{username}@{server}/{share}")
}
