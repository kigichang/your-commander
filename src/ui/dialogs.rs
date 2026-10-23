use crate::config::{Config, SmbConnection, SshConnection};
use crate::ops::archive::ArchiveFormat;
use crate::ops::queue::{ConflictChoice, ConflictRequest};
use rust_i18n::t;
use std::path::PathBuf;

/// Modal state owned by the app. At most one is open at a time.
pub enum Dialog {
    CopyMove {
        move_op: bool,
        sources: Vec<PathBuf>,
        dest: String,
        focus: bool,
    },
    NewFolder {
        name: String,
        focus: bool,
    },
    Rename {
        path: PathBuf,
        name: String,
        focus: bool,
    },
    DeleteConfirm {
        items: Vec<PathBuf>,
        /// Remote items have no trash; only permanent delete is offered.
        remote: bool,
    },
    Compress {
        sources: Vec<PathBuf>,
        name: String,
        format: ArchiveFormat,
        dest_dir: String,
        focus: bool,
    },
    Smb {
        form: SmbConnection,
        password: String,
        focus: bool,
    },
    Ssh {
        form: SshConnection,
        secret: String,
        config_hosts: Vec<SshConfigHost>,
        focus: bool,
    },
}

/// One usable Host entry from ~/.ssh/config.
#[derive(Clone)]
pub struct SshConfigHost {
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: u16,
    pub identity: String,
}

pub enum DialogOutcome {
    CopyMove {
        move_op: bool,
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    NewFolder {
        name: String,
    },
    Rename {
        path: PathBuf,
        name: String,
    },
    Trash {
        items: Vec<PathBuf>,
    },
    DeletePermanent {
        items: Vec<PathBuf>,
    },
    Compress {
        sources: Vec<PathBuf>,
        dest: PathBuf,
        format: ArchiveFormat,
    },
    SmbConnect {
        conn: SmbConnection,
        password: String,
    },
    SmbConnectSaved {
        index: usize,
    },
    SmbDelete {
        index: usize,
    },
    SshConnect {
        conn: SshConnection,
        secret: String,
    },
    SshConnectSaved {
        index: usize,
    },
    SshDelete {
        index: usize,
    },
    Cancel,
}

fn dialog_window<'a>(_ctx: &egui::Context, title: &str) -> egui::Window<'a> {
    egui::Window::new(title.to_string())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -60.0])
}

/// Render the active dialog; Some(outcome) when the user decided.
pub fn show_dialog(
    ctx: &egui::Context,
    dialog: &mut Dialog,
    config: &Config,
) -> Option<DialogOutcome> {
    let mut outcome = None;
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));

    match dialog {
        Dialog::CopyMove {
            move_op,
            sources,
            dest,
            focus,
        } => {
            let title = if *move_op {
                t!("dialog.move_title", count = sources.len())
            } else {
                t!("dialog.copy_title", count = sources.len())
            };
            dialog_window(ctx, &title).show(ctx, |ui| {
                ui.label(t!("dialog.dest_label"));
                let resp = ui.add(
                    egui::TextEdit::singleline(dest).desired_width(380.0),
                );
                if *focus {
                    resp.request_focus();
                    *focus = false;
                }
                let submitted =
                    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ok_label = if *move_op {
                        t!("cmd.r#move")
                    } else {
                        t!("cmd.copy")
                    };
                    if ui.button(ok_label).clicked() || submitted {
                        outcome = Some(DialogOutcome::CopyMove {
                            move_op: *move_op,
                            sources: sources.clone(),
                            dest: PathBuf::from(dest.clone()),
                        });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
        Dialog::NewFolder { name, focus } => {
            dialog_window(ctx, &t!("dialog.new_folder_title")).show(ctx, |ui| {
                let resp = ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                if *focus {
                    resp.request_focus();
                    *focus = false;
                }
                let submitted =
                    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if (ui.button(t!("dialog.create")).clicked() || submitted)
                        && !name.trim().is_empty()
                    {
                        outcome = Some(DialogOutcome::NewFolder { name: name.clone() });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
        Dialog::Rename { path, name, focus } => {
            dialog_window(ctx, &t!("dialog.rename_title")).show(ctx, |ui| {
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                let resp = ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                if *focus {
                    resp.request_focus();
                    *focus = false;
                }
                let submitted =
                    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if (ui.button(t!("dialog.rename")).clicked() || submitted)
                        && !name.trim().is_empty()
                    {
                        outcome = Some(DialogOutcome::Rename {
                            path: path.clone(),
                            name: name.clone(),
                        });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
        Dialog::DeleteConfirm { items, remote } => {
            dialog_window(ctx, &t!("dialog.delete_title", count = items.len())).show(ctx, |ui| {
                for item in items.iter().take(8) {
                    ui.label(item.file_name().unwrap_or_default().to_string_lossy());
                }
                if items.len() > 8 {
                    ui.label(t!("dialog.and_more", count = items.len() - 8));
                }
                if *remote {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().warn_fg_color, t!("dialog.remote_no_trash"));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if !*remote && ui.button(t!("dialog.to_trash")).clicked() {
                        outcome = Some(DialogOutcome::Trash {
                            items: items.clone(),
                        });
                    }
                    let danger = egui::Button::new(
                        egui::RichText::new(t!("dialog.delete_permanent"))
                            .color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(180, 40, 40));
                    if ui.add(danger).clicked() {
                        outcome = Some(DialogOutcome::DeletePermanent {
                            items: items.clone(),
                        });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
        Dialog::Compress {
            sources,
            name,
            format,
            dest_dir,
            focus,
        } => {
            dialog_window(ctx, &t!("dialog.compress_title", count = sources.len())).show(
                ctx,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(t!("dialog.archive_name"));
                        let resp =
                            ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
                        if *focus {
                            resp.request_focus();
                            *focus = false;
                        }
                        egui::ComboBox::from_id_salt("archive_format")
                            .selected_text(format!(".{}", format.extension()))
                            .show_ui(ui, |ui| {
                                for f in ArchiveFormat::all() {
                                    ui.selectable_value(
                                        format,
                                        f,
                                        format!(".{}", f.extension()),
                                    );
                                }
                            });
                    });
                    ui.label(t!("dialog.dest_label"));
                    ui.add(egui::TextEdit::singleline(dest_dir).desired_width(380.0));
                    if *format == ArchiveFormat::Gz
                        && (sources.len() != 1 || sources[0].is_dir())
                    {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            t!("status.gz_single_file"),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("toolbar.pack")).clicked() && !name.trim().is_empty() {
                            let file_name = format!("{}.{}", name.trim(), format.extension());
                            outcome = Some(DialogOutcome::Compress {
                                sources: sources.clone(),
                                dest: PathBuf::from(dest_dir.clone()).join(file_name),
                                format: *format,
                            });
                        }
                        if ui.button(t!("dialog.cancel")).clicked() || esc {
                            outcome = Some(DialogOutcome::Cancel);
                        }
                    });
                },
            );
        }
        Dialog::Smb {
            form,
            password,
            focus,
        } => {
            dialog_window(ctx, &t!("dialog.smb_title")).show(ctx, |ui| {
                if !config.smb_connections.is_empty() {
                    ui.label(egui::RichText::new(t!("dialog.saved_connections")).strong());
                    let mut delete_index = None;
                    for (i, conn) in config.smb_connections.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let label = format!(
                                "{} (//{}/{})",
                                conn.name, conn.server, conn.share
                            );
                            if ui.button(label).clicked() {
                                outcome = Some(DialogOutcome::SmbConnectSaved { index: i });
                            }
                            if ui
                                .button("✎")
                                .on_hover_text(t!("dialog.edit"))
                                .clicked()
                            {
                                *form = conn.clone();
                                password.clear();
                            }
                            if ui
                                .button("🗑")
                                .on_hover_text(t!("dialog.delete"))
                                .clicked()
                            {
                                delete_index = Some(i);
                            }
                        });
                    }
                    if let Some(i) = delete_index {
                        outcome = Some(DialogOutcome::SmbDelete { index: i });
                    }
                    ui.separator();
                }

                egui::Grid::new("smb_form").num_columns(2).show(ui, |ui| {
                    ui.label(t!("dialog.conn_name"));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut form.name).desired_width(240.0),
                    );
                    if *focus {
                        resp.request_focus();
                        *focus = false;
                    }
                    ui.end_row();
                    ui.label(t!("dialog.server"));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.server)
                            .hint_text("192.168.1.10 / nas.local")
                            .desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label(t!("dialog.share"));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.share).desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label(t!("dialog.username"));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.username).desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label(t!("dialog.password"));
                    ui.add(
                        egui::TextEdit::singleline(password)
                            .password(true)
                            .desired_width(240.0),
                    );
                    ui.end_row();
                });
                ui.checkbox(&mut form.remember, t!("dialog.remember"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready = !form.server.trim().is_empty() && !form.share.trim().is_empty();
                    if ui
                        .add_enabled(ready, egui::Button::new(t!("dialog.connect")))
                        .clicked()
                    {
                        outcome = Some(DialogOutcome::SmbConnect {
                            conn: form.clone(),
                            password: password.clone(),
                        });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
        Dialog::Ssh {
            form,
            secret,
            config_hosts,
            focus,
        } => {
            dialog_window(ctx, &t!("dialog.ssh_title")).show(ctx, |ui| {
                if !config.ssh_connections.is_empty() {
                    ui.label(egui::RichText::new(t!("dialog.saved_connections")).strong());
                    let mut delete_index = None;
                    for (i, conn) in config.ssh_connections.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let label = format!(
                                "{} ({}@{}:{})",
                                conn.name, conn.username, conn.host, conn.port
                            );
                            if ui.button(label).clicked() {
                                outcome = Some(DialogOutcome::SshConnectSaved { index: i });
                            }
                            if ui
                                .button("✎")
                                .on_hover_text(t!("dialog.edit"))
                                .clicked()
                            {
                                *form = conn.clone();
                                secret.clear();
                            }
                            if ui
                                .button("🗑")
                                .on_hover_text(t!("dialog.delete"))
                                .clicked()
                            {
                                delete_index = Some(i);
                            }
                        });
                    }
                    if let Some(i) = delete_index {
                        outcome = Some(DialogOutcome::SshDelete { index: i });
                    }
                    ui.separator();
                }

                if !config_hosts.is_empty() {
                    egui::ComboBox::from_label(t!("dialog.from_ssh_config"))
                        .selected_text(t!("dialog.pick_host"))
                        .show_ui(ui, |ui| {
                            for host in config_hosts.iter() {
                                if ui.selectable_label(false, &host.alias).clicked() {
                                    form.name = host.alias.clone();
                                    form.host = host.host_name.clone();
                                    form.username = host.user.clone();
                                    form.port = host.port;
                                    form.identity_file = host.identity.clone();
                                    form.use_password = false;
                                }
                            }
                        });
                    ui.separator();
                }

                egui::Grid::new("ssh_form").num_columns(2).show(ui, |ui| {
                    ui.label(t!("dialog.conn_name"));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut form.name).desired_width(240.0),
                    );
                    if *focus {
                        resp.request_focus();
                        *focus = false;
                    }
                    ui.end_row();
                    ui.label(t!("dialog.host"));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut form.host).desired_width(170.0),
                        );
                        ui.label(":");
                        ui.add(egui::DragValue::new(&mut form.port).range(1..=65535));
                    });
                    ui.end_row();
                    ui.label(t!("dialog.username"));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.username).desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label(t!("dialog.auth"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut form.use_password, false, t!("dialog.auth_key"));
                        ui.selectable_value(
                            &mut form.use_password,
                            true,
                            t!("dialog.auth_password"),
                        );
                    });
                    ui.end_row();
                    if form.use_password {
                        ui.label(t!("dialog.password"));
                    } else {
                        ui.label(t!("dialog.identity_file"));
                        ui.add(
                            egui::TextEdit::singleline(&mut form.identity_file)
                                .hint_text(t!("dialog.identity_hint"))
                                .desired_width(240.0),
                        );
                        ui.end_row();
                        ui.label(t!("dialog.passphrase"));
                    }
                    ui.add(
                        egui::TextEdit::singleline(secret)
                            .password(true)
                            .desired_width(240.0),
                    );
                    ui.end_row();
                });
                ui.checkbox(&mut form.remember, t!("dialog.remember"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready =
                        !form.host.trim().is_empty() && !form.username.trim().is_empty();
                    if ui
                        .add_enabled(ready, egui::Button::new(t!("dialog.connect")))
                        .clicked()
                    {
                        outcome = Some(DialogOutcome::SshConnect {
                            conn: form.clone(),
                            secret: secret.clone(),
                        });
                    }
                    if ui.button(t!("dialog.cancel")).clicked() || esc {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        }
    }
    outcome
}

/// Render the file-collision prompt; Some(choice) when the user picked.
pub fn show_conflict(ctx: &egui::Context, req: &ConflictRequest) -> Option<ConflictChoice> {
    let mut choice = None;
    dialog_window(ctx, &t!("dialog.conflict_title")).show(ctx, |ui| {
        ui.label(t!(
            "dialog.conflict_msg",
            name = req
                .dest
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("→ {}", req.dest.to_string_lossy()))
                .small()
                .weak(),
        );
        ui.label(
            egui::RichText::new(format!("← {}", req.src.to_string_lossy()))
                .small()
                .weak(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(t!("dialog.overwrite")).clicked() {
                choice = Some(ConflictChoice::Overwrite);
            }
            if ui.button(t!("dialog.skip")).clicked() {
                choice = Some(ConflictChoice::Skip);
            }
            if ui.button(t!("dialog.keep_both")).clicked() {
                choice = Some(ConflictChoice::KeepBoth);
            }
        });
        ui.horizontal(|ui| {
            if ui.button(t!("dialog.overwrite_all")).clicked() {
                choice = Some(ConflictChoice::OverwriteAll);
            }
            if ui.button(t!("dialog.skip_all")).clicked() {
                choice = Some(ConflictChoice::SkipAll);
            }
            if ui.button(t!("dialog.cancel_op")).clicked() {
                choice = Some(ConflictChoice::CancelOp);
            }
        });
    });
    choice
}
