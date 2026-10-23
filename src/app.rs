use crate::config::{Config, Favorite, SmbConnection, SshConnection};
use crate::fs::sftp::{self, SftpFs, SshAuth};
use crate::fs::smb;
use crate::keychain;
use crate::ops::remote;
use crate::ui::dialogs::SshConfigHost;
use crate::ops::archive::{self, ArchiveFormat};
use crate::ops::queue::OpsManager;
use crate::ops::{delete, transfer};
use crate::ui::command_bar::{self, Command};
use crate::ui::dialogs::{self, Dialog, DialogOutcome};
use crate::ui::panel::{self, PanelAction, PanelState, ViewMode};
use crate::ui::toolbar::{self, ToolbarAction};
use notify::Watcher;
use rust_i18n::t;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub struct YcApp {
    panels: [PanelState; 2],
    active: usize,
    config: Config,
    status: Option<(String, Instant)>,
    ops: OpsManager,
    dialog: Option<Dialog>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: [Option<PathBuf>; 2],
    fs_rx: Receiver<notify::Result<notify::Event>>,
    show_shortcuts: bool,
}

fn view_from_str(s: Option<&String>) -> ViewMode {
    match s.map(String::as_str) {
        Some("icons") => ViewMode::Icons,
        _ => ViewMode::List,
    }
}

fn view_to_str(v: ViewMode) -> String {
    match v {
        ViewMode::Icons => "icons".to_string(),
        ViewMode::List => "list".to_string(),
    }
}

impl YcApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        let left = PanelState::new_local(
            0,
            config.left_path.clone(),
            view_from_str(config.left_view.as_ref()),
            config.show_hidden,
        );
        let right = PanelState::new_local(
            1,
            config.right_path.clone(),
            view_from_str(config.right_view.as_ref()),
            config.show_hidden,
        );

        let (fs_tx, fs_rx) = std::sync::mpsc::channel();
        let repaint_ctx = cc.egui_ctx.clone();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = fs_tx.send(event);
            repaint_ctx.request_repaint();
        })
        .ok();

        Self {
            panels: [left, right],
            active: 0,
            config,
            status: None,
            ops: OpsManager::new(cc.egui_ctx.clone()),
            dialog: None,
            watcher,
            watched: [None, None],
            fs_rx,
            show_shortcuts: false,
        }
    }

    /// QuickLook preview via the system `qlmanage` tool (local files only).
    fn quick_look(&mut self) {
        if !self.panels[self.active].provider.is_local() {
            self.set_status(t!("status.remote_open_unsupported"));
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let _ = std::process::Command::new("/usr/bin/qlmanage")
            .arg("-p")
            .args(&paths)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn save_config(&mut self) {
        self.config.left_path = Some(self.panels[0].cwd.clone());
        self.config.right_path = Some(self.panels[1].cwd.clone());
        self.config.left_view = Some(view_to_str(self.panels[0].view));
        self.config.right_view = Some(view_to_str(self.panels[1].view));
        self.config.save();
    }

    /// Keep the fs watcher pointed at both panels' local directories.
    fn sync_watchers(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for side in 0..2 {
            let want = if self.panels[side].provider.is_local() {
                Some(self.panels[side].cwd.clone())
            } else {
                None
            };
            if self.watched[side] != want {
                if let Some(old) = self.watched[side].take() {
                    // The other side may still watch the same path.
                    let other = self.watched[1 - side].as_ref();
                    if other != Some(&old) {
                        let _ = watcher.unwatch(&old);
                    }
                }
                if let Some(new) = want {
                    if watcher
                        .watch(&new, notify::RecursiveMode::NonRecursive)
                        .is_ok()
                    {
                        self.watched[side] = Some(new);
                    }
                } else {
                    self.watched[side] = None;
                }
            }
        }
    }

    fn drain_fs_events(&mut self) {
        let mut refresh = [false, false];
        while let Ok(Ok(event)) = self.fs_rx.try_recv() {
            for path in &event.paths {
                for (side, watched) in self.watched.iter().enumerate() {
                    if let Some(watched) = watched
                        && path.starts_with(watched)
                    {
                        refresh[side] = true;
                    }
                }
            }
        }
        for (side, do_it) in refresh.into_iter().enumerate() {
            if do_it {
                self.panels[side].refresh();
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }

        #[allow(clippy::type_complexity)]
        let (
            tab,
            enter,
            backspace,
            up,
            down,
            shift,
            cmd,
            esc,
            cmd_a,
            cmd_up,
            toggle_hidden,
            cmd_l,
            f5,
            f6,
            f7,
            f8,
            f2,
            cmd_del,
            cmd_shift_del,
            space,
            cmd_slash,
            texts,
        ) = ctx.input_mut(|i| {
            let texts: Vec<String> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::ArrowUp) && !i.modifiers.command,
                i.key_pressed(egui::Key::ArrowDown),
                i.modifiers.shift,
                i.modifiers.command,
                i.key_pressed(egui::Key::Escape),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::A),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowUp),
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Period,
                ),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::L),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F5),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F6),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F7),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F8),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F2),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace),
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Backspace,
                ),
                i.key_pressed(egui::Key::Space),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Slash),
                texts,
            )
        });

        if tab {
            self.active = 1 - self.active;
            return;
        }
        if cmd_l {
            let panel = &mut self.panels[self.active];
            panel.path_edit = Some(panel.cwd.to_string_lossy().to_string());
            panel.focus_path_edit = true;
            return;
        }
        if toggle_hidden {
            self.config.show_hidden = !self.config.show_hidden;
            for p in &mut self.panels {
                p.show_hidden = self.config.show_hidden;
                p.refresh();
            }
            self.save_config();
            return;
        }
        if f5 {
            self.run_command(Command::Copy);
            return;
        }
        if f6 {
            self.run_command(Command::Move);
            return;
        }
        if f7 {
            self.run_command(Command::NewFolder);
            return;
        }
        if f8 || cmd_del || cmd_shift_del {
            self.run_command(Command::Delete);
            return;
        }
        if f2 {
            self.open_rename_dialog();
            return;
        }
        if cmd_slash {
            self.show_shortcuts = !self.show_shortcuts;
            return;
        }
        if space && self.panels[self.active].filter.is_empty() {
            self.quick_look();
            return;
        }

        let panel = &mut self.panels[self.active];

        if cmd_a {
            panel.select_all();
        }
        if up {
            panel.move_cursor(-1, shift);
        }
        if down {
            panel.move_cursor(1, shift);
        }
        if enter && !cmd {
            if let Some(msg) = panel.open_cursor() {
                self.set_status(msg);
            }
            return;
        }
        if cmd_up {
            panel.go_parent();
            return;
        }
        if backspace {
            if panel.filter.is_empty() {
                panel.go_parent();
            } else {
                panel.filter.pop();
                panel.cursor = None;
            }
            return;
        }
        if esc {
            panel.filter.clear();
            return;
        }
        if !cmd {
            for t in texts {
                for ch in t.chars() {
                    if !ch.is_control() {
                        panel.filter.push(ch);
                        panel.cursor = None;
                    }
                }
            }
        }
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.panels[self.active]
            .selected_entries()
            .iter()
            .map(|e| e.path.clone())
            .collect()
    }

    fn open_compress_dialog(&mut self) {
        if !self.panels[self.active].provider.is_local()
            || !self.panels[1 - self.active].provider.is_local()
        {
            self.set_status(t!("status.archive_local_only"));
            return;
        }
        let sources = self.selected_paths();
        if sources.is_empty() {
            self.set_status(t!("status.nothing_selected"));
            return;
        }
        let name = if sources.len() == 1 {
            archive::archive_stem(
                &sources[0]
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
        } else {
            self.panels[self.active]
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Archive".to_string())
        };
        let dest_dir = self.panels[1 - self.active]
            .cwd
            .to_string_lossy()
            .to_string();
        self.dialog = Some(Dialog::Compress {
            sources,
            name,
            format: ArchiveFormat::Zip,
            dest_dir,
            focus: true,
        });
    }

    fn extract_selection(&mut self) {
        if !self.panels[self.active].provider.is_local()
            || !self.panels[1 - self.active].provider.is_local()
        {
            self.set_status(t!("status.archive_local_only"));
            return;
        }
        let archives: Vec<PathBuf> = self
            .selected_paths()
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .map(|n| archive::is_archive(&n.to_string_lossy()))
                    .unwrap_or(false)
            })
            .collect();
        if archives.is_empty() {
            self.set_status(t!("status.no_archives_selected"));
            return;
        }
        if !self.panels[1 - self.active].provider.is_local() {
            self.set_status(t!("status.remote_target_unsupported"));
            return;
        }
        let dest = self.panels[1 - self.active].cwd.clone();
        self.ops
            .spawn(t!("toolbar.unpack").to_string(), move |ctx| {
                archive::extract_op(ctx, archives, dest)
            });
    }

    /// Mount an SMB share in the background, then jump the panel into it.
    fn spawn_smb_mount(&mut self, conn: SmbConnection, password: String) {
        let side = self.active;
        let label = format!("SMB //{}/{}", conn.server, conn.share);
        self.ops.spawn(label, move |ctx| {
            ctx.report(&t!("status.mounting"));
            let path = smb::mount(&conn.server, &conn.share, &conn.username, &password)?;
            ctx.open_path(side, path.clone());
            Ok(t!("status.mounted", path = path.to_string_lossy()).to_string())
        });
    }

    fn smb_connect(&mut self, conn: SmbConnection, password: String) {
        if conn.remember {
            let account = keychain::smb_account(&conn.username, &conn.server, &conn.share);
            if !password.is_empty()
                && let Err(e) = keychain::store(&account, &password)
            {
                self.set_status(format!("Keychain: {e}"));
            }
            let existing = self
                .config
                .smb_connections
                .iter()
                .position(|c| c.server == conn.server && c.share == conn.share);
            match existing {
                Some(i) => self.config.smb_connections[i] = conn.clone(),
                None => self.config.smb_connections.push(conn.clone()),
            }
            self.save_config();
        }
        self.spawn_smb_mount(conn, password);
    }

    fn smb_connect_saved(&mut self, index: usize) {
        let Some(conn) = self.config.smb_connections.get(index).cloned() else {
            return;
        };
        let account = keychain::smb_account(&conn.username, &conn.server, &conn.share);
        match keychain::load(&account) {
            Some(password) => self.spawn_smb_mount(conn, password),
            None => {
                // No stored password: reopen the form pre-filled to ask for one.
                self.dialog = Some(Dialog::Smb {
                    form: conn,
                    password: String::new(),
                    focus: false,
                });
            }
        }
    }

    fn ssh_keychain_account(conn: &SshConnection) -> String {
        format!("ssh://{}@{}:{}", conn.username, conn.host, conn.port)
    }

    fn spawn_ssh_connect(&mut self, conn: SshConnection, secret: String) {
        let side = self.active;
        let label = format!("SSH {}@{}", conn.username, conn.host);
        self.ops.spawn(label, move |ctx| {
            ctx.report(&t!("status.connecting"));
            let auth = if conn.use_password {
                SshAuth::Password(secret.clone())
            } else {
                SshAuth::Key {
                    path: if conn.identity_file.trim().is_empty() {
                        None
                    } else {
                        Some(expand_tilde(conn.identity_file.trim()))
                    },
                    passphrase: if secret.is_empty() {
                        None
                    } else {
                        Some(secret.clone())
                    },
                }
            };
            let connection = sftp::connect(&conn.host, conn.port, &conn.username, &auth)?;
            let label = connection.label.clone();
            ctx.sftp_ready(side, connection);
            Ok(t!("status.connected", host = label).to_string())
        });
    }

    fn ssh_connect(&mut self, conn: SshConnection, secret: String) {
        if conn.remember {
            let account = Self::ssh_keychain_account(&conn);
            if !secret.is_empty()
                && let Err(e) = keychain::store(&account, &secret)
            {
                self.set_status(format!("Keychain: {e}"));
            }
            let existing = self
                .config
                .ssh_connections
                .iter()
                .position(|c| c.host == conn.host && c.username == conn.username && c.port == conn.port);
            match existing {
                Some(i) => self.config.ssh_connections[i] = conn.clone(),
                None => self.config.ssh_connections.push(conn.clone()),
            }
            self.save_config();
        }
        self.spawn_ssh_connect(conn, secret);
    }

    fn ssh_connect_saved(&mut self, index: usize) {
        let Some(conn) = self.config.ssh_connections.get(index).cloned() else {
            return;
        };
        let secret = keychain::load(&Self::ssh_keychain_account(&conn));
        match secret {
            Some(secret) => self.spawn_ssh_connect(conn, secret),
            None if !conn.use_password => {
                // Key auth without a stored passphrase: try an unencrypted key.
                self.spawn_ssh_connect(conn, String::new());
            }
            None => {
                self.dialog = Some(Dialog::Ssh {
                    form: conn,
                    secret: String::new(),
                    config_hosts: parse_ssh_config_hosts(),
                    focus: false,
                });
            }
        }
    }

    fn open_rename_dialog(&mut self) {
        let sel = self.selected_paths();
        if sel.len() != 1 {
            self.set_status(t!("status.select_one"));
            return;
        }
        let path = sel.into_iter().next().unwrap();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.dialog = Some(Dialog::Rename {
            path,
            name,
            focus: true,
        });
    }

    fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::Copy | Command::Move => {
                let move_op = cmd == Command::Move;
                let sources = self.selected_paths();
                if sources.is_empty() {
                    self.set_status(t!("status.nothing_selected"));
                    return;
                }
                let src_conn = self.panels[self.active].provider.sftp_conn();
                let dst_conn = self.panels[1 - self.active].provider.sftp_conn();
                match (src_conn, dst_conn) {
                    (None, None) => {
                        let dest = self.panels[1 - self.active]
                            .cwd
                            .to_string_lossy()
                            .to_string();
                        self.dialog = Some(Dialog::CopyMove {
                            move_op,
                            sources,
                            dest,
                            focus: false,
                        });
                    }
                    (None, Some(conn)) => {
                        let remote_dir = self.panels[1 - self.active].cwd.clone();
                        let label = t!("status.upload_label").to_string();
                        self.ops.spawn(label, move |ctx| {
                            remote::upload_op(ctx, conn, sources, remote_dir, move_op)
                        });
                    }
                    (Some(conn), None) => {
                        let items: Vec<(std::path::PathBuf, bool)> = self.panels[self.active]
                            .selected_entries()
                            .iter()
                            .map(|e| (e.path.clone(), e.is_dir))
                            .collect();
                        let local_dir = self.panels[1 - self.active].cwd.clone();
                        let label = t!("status.download_label").to_string();
                        self.ops.spawn(label, move |ctx| {
                            remote::download_op(ctx, conn, items, local_dir, move_op)
                        });
                    }
                    (Some(_), Some(_)) => {
                        self.set_status(t!("status.remote_to_remote_unsupported"));
                    }
                }
            }
            Command::NewFolder => {
                self.dialog = Some(Dialog::NewFolder {
                    name: String::new(),
                    focus: true,
                });
            }
            Command::Delete => {
                let items = self.selected_paths();
                if items.is_empty() {
                    self.set_status(t!("status.nothing_selected"));
                    return;
                }
                let remote = !self.panels[self.active].provider.is_local();
                self.dialog = Some(Dialog::DeleteConfirm { items, remote });
            }
        }
    }

    fn apply_dialog_outcome(&mut self, outcome: DialogOutcome) {
        match outcome {
            DialogOutcome::Cancel => {}
            DialogOutcome::CopyMove {
                move_op,
                sources,
                dest,
            } => {
                if !dest.is_dir() {
                    self.set_status(
                        t!("status.not_a_dir", path = dest.to_string_lossy()).to_string(),
                    );
                    return;
                }
                let label = if move_op {
                    t!("cmd.r#move").to_string()
                } else {
                    t!("cmd.copy").to_string()
                };
                self.ops.spawn(label, move |ctx| {
                    transfer::copy_op(ctx, sources, dest, move_op)
                });
            }
            DialogOutcome::NewFolder { name } => {
                let dir = self.panels[self.active].cwd.clone();
                let result = match self.panels[self.active].provider.sftp_conn() {
                    Some(conn) => conn.mkdir(&dir.join(name.trim())),
                    None => transfer::new_folder(&dir, name.trim()),
                };
                match result {
                    Ok(()) => {
                        self.panels[self.active].refresh();
                        self.set_status(t!("status.folder_created"));
                    }
                    Err(e) => self.set_status(e),
                }
            }
            DialogOutcome::Rename { path, name } => {
                let result = match self.panels[self.active].provider.sftp_conn() {
                    Some(conn) => {
                        let parent = path.parent().unwrap_or(&path).to_path_buf();
                        conn.rename(&path, &parent.join(name.trim()))
                    }
                    None => transfer::rename(&path, name.trim()),
                };
                match result {
                    Ok(()) => {
                        self.panels[self.active].refresh();
                        self.set_status(t!("status.renamed"));
                    }
                    Err(e) => self.set_status(e),
                }
            }
            DialogOutcome::Trash { items } => {
                self.ops
                    .spawn(t!("dialog.to_trash").to_string(), move |ctx| {
                        delete::trash_op(ctx, items)
                    });
            }
            DialogOutcome::DeletePermanent { items } => {
                match self.panels[self.active].provider.sftp_conn() {
                    Some(conn) => {
                        self.ops
                            .spawn(t!("dialog.delete_permanent").to_string(), move |ctx| {
                                remote::delete_remote_op(ctx, conn, items)
                            });
                    }
                    None => {
                        self.ops
                            .spawn(t!("dialog.delete_permanent").to_string(), move |ctx| {
                                delete::delete_op(ctx, items)
                            });
                    }
                }
            }
            DialogOutcome::SmbConnect { conn, password } => {
                self.smb_connect(conn, password);
            }
            DialogOutcome::SshConnect { conn, secret } => {
                self.ssh_connect(conn, secret);
            }
            DialogOutcome::SshConnectSaved { index } => {
                self.ssh_connect_saved(index);
            }
            DialogOutcome::SshDelete { index } => {
                if index < self.config.ssh_connections.len() {
                    let conn = self.config.ssh_connections.remove(index);
                    keychain::remove(&Self::ssh_keychain_account(&conn));
                    self.save_config();
                    self.dialog = Some(Dialog::Ssh {
                        form: SshConnection::default(),
                        secret: String::new(),
                        config_hosts: parse_ssh_config_hosts(),
                        focus: false,
                    });
                }
            }
            DialogOutcome::SmbConnectSaved { index } => {
                self.smb_connect_saved(index);
            }
            DialogOutcome::SmbDelete { index } => {
                if index < self.config.smb_connections.len() {
                    let conn = self.config.smb_connections.remove(index);
                    keychain::remove(&keychain::smb_account(
                        &conn.username,
                        &conn.server,
                        &conn.share,
                    ));
                    self.save_config();
                    // Keep the dialog open so the list updates visibly.
                    self.dialog = Some(Dialog::Smb {
                        form: SmbConnection::default(),
                        password: String::new(),
                        focus: false,
                    });
                }
            }
            DialogOutcome::Compress {
                sources,
                dest,
                format,
            } => {
                let parent_ok = dest.parent().map(|p| p.is_dir()).unwrap_or(false);
                if !parent_ok {
                    self.set_status(
                        t!("status.not_a_dir", path = dest.to_string_lossy()).to_string(),
                    );
                    return;
                }
                self.ops
                    .spawn(t!("toolbar.pack").to_string(), move |ctx| {
                        archive::compress_op(ctx, sources, dest, format)
                    });
            }
        }
    }

    fn apply_panel_actions(&mut self, side: usize, actions: Vec<PanelAction>) {
        for action in actions {
            match action {
                PanelAction::Activate => self.active = side,
                PanelAction::Status(msg) => self.set_status(msg),
                PanelAction::RemoveFavorite(index) => {
                    if index < self.config.favorites.len() {
                        self.config.favorites.remove(index);
                        self.save_config();
                    }
                }
                PanelAction::Eject(path) => {
                    // Leave the volume before unmounting if a panel is inside it.
                    for p in &mut self.panels {
                        if p.cwd.starts_with(&path) {
                            p.go_home();
                        }
                    }
                    self.ops
                        .spawn(t!("panel.eject").to_string(), move |_ctx| {
                            smb::unmount(&path)?;
                            Ok(t!("status.ejected").to_string())
                        });
                }
                PanelAction::AddFavorite(path) => {
                    if !self.config.favorites.iter().any(|f| f.path == path) {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.to_string_lossy().to_string());
                        self.config.favorites.push(Favorite { name, path });
                        self.save_config();
                        self.set_status(t!("status.favorite_added"));
                    }
                }
            }
        }
    }
}

impl eframe::App for YcApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Background work: worker events, fs notifications.
        let drained = self.ops.drain();
        for (label, result) in drained.finished {
            match result {
                Ok(msg) => self.set_status(msg),
                Err(err) => self.set_status(format!("{label}: {err}")),
            }
            for p in &mut self.panels {
                p.refresh();
            }
        }
        for (side, path) in drained.open_paths {
            self.panels[side].navigate(path);
            self.active = side;
        }
        for (side, conn) in drained.sftp_ready {
            self.panels[side].set_provider(Box::new(SftpFs { conn }));
            self.active = side;
        }
        self.drain_fs_events();
        self.sync_watchers();

        self.handle_keys(&ctx);

        if let Some((_, at)) = &self.status {
            if at.elapsed() > Duration::from_secs(5) {
                self.status = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        egui::Panel::top("main_toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            let actions = toolbar::show_toolbar(ui, self.panels[self.active].view);
            ui.add_space(4.0);
            for action in actions {
                match action {
                    ToolbarAction::SetView(v) => {
                        self.panels[self.active].view = v;
                        self.save_config();
                    }
                    ToolbarAction::ToggleShortcuts => {
                        self.show_shortcuts = !self.show_shortcuts;
                    }
                    ToolbarAction::Pack => self.open_compress_dialog(),
                    ToolbarAction::Unpack => self.extract_selection(),
                    ToolbarAction::ConnectSmb => {
                        self.dialog = Some(Dialog::Smb {
                            form: SmbConnection {
                                remember: true,
                                ..Default::default()
                            },
                            password: String::new(),
                            focus: true,
                        });
                    }
                    ToolbarAction::ConnectSsh => {
                        self.dialog = Some(Dialog::Ssh {
                            form: SshConnection::default(),
                            secret: String::new(),
                            config_hosts: parse_ssh_config_hosts(),
                            focus: true,
                        });
                    }
                }
            }
        });

        egui::Panel::bottom("command_bar").show(ui, |ui| {
            ui.add_space(2.0);
            // Running operations with progress and cancel.
            let mut cancel_ids = Vec::new();
            for op in &self.ops.running {
                ui.horizontal(|ui| {
                    ui.label(&op.label);
                    ui.add(
                        egui::ProgressBar::new(op.fraction)
                            .desired_width(ui.available_width() - 160.0)
                            .show_percentage(),
                    );
                    if ui.button("✖").on_hover_text(t!("dialog.cancel")).clicked() {
                        cancel_ids.push(op.id);
                    }
                });
                ui.label(egui::RichText::new(&op.detail).small().weak());
            }
            for id in cancel_ids {
                if let Some(op) = self.ops.running.iter().find(|o| o.id == id) {
                    op.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }

            let left = self.panels[0].selection_summary();
            let right = self.panels[1].selection_summary();
            let transient = self.status.as_ref().map(|(m, _)| m.clone());
            if let Some(cmd) =
                command_bar::show_command_bar(ui, &left, &right, transient.as_deref())
            {
                self.run_command(cmd);
            }
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let mut pending: Vec<(usize, Vec<PanelAction>)> = Vec::new();
            ui.columns(2, |cols| {
                for (side, col) in cols.iter_mut().enumerate() {
                    let is_active = self.active == side;
                    let actions =
                        panel::show_panel(col, &mut self.panels[side], &self.config, is_active);
                    if !actions.is_empty() {
                        pending.push((side, actions));
                    }
                }
            });
            for (side, actions) in pending {
                self.apply_panel_actions(side, actions);
            }
        });


        // Remember the window size for the next launch.
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.config.window_size = Some((rect.width(), rect.height()));
        }

        if self.show_shortcuts {
            let mut open = true;
            egui::Window::new(t!("shortcuts.title"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    let rows: [(String, String); 14] = [
                        ("Tab".into(), t!("shortcuts.switch_panel").to_string()),
                        ("Enter".into(), t!("shortcuts.open").to_string()),
                        ("Cmd+↑ / Backspace".into(), t!("shortcuts.parent").to_string()),
                        ("Space".into(), t!("shortcuts.quicklook").to_string()),
                        ("F5".into(), t!("cmd.copy").to_string()),
                        ("F6".into(), t!("cmd.r#move").to_string()),
                        ("F7".into(), t!("cmd.new_folder").to_string()),
                        ("F8 / Cmd+Delete".into(), t!("cmd.delete").to_string()),
                        ("F2".into(), t!("dialog.rename").to_string()),
                        ("Cmd+A".into(), t!("shortcuts.select_all").to_string()),
                        ("Cmd+L".into(), t!("panel.edit_path").to_string()),
                        ("Cmd+Shift+.".into(), t!("shortcuts.toggle_hidden").to_string()),
                        (
                            t!("shortcuts.type_keys").to_string(),
                            t!("shortcuts.type_filter").to_string(),
                        ),
                        ("Esc".into(), t!("shortcuts.clear_filter").to_string()),
                    ];
                    egui::Grid::new("shortcuts_grid")
                        .num_columns(2)
                        .spacing([24.0, 4.0])
                        .show(ui, |ui| {
                            for (key, desc) in rows {
                                ui.label(egui::RichText::new(key).monospace().strong());
                                ui.label(desc);
                                ui.end_row();
                            }
                        });
                });
            if !open {
                self.show_shortcuts = false;
            }
        }

        // Modal dialogs (at most one at a time; conflicts take precedence).
        if let Some(req) = self.ops.pending_conflict.take() {
            match dialogs::show_conflict(&ctx, &req) {
                Some(choice) => {
                    let _ = req.respond.send(choice);
                }
                None => self.ops.pending_conflict = Some(req),
            }
        } else if let Some(mut dialog) = self.dialog.take() {
            match dialogs::show_dialog(&ctx, &mut dialog, &self.config) {
                Some(outcome) => self.apply_dialog_outcome(outcome),
                None => self.dialog = Some(dialog),
            }
        }
    }

    fn on_exit(&mut self) {
        self.save_config();
    }
}

impl Drop for YcApp {
    fn drop(&mut self) {
        self.save_config();
    }
}

fn expand_tilde(input: &str) -> std::path::PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        crate::fs::local::home_dir().join(rest)
    } else {
        std::path::PathBuf::from(input)
    }
}

/// Usable (non-wildcard) Host entries from ~/.ssh/config.
fn parse_ssh_config_hosts() -> Vec<SshConfigHost> {
    use ssh2_config::{ParseRule, SshConfig};
    let Ok(config) = SshConfig::parse_default_file(ParseRule::ALLOW_UNKNOWN_FIELDS) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for host in config.get_hosts() {
        let Some(clause) = host.pattern.first() else {
            continue;
        };
        let alias = clause.pattern.clone();
        if alias.contains('*') || alias.contains('?') {
            continue;
        }
        let params = config.query(&alias);
        out.push(SshConfigHost {
            host_name: params.host_name.clone().unwrap_or_else(|| alias.clone()),
            user: params.user.clone().unwrap_or_default(),
            port: params.port.unwrap_or(22),
            identity: params
                .identity_file
                .as_ref()
                .and_then(|v| v.first())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            alias,
        });
    }
    out.sort_by_key(|h| h.alias.to_lowercase());
    out
}
