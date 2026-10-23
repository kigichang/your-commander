use super::panel::{PanelAction, PanelState, SortKey};
use crate::fs::FsEntry;
use crate::util;
use egui_extras::{Column, TableBuilder};
use rust_i18n::t;

pub fn icon_for(entry: &FsEntry) -> &'static str {
    if entry.is_symlink && entry.is_dir {
        return "📁";
    }
    if entry.is_dir {
        return "📁";
    }
    match entry.ext.as_str() {
        "zip" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "rar" => "📦",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "svg" | "tiff" => "🖼",
        "mp4" | "mov" | "avi" | "mkv" | "webm" => "🎬",
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" => "🎵",
        "pdf" => "📕",
        "doc" | "docx" | "txt" | "md" | "rtf" | "pages" => "📝",
        "xls" | "xlsx" | "csv" | "numbers" => "📊",
        "app" => "🚀",
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "go" | "java" | "swift" | "sh"
        | "toml" | "json" | "yml" | "yaml" | "html" | "css" => "📃",
        _ => "📄",
    }
}

/// Detail/list view backed by a virtualized table.
pub fn show_list(ui: &mut egui::Ui, panel: &mut PanelState, is_active: bool) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    let vis = panel.visible();
    let row_height = 22.0;

    let mut clicked: Option<(usize, bool, bool)> = None; // (vis_pos, cmd, shift)
    let mut double_clicked = false;
    let mut sort_request: Option<SortKey> = None;

    let header = |name: String, key: SortKey, panel: &PanelState, ui: &mut egui::Ui,
                  sort_request: &mut Option<SortKey>| {
        let arrow = if panel.sort_key == key {
            if panel.sort_asc { " ⏶" } else { " ⏷" }
        } else {
            ""
        };
        if ui
            .add(egui::Button::new(egui::RichText::new(format!("{name}{arrow}")).strong()).frame(false))
            .clicked()
        {
            *sort_request = Some(key);
        }
    };

    TableBuilder::new(ui)
        .id_salt(("file_table", panel.side))
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .column(Column::remainder().at_least(140.0).clip(true))
        .column(Column::auto_with_initial_suggestion(80.0).at_least(60.0))
        .column(Column::auto_with_initial_suggestion(130.0).at_least(100.0))
        .column(Column::auto_with_initial_suggestion(70.0).at_least(50.0))
        .header(24.0, |mut row| {
            row.col(|ui| header(t!("list.name").to_string(), SortKey::Name, panel, ui, &mut sort_request));
            row.col(|ui| header(t!("list.size").to_string(), SortKey::Size, panel, ui, &mut sort_request));
            row.col(|ui| header(t!("list.date").to_string(), SortKey::Modified, panel, ui, &mut sort_request));
            row.col(|ui| header(t!("list.kind").to_string(), SortKey::Kind, panel, ui, &mut sort_request));
        })
        .body(|body| {
            body.rows(row_height, vis.len(), |mut row| {
                let vis_pos = row.index();
                let entry = &panel.entries[vis[vis_pos]];
                let is_selected = panel.selected.contains(&entry.name);
                row.set_selected(is_selected);

                row.col(|ui| {
                    let cursor_mark = if is_active && panel.cursor == Some(vis_pos) {
                        "▸"
                    } else {
                        " "
                    };
                    let name = if entry.is_symlink {
                        format!("{cursor_mark}{} {} ↩", icon_for(entry), entry.name)
                    } else {
                        format!("{cursor_mark}{} {}", icon_for(entry), entry.name)
                    };
                    ui.add(egui::Label::new(name).selectable(false).truncate());
                });
                row.col(|ui| {
                    let text = if entry.is_dir {
                        "--".to_string()
                    } else {
                        util::human_size(entry.size)
                    };
                    ui.add(egui::Label::new(text).selectable(false));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(util::format_time(entry.modified)).selectable(false));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(entry.kind_label()).selectable(false));
                });

                let resp = row.response();
                if resp.double_clicked() {
                    clicked = Some((vis_pos, false, false));
                    double_clicked = true;
                } else if resp.clicked() {
                    let (cmd, shift) = resp
                        .ctx
                        .input(|i| (i.modifiers.command, i.modifiers.shift));
                    clicked = Some((vis_pos, cmd, shift));
                }
            });
        });

    if let Some(key) = sort_request {
        panel.set_sort(key);
    }
    if let Some((pos, cmd, shift)) = clicked {
        panel.click_select(pos, cmd, shift);
        actions.push(PanelAction::Activate);
        if double_clicked
            && let Some(msg) = panel.open_cursor()
        {
            actions.push(PanelAction::Status(msg));
        }
    }
    actions
}

/// Big-icon grid view.
pub fn show_icons(ui: &mut egui::Ui, panel: &mut PanelState, is_active: bool) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    let vis = panel.visible();
    let tile = egui::vec2(96.0, 92.0);

    let mut clicked: Option<(usize, bool, bool)> = None;
    let mut double_clicked = false;

    egui::ScrollArea::vertical()
        .id_salt(("icon_grid", panel.side))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for (vis_pos, &entry_idx) in vis.iter().enumerate() {
                    let entry = &panel.entries[entry_idx];
                    let is_selected = panel.selected.contains(&entry.name);
                    let (rect, resp) = ui.allocate_exact_size(tile, egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let visuals = ui.visuals();
                        if is_selected {
                            ui.painter().rect_filled(rect, 6.0, visuals.selection.bg_fill);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                6.0,
                                visuals.widgets.hovered.weak_bg_fill,
                            );
                        }
                        if is_active && panel.cursor == Some(vis_pos) {
                            ui.painter().rect_stroke(
                                rect,
                                6.0,
                                egui::Stroke::new(1.5, visuals.selection.stroke.color),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let text_color = if is_selected {
                            visuals.selection.stroke.color
                        } else {
                            visuals.text_color()
                        };
                        ui.painter().text(
                            rect.center_top() + egui::vec2(0.0, 28.0),
                            egui::Align2::CENTER_CENTER,
                            icon_for(entry),
                            egui::FontId::proportional(34.0),
                            text_color,
                        );
                        ui.painter().text(
                            rect.center_bottom() - egui::vec2(0.0, 16.0),
                            egui::Align2::CENTER_CENTER,
                            util::truncate_middle(&entry.name, 14),
                            egui::FontId::proportional(12.0),
                            text_color,
                        );
                    }
                    if resp.double_clicked() {
                        clicked = Some((vis_pos, false, false));
                        double_clicked = true;
                    } else if resp.clicked() {
                        let (cmd, shift) =
                            ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                        clicked = Some((vis_pos, cmd, shift));
                    }
                    resp.on_hover_text(&entry.name);
                }
            });
        });

    if let Some((pos, cmd, shift)) = clicked {
        panel.click_select(pos, cmd, shift);
        actions.push(PanelAction::Activate);
        if double_clicked
            && let Some(msg) = panel.open_cursor()
        {
            actions.push(PanelAction::Status(msg));
        }
    }
    actions
}
