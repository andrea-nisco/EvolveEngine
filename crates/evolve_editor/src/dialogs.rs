//! Modal dialogs: unsaved changes, crash recovery and image-export options. Also the logic that
//! decides when to ask, and the periodic autosave.

use crate::app::EditorApp;
use crate::autosave::{self, Recovery};
use eframe::egui::{self, Align2};
use std::time::{Duration, Instant};

/// Something that would discard the current document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    New,
    Open,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Sequence,
}

/// Choices for a PNG or PNG-sequence export.
pub struct ImageExport {
    pub kind: ImageKind,
    pub scale: f32,
    pub transparent: bool,
}

#[derive(Default)]
pub struct Dialogs {
    pub pending: Option<Pending>,
    pub recovery: Option<Recovery>,
    pub image_export: Option<ImageExport>,
}

/// Timers and flags behind the dialogs.
pub struct DialogState {
    pub last_autosave: Instant,
    /// The window may close without asking.
    pub allow_close: bool,
    /// Ask the window to close on the next frame.
    pub close_requested: bool,
}

impl Default for DialogState {
    fn default() -> Self {
        Self { last_autosave: Instant::now(), allow_close: false, close_requested: false }
    }
}

impl EditorApp {
    /// Runs `action` now, or after asking what to do with unsaved changes.
    pub fn request(&mut self, action: Pending) {
        if self.dirty {
            self.dialogs.pending = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_document(),
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
                autosave::clear();
                self.dialog_state.allow_close = true;
                self.dialog_state.close_requested = true;
            }
        }
    }

    /// Intercepts the window's close button while there are unsaved changes, and closes the
    /// window when asked to.
    pub fn watch_window_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.dialog_state.allow_close {
            if self.dirty {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialogs.pending = Some(Pending::Quit);
            } else {
                autosave::clear();
            }
        }
        if std::mem::take(&mut self.dialog_state.close_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Writes a recovery copy when there is unsaved work and a few seconds have passed.
    pub fn autosave_tick(&mut self, ctx: &egui::Context, busy: bool) {
        if self.dirty {
            // egui only redraws when something happens; make sure the timer is checked while idle.
            ctx.request_repaint_after(Duration::from_secs(5));
        }
        let due = self.dialog_state.last_autosave.elapsed() >= Duration::from_secs(autosave::INTERVAL_SECONDS);
        if self.dirty && !busy && due && self.play.is_none() {
            self.sync_open_scene();
            let _ = autosave::write(&self.project, self.file.as_deref());
            self.dialog_state.last_autosave = Instant::now();
        }
    }

    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        self.unsaved_dialog(ctx);
        self.recovery_dialog(ctx);
        self.image_export_dialog(ctx);
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.dialogs.pending else { return };
        let name = self.file.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy());
        let mut choice = None;
        egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("\"{name}\" has changes that are not saved."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(true);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.dialogs.pending = None;
                }
            });
        });
        match choice {
            Some(true) => {
                self.save();
                if !self.dirty {
                    self.dialogs.pending = None;
                    self.perform(action);
                }
            }
            Some(false) => {
                self.dialogs.pending = None;
                self.dirty = false;
                autosave::clear();
                self.perform(action);
            }
            None => {}
        }
    }

    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        if self.dialogs.recovery.is_none() {
            return;
        }
        let mut choice = None;
        egui::Window::new("Recover unsaved work?").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("The editor closed unexpectedly last time. A recovery copy of your work was found.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    choice = Some(true);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(false);
                }
            });
        });
        let Some(recover) = choice else { return };
        if let (true, Some(recovery)) = (recover, self.dialogs.recovery.take()) {
            self.recover(recovery);
        } else {
            self.dialogs.recovery = None;
            autosave::clear();
        }
    }

    fn image_export_dialog(&mut self, ctx: &egui::Context) {
        let Some(options) = &mut self.dialogs.image_export else { return };
        let title = if options.kind == ImageKind::Png { "Export PNG" } else { "Export PNG sequence" };
        let size = self.scene.camera_view().size;
        let mut action = None;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Scale");
                ui.add(egui::DragValue::new(&mut options.scale).speed(0.05).range(0.1..=8.0).suffix("x"));
                ui.weak(format!("= {} x {} pixels", (size.x * options.scale).ceil(), (size.y * options.scale).ceil()));
            });
            ui.checkbox(&mut options.transparent, "Transparent background");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Export...").clicked() {
                    action = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    action = Some(false);
                }
            });
        });
        match action {
            Some(true) => self.run_image_export(),
            Some(false) => self.dialogs.image_export = None,
            None => {}
        }
    }
}
