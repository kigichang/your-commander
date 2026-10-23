use super::panel::ViewMode;
use rust_i18n::t;

pub enum ToolbarAction {
    SetView(ViewMode),
    Pack,
    Unpack,
    ConnectSmb,
    ConnectSsh,
    ToggleShortcuts,
}

/// Main toolbar: view-mode toggle for the active panel + network connections.
pub fn show_toolbar(ui: &mut egui::Ui, active_view: ViewMode) -> Vec<ToolbarAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        if ui
            .selectable_label(active_view == ViewMode::Icons, format!("🗔 {}", t!("toolbar.icons")))
            .on_hover_text(t!("toolbar.icons_hint"))
            .clicked()
        {
            actions.push(ToolbarAction::SetView(ViewMode::Icons));
        }
        if ui
            .selectable_label(active_view == ViewMode::List, format!("☰ {}", t!("toolbar.list")))
            .on_hover_text(t!("toolbar.list_hint"))
            .clicked()
        {
            actions.push(ToolbarAction::SetView(ViewMode::List));
        }

        ui.separator();

        if ui
            .button(format!("📦 {}", t!("toolbar.pack")))
            .on_hover_text(t!("toolbar.pack_hint"))
            .clicked()
        {
            actions.push(ToolbarAction::Pack);
        }
        if ui
            .button(format!("📤 {}", t!("toolbar.unpack")))
            .on_hover_text(t!("toolbar.unpack_hint"))
            .clicked()
        {
            actions.push(ToolbarAction::Unpack);
        }

        ui.separator();

        if ui.button(format!("🖧 {}", t!("toolbar.smb"))).clicked() {
            actions.push(ToolbarAction::ConnectSmb);
        }
        if ui.button(format!("🔐 {}", t!("toolbar.ssh"))).clicked() {
            actions.push(ToolbarAction::ConnectSsh);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("⌨")
                .on_hover_text(t!("toolbar.shortcuts_hint"))
                .clicked()
            {
                actions.push(ToolbarAction::ToggleShortcuts);
            }
        });
    });
    actions
}
