mod commands;
mod config;
mod fnkeys;
mod fs;
mod icons;
mod keychain;
mod ops;
mod state;
mod watch;

use fnkeys::FnKeysState;
use state::AppState;
use tauri::{LogicalSize, Manager, WindowEvent};

fn apply_fnkeys(app: &tauri::AppHandle, focused: bool) {
    let enabled = app
        .state::<AppState>()
        .config
        .lock()
        .unwrap()
        .fkeys_without_fn;
    if let Some(fk) = &app.state::<FnKeysState>().0 {
        if focused && enabled {
            fk.acquire();
        } else {
            fk.release();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = AppState::default();
    *app_state.config.lock().unwrap() = config::Config::load();

    let app = tauri::Builder::default()
        .manage(app_state)
        .manage(FnKeysState(fnkeys::FnKeys::open()))
        .setup(|app| {
            // Restore the last window size (logical points).
            let size = app
                .state::<AppState>()
                .config
                .lock()
                .unwrap()
                .window_size;
            if let (Some((w, h)), Some(window)) = (size, app.get_webview_window("main")) {
                let _ = window.set_size(LogicalSize::new(w.max(800.0), h.max(500.0)));
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(focused) => {
                apply_fnkeys(window.app_handle(), *focused);
            }
            WindowEvent::CloseRequested { .. } => {
                let app = window.app_handle();
                if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
                    let logical = size.to_logical::<f32>(scale);
                    let config_state = app.state::<AppState>();
                    let mut config = config_state.config.lock().unwrap();
                    config.window_size = Some((logical.width, logical.height));
                    config.save();
                }
                apply_fnkeys(app, false);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::mkdir,
            commands::fs::rename,
            commands::fs::batch_rename,
            commands::fs::exists,
            commands::system::volumes,
            commands::system::home_dir,
            commands::system::open_path,
            commands::system::reveal_in_finder,
            commands::system::is_archive,
            commands::system::can_browse_archive,
            commands::system::file_tags,
            commands::preview::read_preview,
            commands::plantuml::render_plantuml,
            commands::sync::sync_compare,
            commands::find::start_find,
            commands::find::cancel_find,
            commands::ops::start_op,
            commands::ops::pause_op,
            commands::ops::cancel_op,
            commands::ops::resolve_conflict,
            commands::sftp::ssh_hosts,
            commands::sftp::sftp_connect,
            commands::sftp::sftp_test,
            commands::sftp::sftp_disconnect,
            commands::smb::smb_test,
            commands::smb::smb_shares,
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
            fnkeys::fkeys_set,
            fnkeys::fkeys_available,
            icons::file_icon,
            watch::watch_dir,
            watch::unwatch,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        // Whatever path the process leaves by, give the fn mode back.
        if let tauri::RunEvent::Exit = event {
            if let Some(fk) = &app.state::<FnKeysState>().0 {
                fk.release();
            }
        }
    });
}
