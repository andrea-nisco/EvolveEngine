//! The project's scenes: switching between them, and the Scenes panel that lists them in play order.
//!
//! One scene is edited at a time (`EditorApp::scene`). The project holds all scenes, but its
//! entry for the open one is stale until `sync_open_scene` copies it back; everything that needs
//! the whole project (saving, playing, exporting) calls that first.

use crate::app::EditorApp;
use crate::theme::ACCENT;
use eframe::egui::{self, Sense};
use evolve_engine::project::Project;

#[derive(Default)]
pub struct ScenesState {
    /// The scene whose name is being typed, and the text so far.
    pub renaming: Option<(usize, String)>,
}

/// What the user asked for in the list; applied after the rows are drawn.
enum Action {
    Open(usize),
    Rename(usize),
    Duplicate(usize),
    SetStart(usize),
    Move(usize, isize),
    Delete(usize),
    CommitName(usize, String),
}

impl EditorApp {
    /// Copies the scene being edited back into the project, so the project is complete.
    pub fn sync_open_scene(&mut self) {
        if let Some(slot) = self.project.scenes.get_mut(self.open) {
            *slot = self.scene.clone();
        }
    }

    fn scene_name_at(&self, index: usize) -> &str {
        if index == self.open {
            &self.scene.name
        } else {
            &self.project.scenes[index].name
        }
    }

    /// Switches the editor to another scene. Undo history and selection start over.
    pub fn open_scene(&mut self, index: usize) {
        if index == self.open || index >= self.project.scenes.len() {
            return;
        }
        self.sync_open_scene();
        self.open = index;
        self.scene = self.project.scenes[index].clone();
        self.reset_editing_state();
        self.status = format!("Opened scene \"{}\"", self.scene.name);
    }

    /// Changes the scene list, keeping the same scene open. Marks the project as changed.
    fn edit_scene_list<R>(&mut self, op: impl FnOnce(&mut Project) -> R) -> R {
        self.sync_open_scene();
        let open_name = self.scene.name.clone();
        let result = op(&mut self.project);
        self.open = self.project.index_of(&open_name).unwrap_or(0);
        self.dirty = true;
        result
    }

    pub fn add_scene(&mut self) {
        let index = self.edit_scene_list(|project| project.add_scene());
        self.open_scene(index);
    }

    fn duplicate_scene(&mut self, index: usize) {
        if let Some(copy) = self.edit_scene_list(|project| project.duplicate_scene(index)) {
            self.open_scene(copy);
        }
    }

    fn delete_scene(&mut self, index: usize) {
        if self.project.scenes.len() <= 1 {
            return;
        }
        if index != self.open {
            self.edit_scene_list(|project| project.remove_scene(index));
            return;
        }
        // Deleting the scene being edited: continue in its neighbour.
        self.sync_open_scene();
        self.project.remove_scene(index);
        self.open = index.min(self.project.scenes.len() - 1);
        self.scene = self.project.scenes[self.open].clone();
        self.reset_editing_state();
        self.dirty = true;
    }

    fn move_scene(&mut self, index: usize, by: isize) {
        let to = index.saturating_add_signed(by).min(self.project.scenes.len() - 1);
        self.edit_scene_list(|project| project.move_scene(index, to));
    }

    fn rename_scene(&mut self, index: usize, name: &str) {
        self.sync_open_scene();
        self.project.rename_scene(index, name);
        if index == self.open {
            self.scene.name = self.project.scenes[index].name.clone();
        }
        self.dirty = true;
    }

    /// The Scenes panel.
    pub fn scenes_pane(&mut self, ui: &mut egui::Ui) {
        let editable = self.play.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(editable, egui::Button::new("+ New scene")).clicked() {
                self.add_scene();
            }
            ui.weak("The game plays scenes in this order. Scripts switch with next_scene() or load_scene(\"name\").");
        });
        ui.separator();

        let mut actions = Vec::new();
        egui::ScrollArea::vertical().id_salt("scenes").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                for index in 0..self.project.scenes.len() {
                    self.scene_row(ui, index, &mut actions);
                }
            });
        });
        for action in actions {
            match action {
                Action::Open(i) => self.open_scene(i),
                Action::Rename(i) => self.scenes_ui.renaming = Some((i, self.scene_name_at(i).to_string())),
                Action::Duplicate(i) => self.duplicate_scene(i),
                Action::SetStart(i) => {
                    self.project.start = i;
                    self.dirty = true;
                }
                Action::Move(i, by) => self.move_scene(i, by),
                Action::Delete(i) => self.delete_scene(i),
                Action::CommitName(i, name) => {
                    self.rename_scene(i, &name);
                    self.scenes_ui.renaming = None;
                }
            }
        }
    }

    fn scene_row(&mut self, ui: &mut egui::Ui, index: usize, actions: &mut Vec<Action>) {
        let count = self.project.scenes.len();
        ui.horizontal(|ui| {
            ui.weak(format!("{}.", index + 1));
            if let Some((renaming, text)) = self.scenes_ui.renaming.as_mut().filter(|(i, _)| *i == index) {
                let edit = ui.add(egui::TextEdit::singleline(text).desired_width(160.0));
                edit.request_focus();
                if edit.lost_focus() {
                    actions.push(Action::CommitName(*renaming, text.clone()));
                }
                return;
            }
            let name = self.scene_name_at(index).to_string();
            let response = ui.selectable_label(index == self.open, name).interact(Sense::click());
            if response.double_clicked() {
                actions.push(Action::Rename(index));
            } else if response.clicked() {
                actions.push(Action::Open(index));
            }
            response.context_menu(|ui| {
                let mut item = |ui: &mut egui::Ui, label: &str, enabled: bool, action: Action| {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        actions.push(action);
                        ui.close();
                    }
                };
                item(ui, "Open", index != self.open, Action::Open(index));
                item(ui, "Rename", true, Action::Rename(index));
                item(ui, "Duplicate", true, Action::Duplicate(index));
                item(ui, "Set as start scene", index != self.project.start, Action::SetStart(index));
                ui.separator();
                item(ui, "Move up", index > 0, Action::Move(index, -1));
                item(ui, "Move down", index + 1 < count, Action::Move(index, 1));
                ui.separator();
                item(ui, "Delete", count > 1, Action::Delete(index));
            });
            if index == self.project.start {
                ui.colored_label(ACCENT, "start");
            }
        });
    }

    /// A drop-down at the top of the Objects panel for switching scenes quickly.
    pub fn scene_picker(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        egui::ComboBox::from_id_salt("scene_picker").selected_text(self.scene.name.clone()).width(150.0).show_ui(ui, |ui| {
            for index in 0..self.project.scenes.len() {
                if ui.selectable_label(index == self.open, self.scene_name_at(index)).clicked() {
                    chosen = Some(index);
                }
            }
        });
        if let Some(index) = chosen.filter(|_| self.play.is_none()) {
            self.open_scene(index);
        }
    }
}
