mod commands;
mod config;
mod fs;
mod keychain;
mod ops;
mod state;
mod watch;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::default();
    *app_state.config.lock().unwrap() = config::Config::load();
    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::mkdir,
            commands::fs::rename,
            commands::fs::exists,
            commands::system::volumes,
            commands::system::home_dir,
            commands::system::open_path,
            commands::system::reveal_in_finder,
            commands::system::is_archive,
            commands::system::file_tags,
            commands::preview::read_preview,
            commands::ops::start_op,
            commands::ops::cancel_op,
            commands::ops::resolve_conflict,
            commands::sftp::ssh_hosts,
            commands::sftp::sftp_connect,
            commands::sftp::sftp_disconnect,
            commands::smb::smb_find_mount,
            commands::smb::smb_unmount,
            commands::smb::smb_saved_password,
            commands::smb::smb_store_password,
            commands::smb::smb_forget_password,
            commands::config::get_config,
            commands::config::set_config,
            commands::config::ssh_saved_password,
            commands::config::ssh_store_password,
            commands::config::ssh_forget_password,
            watch::watch_dir,
            watch::unwatch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
