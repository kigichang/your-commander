use crate::config::Config;
use crate::fs::local::{self, LocalFs};
use crate::fs::{FsEntry, FsProvider};
use rust_i18n::t;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    List,
    Icons,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Kind,
}

/// Actions a panel raises that the app level must handle.
pub enum PanelAction {
    Activate,
    Status(String),
    AddFavorite(PathBuf),
    Eject(PathBuf),
    RemoveFavorite(usize),
}

pub struct PanelState {
    pub side: usize, // 0 = left, 1 = right
    pub provider: Box<dyn FsProvider>,
    pub cwd: PathBuf,
    pub entries: Vec<FsEntry>,
    /// Names of selected entries (survives refresh/sort).
    pub selected: HashSet<String>,
    /// Cursor position as index into `visible()`.
    pub cursor: Option<usize>,
    /// Anchor for shift-range selection, index into `visible()`.
    pub anchor: Option<usize>,
    pub sort_key: SortKey,
    pub sort_asc: bool,
    pub filter: String,
    pub view: ViewMode,
    pub error: Option<String>,
    /// Some(text) while the path bar is in edit mode.
    pub path_edit: Option<String>,
    pub show_hidden: bool,
    /// Set when the path edit field should grab focus this frame.
    pub focus_path_edit: bool,
}

impl PanelState {
    pub fn new_local(side: usize, start: Option<PathBuf>, view: ViewMode, show_hidden: bool) -> Self {
        let provider: Box<dyn FsProvider> = Box::new(LocalFs);
        let cwd = start
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| provider.initial_dir());
        let mut p = Self {
            side,
            provider,
            cwd,
            entries: Vec::new(),
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
            sort_key: SortKey::Name,
            sort_asc: true,
            filter: String::new(),
            view,
            error: None,
            path_edit: None,
            show_hidden,
            focus_path_edit: false,
        };
        p.refresh();
        p
    }

    pub fn refresh(&mut self) {
        match self.provider.list_dir(&self.cwd.clone()) {
            Ok(mut entries) => {
                if !self.show_hidden {
                    entries.retain(|e| !e.is_hidden);
                }
                self.entries = entries;
                self.error = None;
                self.sort();
                // Drop selection of names that no longer exist.
                let names: HashSet<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
                self.selected.retain(|n| names.contains(n.as_str()));
                let count = self.visible().len();
                self.cursor = match self.cursor {
                    Some(c) if count > 0 => Some(c.min(count - 1)),
                    _ if count > 0 => None,
                    _ => None,
                };
            }
            Err(err) => {
                self.entries.clear();
                self.selected.clear();
                self.cursor = None;
                self.error = Some(err.to_string());
            }
        }
    }

    pub fn sort(&mut self) {
        let key = self.sort_key;
        let asc = self.sort_asc;
        self.entries.sort_by(|a, b| {
            // Folders always group before files.
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| {
                    let ord = match key {
                        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                        SortKey::Size => a.size.cmp(&b.size),
                        SortKey::Modified => a.modified.cmp(&b.modified),
                        SortKey::Kind => a
                            .ext
                            .cmp(&b.ext)
                            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                    };
                    if asc { ord } else { ord.reverse() }
                })
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    pub fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        self.sort();
    }

    /// Indices into `entries` that pass the quick filter, in display order.
    pub fn visible(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn navigate(&mut self, path: PathBuf) {
        self.cwd = path;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.filter.clear();
        self.path_edit = None;
        self.refresh();
    }

    pub fn go_parent(&mut self) {
        let from = self.cwd.clone();
        if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
            self.navigate(parent);
            // Put the cursor on the directory we just came from.
            if let Some(name) = from.file_name().map(|n| n.to_string_lossy().to_string()) {
                let vis = self.visible();
                if let Some(pos) = vis
                    .iter()
                    .position(|&i| self.entries[i].name == name)
                {
                    self.cursor = Some(pos);
                    self.selected.insert(name);
                }
            }
        }
    }

    pub fn go_home(&mut self) {
        let home = self.provider.initial_dir();
        self.navigate(home);
    }

    /// Replace the provider (e.g. a fresh SFTP session) and jump to its home.
    pub fn set_provider(&mut self, provider: Box<dyn FsProvider>) {
        let home = provider.initial_dir();
        self.provider = provider;
        self.navigate(home);
    }

    /// Fall back to the local disk (dropping any remote session).
    pub fn ensure_local(&mut self) {
        if !self.provider.is_local() {
            self.provider = Box::new(LocalFs);
        }
    }

    /// Open the entry under the cursor: enter directories, open files.
    pub fn open_cursor(&mut self) -> Option<String> {
        let vis = self.visible();
        let idx = *vis.get(self.cursor?)?;
        let entry = self.entries[idx].clone();
        if entry.is_dir {
            self.navigate(entry.path);
            None
        } else if self.provider.is_local() {
            if let Err(err) = open::that_detached(&entry.path) {
                return Some(format!("{}: {err}", t!("status.open_failed")));
            }
            None
        } else {
            Some(t!("status.remote_open_unsupported").to_string())
        }
    }

    /// Mac-style click selection.
    pub fn click_select(&mut self, vis_pos: usize, cmd: bool, shift: bool) {
        let vis = self.visible();
        let Some(&entry_idx) = vis.get(vis_pos) else {
            return;
        };
        let name = self.entries[entry_idx].name.clone();
        if shift {
            let anchor = self.anchor.or(self.cursor).unwrap_or(vis_pos);
            let (lo, hi) = (anchor.min(vis_pos), anchor.max(vis_pos));
            if !cmd {
                self.selected.clear();
            }
            for &i in &vis[lo..=hi] {
                self.selected.insert(self.entries[i].name.clone());
            }
            self.anchor = Some(anchor);
        } else if cmd {
            if !self.selected.remove(&name) {
                self.selected.insert(name);
            }
            self.anchor = Some(vis_pos);
        } else {
            self.selected.clear();
            self.selected.insert(name);
            self.anchor = Some(vis_pos);
        }
        self.cursor = Some(vis_pos);
    }

    pub fn move_cursor(&mut self, delta: isize, shift: bool) {
        let vis = self.visible();
        if vis.is_empty() {
            return;
        }
        let cur = self.cursor.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, vis.len() as isize - 1) as usize;
        if shift {
            let anchor = self.anchor.or(self.cursor).unwrap_or(next);
            let (lo, hi) = (anchor.min(next), anchor.max(next));
            self.selected.clear();
            for &i in &vis[lo..=hi] {
                self.selected.insert(self.entries[i].name.clone());
            }
            self.anchor = Some(anchor);
        } else {
            self.selected.clear();
            self.selected
                .insert(self.entries[vis[next]].name.clone());
            self.anchor = Some(next);
        }
        self.cursor = Some(next);
    }

    pub fn select_all(&mut self) {
        let vis = self.visible();
        self.selected = vis
            .iter()
            .map(|&i| self.entries[i].name.clone())
            .collect();
    }

    pub fn selected_entries(&self) -> Vec<&FsEntry> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.name))
            .collect()
    }

    pub fn selection_summary(&self) -> String {
        let total = self.visible().len();
        let sel = self.selected_entries();
        if sel.is_empty() {
            t!("status.items", count = total).to_string()
        } else {
            let bytes: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
            t!(
                "status.selected",
                count = sel.len(),
                total = total,
                size = crate::util::human_size(bytes)
            )
            .to_string()
        }
    }
}

/// Draw one panel (toolbar, path bar, file list). Returns actions for the app.
pub fn show_panel(
    ui: &mut egui::Ui,
    panel: &mut PanelState,
    config: &Config,
    is_active: bool,
) -> Vec<PanelAction> {
    let mut actions = Vec::new();

    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::same(6))
        .stroke(if is_active {
            egui::Stroke::new(1.5, ui.visuals().selection.bg_fill)
        } else {
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
        })
        .corner_radius(4.0);

    let response = frame
        .show(ui, |ui| {
            panel_toolbar(ui, panel, config, &mut actions);
            path_bar(ui, panel);
            ui.add_space(4.0);
            if let Some(err) = panel.error.clone() {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            let list_actions = match panel.view {
                ViewMode::List => super::file_list::show_list(ui, panel, is_active),
                ViewMode::Icons => super::file_list::show_icons(ui, panel, is_active),
            };
            actions.extend(list_actions);
            if !panel.filter.is_empty() {
                ui.label(
                    egui::RichText::new(format!("🔍 {}", panel.filter))
                        .color(ui.visuals().hyperlink_color),
                );
            }
        })
        .response;

    if response.interact(egui::Sense::click()).clicked() {
        actions.push(PanelAction::Activate);
    }

    actions
}

fn panel_toolbar(
    ui: &mut egui::Ui,
    panel: &mut PanelState,
    config: &Config,
    actions: &mut Vec<PanelAction>,
) {
    ui.horizontal(|ui| {
        if ui
            .button("🏠")
            .on_hover_text(t!("panel.home"))
            .clicked()
        {
            panel.go_home();
            actions.push(PanelAction::Activate);
        }

        ui.menu_button(format!("★ {}", t!("panel.favorites")), |ui| {
            if config.favorites.is_empty() {
                ui.label(t!("panel.no_favorites"));
            }
            for (i, fav) in config.favorites.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(&fav.name).on_hover_text(fav.path.to_string_lossy()).clicked() {
                        panel.ensure_local();
                        panel.navigate(fav.path.clone());
                        actions.push(PanelAction::Activate);
                        ui.close();
                    }
                    if ui
                        .small_button("🗑")
                        .on_hover_text(t!("panel.remove_favorite"))
                        .clicked()
                    {
                        actions.push(PanelAction::RemoveFavorite(i));
                        ui.close();
                    }
                });
            }
            ui.separator();
            if ui.button(t!("panel.add_favorite")).clicked() {
                actions.push(PanelAction::AddFavorite(panel.cwd.clone()));
                ui.close();
            }
        });

        ui.menu_button(format!("🖴 {}", t!("panel.drives")), |ui| {
            let vols = local::volumes();
            if vols.is_empty() {
                ui.label(t!("panel.no_drives"));
            }
            for (name, path) in vols {
                ui.horizontal(|ui| {
                    if ui.button(&name).clicked() {
                        panel.ensure_local();
                        panel.navigate(path.clone());
                        actions.push(PanelAction::Activate);
                        ui.close();
                    }
                    if ui
                        .small_button("⏏")
                        .on_hover_text(t!("panel.eject"))
                        .clicked()
                    {
                        actions.push(PanelAction::Eject(path.clone()));
                        ui.close();
                    }
                });
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("⟳")
                .on_hover_text(t!("panel.refresh"))
                .clicked()
            {
                panel.refresh();
            }
            if !panel.provider.is_local()
                && ui
                    .button(format!("⏻ {}", panel.provider.display_prefix()))
                    .on_hover_text(t!("panel.disconnect"))
                    .clicked()
            {
                panel.ensure_local();
                panel.go_home();
            }
        });
    });
}

fn path_bar(ui: &mut egui::Ui, panel: &mut PanelState) {
    ui.horizontal(|ui| {
        if let Some(text) = &mut panel.path_edit {
            let edit = egui::TextEdit::singleline(text)
                .desired_width(f32::INFINITY)
                .id_salt(("path_edit", panel.side));
            let resp = ui.add(edit);
            if panel.focus_path_edit {
                resp.request_focus();
                panel.focus_path_edit = false;
            }
            let confirmed = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if confirmed {
                let path = PathBuf::from(shellexpand_home(text));
                if path.is_dir() {
                    panel.navigate(path);
                } else {
                    panel.error = Some(t!("status.not_a_dir", path = text.clone()).to_string());
                    panel.path_edit = None;
                }
            } else if cancelled {
                panel.path_edit = None;
            }
            return;
        }

        if ui
            .button("✏")
            .on_hover_text(t!("panel.edit_path"))
            .clicked()
        {
            panel.path_edit = Some(panel.cwd.to_string_lossy().to_string());
            panel.focus_path_edit = true;
            return;
        }

        let prefix = panel.provider.display_prefix();
        if !prefix.is_empty() {
            ui.label(egui::RichText::new(prefix).strong());
        }

        // Breadcrumb: every component navigates to its prefix path.
        egui::ScrollArea::horizontal()
            .id_salt(("breadcrumb", panel.side))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut accum = PathBuf::new();
                    let components: Vec<_> = panel.cwd.components().collect();
                    let mut target: Option<PathBuf> = None;
                    for (i, comp) in components.iter().enumerate() {
                        accum.push(comp);
                        let label = match comp {
                            std::path::Component::RootDir => "/".to_string(),
                            other => other.as_os_str().to_string_lossy().to_string(),
                        };
                        let is_last = i == components.len() - 1;
                        let text = if is_last {
                            egui::RichText::new(label).strong()
                        } else {
                            egui::RichText::new(label)
                        };
                        if ui.add(egui::Button::new(text).frame(false)).clicked() && !is_last {
                            target = Some(accum.clone());
                        }
                        if !is_last && i > 0 {
                            ui.label("›");
                        }
                    }
                    if let Some(t) = target {
                        panel.navigate(t);
                    }
                });
            });
    });
}

fn shellexpand_home(input: &str) -> String {
    if let Some(rest) = input.strip_prefix("~") {
        let home = local::home_dir();
        format!("{}{}", home.to_string_lossy(), rest)
    } else {
        input.to_string()
    }
}
