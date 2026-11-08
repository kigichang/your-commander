mod commands;
mod config;
mod dock;
mod fnkeys;
mod fonts;
mod fs;
mod icons;
mod keychain;
mod lru;
// Linux only: WebKitGTK sends no Range requests to custom schemes, so media
// streams over localhost HTTP there; macOS streams through stream://.
// Compiled under test everywhere so its tests run with the macOS suite.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod media_server;
mod ops;
mod platform;
mod state;
mod stream_protocol;
mod watch;

use fnkeys::FnKeysState;
use state::AppState;
use tauri::{LogicalSize, Manager, WindowEvent};

/// Add "New Window" (⌘N) to the top of the default menu's File submenu.
fn add_new_window_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{MenuItem, MenuItemKind, PredefinedMenuItem};
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    let lang = app
        .state::<AppState>()
        .config
        .lock()
        .unwrap()
        .language
        .clone();
    let title = if lang.as_deref().is_some_and(|l| l.starts_with("zh")) {
        "開新視窗"
    } else {
        "New Window"
    };
    for item in menu.items()? {
        if let MenuItemKind::Submenu(sub) = item
            && sub.text()? == "File"
        {
            let new_win = MenuItem::with_id(app, "new-window", title, true, Some("CmdOrCtrl+N"))?;
            sub.insert(&new_win, 0)?;
            sub.insert(&PredefinedMenuItem::separator(app)?, 1)?;
            break;
        }
    }
    Ok(())
}

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
        .register_asynchronous_uri_scheme_protocol("stream", |_ctx, request, responder| {
            stream_protocol::handle(request, responder)
        })
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
                use commands::system::{MIN_HEIGHT, MIN_WIDTH};
                let (w, h) = (f64::from(w).max(MIN_WIDTH), f64::from(h).max(MIN_HEIGHT));
                let _ = window.set_size(LogicalSize::new(w, h));
            }
            dock::install(app.handle());
            add_new_window_menu(app.handle())?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "new-window" {
                let _ = commands::system::open_new_window(app);
            }
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
                // Drop this window's directory watchers — off the main
                // thread, since FSEvents teardown can block on a dead volume.
                let prefix = format!("{}:", window.label());
                let dropped: Vec<_> = {
                    let state = app.state::<AppState>();
                    let mut watchers = state.watchers.lock().unwrap();
                    let keys: Vec<String> = watchers
                        .keys()
                        .filter(|k| k.starts_with(&prefix))
                        .cloned()
                        .collect();
                    keys.iter().filter_map(|k| watchers.remove(k)).collect()
                };
                if !dropped.is_empty() {
                    std::thread::spawn(move || drop(dropped));
                }
                // Closing a background window must not hand the F-keys back
                // while another of ours is still in front.
                let other_focused = app.webview_windows().values().any(|w| {
                    w.label() != window.label() && w.is_focused().unwrap_or(false)
                });
                apply_fnkeys(app, other_focused);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::mkdir,
            commands::fs::rename,
            commands::fs::batch_rename,
            commands::fs::stat_paths,
            commands::system::new_window,
            commands::system::quit_app,
            commands::system::volumes,
            commands::system::home_dir,
            commands::system::trash_dir,
            commands::system::desktop_dir,
            commands::system::open_path,
            commands::system::open_in,
            commands::system::open_with,
            commands::system::apps_for_path,
            commands::system::video_tools,
            commands::system::repair_video,
            commands::system::finish_repair,
            commands::system::get_info,
            commands::system::reveal_in_finder,
            commands::system::file_tags,
            commands::preview::read_preview,
            commands::preview::media_info,
            commands::preview::video_poster,
            #[cfg(target_os = "linux")]
            media_server::media_url,
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
            commands::smb::smb_unmount,
            commands::smb::smb_saved_password,
            commands::smb::smb_store_password,
            commands::smb::smb_forget_password,
            commands::config::get_config,
            commands::config::update_config,
            commands::config::ssh_saved_password,
            commands::config::ssh_store_password,
            commands::config::ssh_forget_password,
            fnkeys::fkeys_set,
            fnkeys::fkeys_available,
            icons::file_icon,
            fonts::list_font_families,
            commands::drag::start_drag,
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

#[cfg(test)]
mod tests {
    use tauri::utils::config::{parse::read_from, Csp};
    use tauri::utils::platform::Target;

    /// The CSP as Tauri builds it for `target` (base config merged with
    /// the platform file, the same way the build does).
    fn media_src(target: Target) -> Vec<String> {
        let (config, _) = read_from(target, std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let csp: Csp = serde_json::from_value(config["app"]["security"]["csp"].clone()).unwrap();
        let policy = csp.to_string();
        let media = policy
            .split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix("media-src "))
            .unwrap_or_else(|| panic!("no media-src in {policy}"));
        media.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn only_linux_lets_media_load_from_localhost_http() {
        let mac = media_src(Target::MacOS);
        let linux = media_src(Target::Linux);
        assert!(!mac.iter().any(|s| s.contains("127.0.0.1")), "{mac:?}");
        assert!(mac.contains(&"stream:".to_string()), "{mac:?}");
        // Linux keeps everything macOS allows and adds the media server.
        assert!(mac.iter().all(|s| linux.contains(s)), "{linux:?}");
        assert!(linux.contains(&"http://127.0.0.1:*".to_string()), "{linux:?}");
    }
}
