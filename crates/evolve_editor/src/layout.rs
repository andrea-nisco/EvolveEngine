//! The editor's panels as dockable tabs: drag a tab by its title to move it, drop it on the
//! edge of another panel to split, resize with the dividers. The arrangement is remembered
//! between sessions; Window â†’ Reset Layout restores the default.

use crate::app::EditorApp;
use eframe::egui::{self, Id, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Objects,
    Scene,
    Game,
    Inspector,
    Assets,
    Scenes,
    Timeline,
    Console,
}

impl Pane {
    pub const ALL: [Pane; 8] =
        [Pane::Objects, Pane::Scene, Pane::Game, Pane::Inspector, Pane::Assets, Pane::Scenes, Pane::Timeline, Pane::Console];

    pub fn title(self) -> &'static str {
        match self {
            Pane::Objects => "Objects",
            Pane::Scene => "Scene",
            Pane::Game => "Game",
            Pane::Inspector => "Inspector",
            Pane::Assets => "Assets",
            Pane::Scenes => "Scenes",
            Pane::Timeline => "Timeline",
            Pane::Console => "Console",
        }
    }
}

/// Objects on the left, the Scene and Game views in the middle, the inspector on the right, and the
/// Assets / Timeline / Console tabs along the bottom.
pub fn default_dock() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Scene]);
    let tree = dock.main_surface_mut();
    let [top, _bottom] = tree.split_below(NodeIndex::root(), 0.72, vec![Pane::Assets, Pane::Scenes, Pane::Timeline, Pane::Console]);
    // A split returns [old, new]. Its fraction is the share of the child that ends up first (left or
    // top), which is the new panel when splitting to the left.
    let [middle, _inspector] = tree.split_right(top, 0.78, vec![Pane::Inspector]);
    let [views, _objects] = tree.split_left(middle, 0.2, vec![Pane::Objects]);
    tree.split_right(views, 0.55, vec![Pane::Game]);
    dock
}

fn layout_file() -> PathBuf {
    crate::autosave::default_dir().join("layout.ron")
}

fn to_text(dock: &DockState<Pane>) -> Option<String> {
    ron::to_string(dock).ok()
}

/// A layout that cannot be used (unreadable, or without the Scene panel) is replaced by the default.
fn from_text(text: &str) -> DockState<Pane> {
    match ron::from_str::<DockState<Pane>>(text) {
        Ok(dock) if dock.find_tab(&Pane::Scene).is_some() => dock,
        _ => default_dock(),
    }
}

pub fn load() -> DockState<Pane> {
    std::fs::read_to_string(layout_file()).map_or_else(|_| default_dock(), |text| from_text(&text))
}

/// Runs the viewer for every panel the dock shows.
struct Viewer<'a> {
    app: &'a mut EditorApp,
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;

    fn id(&mut self, pane: &mut Pane) -> Id {
        Id::new(("pane", pane.title()))
    }

    fn title(&mut self, pane: &mut Pane) -> WidgetText {
        pane.title().into()
    }

    fn ui(&mut self, ui: &mut Ui, pane: &mut Pane) {
        match pane {
            Pane::Objects => self.app.objects_pane(ui),
            Pane::Scene => self.app.canvas_ui(ui),
            Pane::Game => self.app.game_pane(ui),
            Pane::Inspector => self.app.inspector_pane(ui),
            Pane::Assets => self.app.assets_contents(ui),
            Pane::Scenes => self.app.scenes_pane(ui),
            Pane::Timeline => {
                self.app.anim.timeline_visible = true;
                self.app.timeline_contents(ui);
            }
            Pane::Console => self.app.console_contents(ui),
        }
    }

    // The scene view zooms with the mouse wheel and the panels scroll themselves.
    fn scroll_bars(&self, _pane: &Pane) -> [bool; 2] {
        [false, false]
    }

    fn clear_background(&self, pane: &Pane) -> bool {
        !matches!(pane, Pane::Scene | Pane::Game)
    }

    fn allowed_in_windows(&self, _pane: &mut Pane) -> bool {
        false
    }
}

impl EditorApp {
    /// Draws all panels inside `ui` (everything below the menu bar and the tool strip).
    pub fn show_panes(&mut self, ui: &mut Ui) {
        let Some(mut dock) = self.dock.take() else { return };
        let mut style = Style::from_egui(ui.style());
        style.main_surface_border_stroke = egui::Stroke::NONE;
        DockArea::new(&mut dock)
            .style(style)
            .show_add_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show_inside(ui, &mut Viewer { app: self });
        self.dock = Some(dock);
    }

    pub fn pane_open(&self, pane: Pane) -> bool {
        self.dock.as_ref().is_some_and(|d| d.find_tab(&pane).is_some())
    }

    /// Brings a panel to the front, opening it again if the user closed it.
    pub fn show_pane(&mut self, pane: Pane) {
        let Some(dock) = &mut self.dock else { return };
        match dock.find_tab(&pane) {
            Some(path) => {
                let _ = dock.set_active_tab(path);
            }
            None => dock.push_to_focused_leaf(pane),
        }
    }

    pub fn close_pane(&mut self, pane: Pane) {
        let Some(dock) = &mut self.dock else { return };
        if let Some(path) = dock.find_tab(&pane) {
            dock.remove_tab(path);
        }
    }

    pub fn reset_layout(&mut self) {
        self.dock = Some(default_dock());
        self.status = "Layout reset".into();
    }

    /// Writes the layout when it changed (checked while no mouse button is down, so a divider
    /// drag is stored once, at the end).
    pub fn save_layout_if_changed(&mut self, busy: bool) {
        if busy {
            return;
        }
        let Some(text) = self.dock.as_ref().and_then(to_text) else { return };
        if text != self.saved_layout {
            let file = layout_file();
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(file, &text);
            self.saved_layout = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(dock: &DockState<Pane>) -> Vec<Pane> {
        dock.iter_all_tabs().map(|(_, p)| *p).collect()
    }

    #[test]
    fn the_default_layout_has_every_panel_once() {
        let dock = default_dock();
        let mut found = panes(&dock);
        found.sort_by_key(|p| p.title());
        let mut all = Pane::ALL.to_vec();
        all.sort_by_key(|p| p.title());
        assert_eq!(found, all);
    }

    #[test]
    fn a_layout_survives_saving_and_loading() {
        let mut dock = default_dock();
        let path = dock.find_tab(&Pane::Timeline).unwrap();
        dock.remove_tab(path);
        assert!(dock.find_tab(&Pane::Timeline).is_none(), "removal itself failed");
        let loaded = from_text(&to_text(&dock).unwrap());
        assert!(loaded.find_tab(&Pane::Timeline).is_none());
        assert!(loaded.find_tab(&Pane::Scene).is_some());
    }

    #[test]
    fn a_broken_or_scene_less_layout_falls_back_to_the_default() {
        assert_eq!(panes(&from_text("not json")).len(), Pane::ALL.len());
        let without_scene = DockState::new(vec![Pane::Objects]);
        assert_eq!(panes(&from_text(&to_text(&without_scene).unwrap())).len(), Pane::ALL.len());
    }
}
