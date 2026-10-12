//! The project's scenes: switching between them, and the Scenes panel that lists them in play order.
//!
//! One scene is edited at a time (`EditorApp::scene`). The project holds all scenes, but its
//! entry for the open one is stale until `sync_open_scene` copies it back; everything that needs
//! the whole project (saving, playing, exporting) calls that first.

use crate::app::EditorApp;
use crate::theme::ACCENT;
use eframe::egui::{self, Align2, Sense};
use evolve_engine::project::Project;

#[derive(Default)]
pub struct ScenesState {
    /// The scene whose name is being typed, and the text so far.
    pub renaming: Option<(usize, String)>,
    /// A scene waiting for the user to confirm its deletion (index and name at the time of asking).
    pub confirm_delete: Option<(usize, String)>,
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
    CancelRename,
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

    /// Asks before deleting a scene, since scene-list changes cannot be undone.
    pub fn delete_scene_dialog(&mut self, ctx: &egui::Context) {
        let Some((index, name)) = self.scenes_ui.confirm_delete.clone() else { return };
        // The list may have changed while the question was open (or the game started).
        if self.play.is_some() || index >= self.project.scenes.len() || self.scene_name_at(index) != name {
            self.scenes_ui.confirm_delete = None;
            return;
        }
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Delete scene").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("Delete the scene \"{name}\" and everything in it?"));
            ui.weak("This cannot be undone.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                confirmed = ui.button("Delete").clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
        if confirmed || cancelled || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.scenes_ui.confirm_delete = None;
        }
        if confirmed {
            self.delete_scene(index);
            self.status = format!("Deleted scene \"{name}\"");
        }
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
                Action::Delete(i) => self.scenes_ui.confirm_delete = Some((i, self.scene_name_at(i).to_string())),
                Action::CancelRename => self.scenes_ui.renaming = None,
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
                // Ask for focus only until the field has it: asking every frame would cancel the
                // focus loss that Enter (or a click elsewhere) causes, and the box would never close.
                if edit.lost_focus() {
                    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    actions.push(if cancelled { Action::CancelRename } else { Action::CommitName(*renaming, text.clone()) });
                } else if !edit.has_focus() {
                    edit.request_focus();
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
