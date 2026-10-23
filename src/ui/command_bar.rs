use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Copy,
    Move,
    NewFolder,
    Delete,
}

/// Bottom bar: status line plus the Total-Commander-style function keys.
pub fn show_command_bar(
    ui: &mut egui::Ui,
    status_left: &str,
    status_right: &str,
    transient: Option<&str>,
) -> Option<Command> {
    let mut command = None;

    ui.horizontal(|ui| {
        ui.label(status_left);
        ui.separator();
        ui.label(status_right);
        if let Some(msg) = transient {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(ui.visuals().warn_fg_color, msg);
            });
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let buttons: [(Command, String); 4] = [
            (Command::Copy, format!("F5 {}", t!("cmd.copy"))),
            (Command::Move, format!("F6 {}", t!("cmd.r#move"))),
            (Command::NewFolder, format!("F7 {}", t!("cmd.new_folder"))),
            (Command::Delete, format!("F8 {}", t!("cmd.delete"))),
        ];
        let width = (ui.available_width() - 3.0 * 4.0) / 4.0;
        for (cmd, label) in buttons {
            if ui
                .add_sized([width, 24.0], egui::Button::new(label))
                .clicked()
            {
                command = Some(cmd);
            }
        }
    });

    command
}
