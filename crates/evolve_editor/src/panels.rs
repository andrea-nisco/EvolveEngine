//! Menu bar, tool bar and status bar.

use crate::renderers::RendererChoice;
use evolve_gpu::profile::Quality;
use crate::app::EditorApp;
use crate::dialogs::{ImageKind, Pending};
use crate::frame_limit::FrameLimit;
use crate::layout::Pane;
use crate::theme;
use crate::tools::Tool;
use eframe::egui::{self, Panel};
use evolve_engine::boolean::BoolOp;

#[cfg(target_os = "macos")]
const CMD: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const CMD: &str = "Ctrl";

/// A menu entry with a shortcut hint. Closes the menu when clicked.
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let shortcut = shortcut.replace("Cmd", CMD);
    let clicked = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl EditorApp {
    pub fn menu_bar(&mut self, ui: &mut egui::Ui) {
        Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if item(ui, "New", "Cmd+N", true) {
                        self.request(Pending::New);
                    }
                    if item(ui, "Open...", "Cmd+O", true) {
                        self.request(Pending::Open);
                    }
                    ui.separator();
                    if item(ui, "Save", "Cmd+S", true) {
                        self.save();
                    }
                    if item(ui, "Save As...", "Cmd+Shift+S", true) {
                        self.save_as();
                    }
                    ui.separator();
                    if item(ui, "Import SVG...", "", true) {
                        self.import_svg(false);
                    }
                    if item(ui, "Import SVG as Symbol...", "", true) {
                        self.import_svg(true);
                    }
                    ui.separator();
                    ui.menu_button("Export Picture", |ui| {
                        if item(ui, "PNG Image...", "", true) {
                            self.begin_image_export(ImageKind::Png);
                        }
                        if item(ui, "SVG Drawing...", "", true) {
                            self.export_svg_dialog();
                        }
                        if item(ui, "PNG Sequence (current clip)...", "", self.current_clip().is_some()) {
                            self.begin_image_export(ImageKind::Sequence);
                        }
                    });
                    if item(ui, "Export Game (desktop)...", "", true) {
                        self.export_desktop_dialog();
                    }
                    if item(ui, "Export for Web (HTML)...", "", true) {
                        self.export_web_dialog();
                    }
                    ui.separator();
                    if item(ui, "Quit", "", true) {
                        self.request(Pending::Quit);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let has_selection = !self.selection.is_empty();
                    if item(ui, "Undo", "Cmd+Z", self.history.can_undo()) {
                        self.undo();
                    }
                    if item(ui, "Redo", "Cmd+Shift+Z", self.history.can_redo()) {
                        self.redo();
                    }
                    ui.separator();
                    let ctx = ui.ctx().clone();
                    if item(ui, "Cut", "Cmd+X", has_selection) {
                        self.cut_selection(&ctx);
                    }
                    if item(ui, "Copy", "Cmd+C", has_selection) {
                        self.copy_selection(&ctx);
                    }
                    if item(ui, "Paste", "Cmd+V", self.clipboard.is_some()) {
                        self.paste(None, false);
                    }
                    if item(ui, "Paste in Place", "Cmd+Shift+V", self.clipboard.is_some()) {
                        self.paste(None, true);
                    }
                    ui.separator();
                    if item(ui, "Duplicate", "Cmd+D", has_selection) {
                        self.duplicate_selection();
                    }
                    if item(ui, "Delete", "Del", has_selection) {
                        self.delete_selection();
                    }
                    if item(ui, "Select All", "Cmd+A", true) {
                        self.select_all();
                    }
                });
                ui.menu_button("Object", |ui| {
                    let has_selection = !self.selection.is_empty();
                    let several = self.selection.len() > 1;
                    if item(ui, "Group", "Cmd+G", has_selection) {
                        self.group_selection();
                    }
                    if item(ui, "Ungroup", "Cmd+Shift+G", has_selection) {
                        self.ungroup_selection();
                    }
                    ui.separator();
                    if item(ui, "Combine into Compound Path", "Cmd+J", several) {
                        self.combine_selection();
                    }
                    if item(ui, "Release Compound Path", "Cmd+Shift+J", has_selection) {
                        self.release_compound_selection();
                    }
                    ui.menu_button("Shape Operations", |ui| {
                        for (label, op) in [
                            ("Union", BoolOp::Union),
                            ("Subtract (lowest minus the rest)", BoolOp::Difference),
                            ("Intersect", BoolOp::Intersection),
                            ("Exclude Overlap", BoolOp::Exclusion),
                        ] {
                            if item(ui, label, "", several) {
                                self.boolean_selection(op);
                            }
                        }
                    });
                    ui.separator();
                    if item(ui, "Create Symbol from Selection", "", has_selection) {
                        self.create_symbol_from_selection();
                    }
                    if item(ui, "Detach Instance", "", has_selection) {
                        self.detach_selected_instances();
                    }
                    if item(ui, "Set Origin to Center", "", has_selection) {
                        self.center_origin_selection();
                    }
                    ui.separator();
                    if item(ui, "Bind Shape to Selected Bones", "", several) {
                        self.bind_selection();
                    }
                    if item(ui, "Create IK (selected bone to target)", "", several) {
                        self.create_ik_selection();
                    }
                    ui.separator();
                    if item(ui, "Make Clipping Mask", "Cmd+7", several) {
                        self.make_clip_selection();
                    }
                    if item(ui, "Release Clipping Mask", "Cmd+Alt+7", has_selection) {
                        self.release_clip_selection();
                    }
                    ui.separator();
                    if item(ui, "Cut Path at Anchor", "X", self.canvas.anchor.is_some()) {
                        self.split_selected_anchor();
                    }
                    ui.separator();
                    if item(ui, "Bring Forward", "]", has_selection) {
                        self.shift_selection_order(true);
                    }
                    if item(ui, "Send Backward", "[", has_selection) {
                        self.shift_selection_order(false);
                    }
                });
                ui.menu_button("Game", |ui| {
                    let running = self.play.is_some();
                    if item(ui, if running { "Stop" } else { "Run this scene" }, "F5", true) {
                        self.toggle_game();
                    }
                    if item(ui, "Run from the start scene", "Shift+F5", !running) {
                        self.start_game_from_start();
                    }
                    let paused = self.play.as_ref().is_some_and(|p| p.paused);
                    if item(ui, if paused { "Resume" } else { "Pause" }, "F6", running) {
                        self.toggle_game_pause();
                    }
                    ui.separator();
                    ui.checkbox(&mut self.game_view.colliders, "Show collision shapes");
                    ui.checkbox(&mut self.game_view.anchors, "Show path anchors");
                    ui.checkbox(&mut self.game_view.stats, "Show statistics");
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.view.show_grid, "Show grid");
                    ui.checkbox(&mut self.view.snap_to_grid, "Snap to grid");
                    ui.checkbox(&mut self.view.smart_guides, "Smart guides");
                    ui.checkbox(&mut self.view.show_frame_times, "Show frame times");
                    ui.horizontal(|ui| {
                        ui.label("Grid size");
                        ui.add(egui::DragValue::new(&mut self.view.grid_size).range(1.0..=500.0).speed(0.5));
                    });
                    ui.separator();
                    if item(ui, "Fit camera frame", "Home", true) {
                        self.frame_camera();
                    }
                    if item(ui, "Zoom to selection", "F", !self.selection.is_empty()) {
                        self.frame_selection();
                    }
                    if item(ui, "Reset rotation", "", true) {
                        self.camera.rotation = 0.0;
                    }
                    ui.separator();
                    ui.menu_button("Frame rate", |ui| {
                        ui.label("While a game runs or a clip plays");
                        for limit in FrameLimit::ALL {
                            if ui.radio(self.frame_limit == limit, limit.to_string()).clicked() {
                                self.frame_limit = limit;
                                self.status = format!("Frame rate: {limit}");
                            }
                        }
                        ui.weak("A cap on top of the display's own rate: on a 240 Hz display 140 lands on its steps (120 Hz, mostly). Idle, the editor draws nothing.");
                    });
                    ui.menu_button("Renderer", |ui| {
                        for choice in RendererChoice::ALL {
                            if ui.radio(self.renderers.choice == choice, choice.to_string()).clicked() {
                                self.renderers.choice = choice;
                                self.status = format!("{choice} renderer");
                            }
                        }
                        ui.separator();
                        ui.label("Quality of the GPU renderer");
                        for quality in Quality::ALL {
                            if ui.radio(self.renderers.quality() == quality, quality.to_string()).clicked() {
                                self.renderers.set_quality(quality);
                                self.status = format!("{quality} quality");
                            }
                        }
                        if let Some(profile) = self.renderers.adapter_profile() {
                            ui.weak(format!("Automatic starts from {profile}, which suits this card, and draws smaller pictures while frames are slow."));
                        }
                        ui.separator();
                        match (self.renderers.disabled_because(), self.renderers.adapter()) {
                            (Some(why), _) => ui.weak(why),
                            (None, Some(adapter)) => ui.weak(format!("Graphics card: {adapter}")),
                            (None, None) => ui.weak("No usable graphics card: the GPU renderer is not available"),
                        };
                        ui.weak("A few things the GPU renderer cannot draw yet (a skinned clip mask, a skinned shape inside a symbol instance); views that have them use the CPU renderer.");
                    });
                });
                ui.menu_button("Window", |ui| {
                    for pane in Pane::ALL {
                        let open = self.pane_open(pane);
                        let label = format!("{}  {}", if open { "✔" } else { "   " }, pane.title());
                        if ui.add(egui::Button::new(label).frame(false)).clicked() {
                            if open {
                                self.close_pane(pane);
                            } else {
                                self.show_pane(pane);
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if item(ui, "Reset Layout", "", true) {
                        self.reset_layout();
                    }
                });
            });
        });
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        Panel::left("toolbar").exact_size(88.0).resizable(false).show(ui, |ui| {
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                for tool in Tool::ALL {
                    let button = egui::Button::new(tool.label()).selected(self.tool == tool).min_size(egui::vec2(72.0, 32.0));
                    let tooltip = format!("{} ({})", tool.label(), tool.key().name());
                    if ui.add(button).on_hover_text(tooltip).clicked() {
                        self.set_tool(tool);
                    }
                }
            });
        });
    }

    /// The renderer choice, always in reach (it is also in View → Renderer).
    fn renderer_selector(&mut self, ui: &mut egui::Ui) {
        let was = self.renderers.choice;
        let tip = match (self.renderers.disabled_because(), self.renderers.adapter()) {
            (Some(why), _) => why,
            (None, Some(adapter)) => format!("Graphics card: {adapter}"),
            (None, None) => "No usable graphics card: the GPU renderer is not available".to_string(),
        };
        egui::ComboBox::from_id_salt("renderer_status")
            .selected_text(format!("Renderer: {}", self.renderers.choice))
            .show_ui(ui, |ui| {
                for choice in RendererChoice::ALL {
                    ui.selectable_value(&mut self.renderers.choice, choice, choice.to_string());
                }
            })
            .response
            .on_hover_text(tip);
        if self.renderers.choice != was {
            self.status = format!("{} renderer", self.renderers.choice);
        }
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(self.tool.hint()).color(theme::ACCENT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.renderer_selector(ui);
                    if self.view.show_frame_times {
                        let line = self.frame_stats.line();
                        if !line.is_empty() {
                            ui.label(egui::RichText::new(line).monospace()).on_hover_text(self.frame_stats.breakdown());
                        }
                    }
                    if ui.small_button(format!("{:.0}%", self.camera.zoom * 100.0)).on_hover_text("Reset zoom to 100%").clicked() {
                        self.camera.zoom = 1.0;
                    }
                    if self.camera.rotation != 0.0 {
                        ui.label(format!("{:.0}°", self.camera.rotation.to_degrees()));
                    }
                    if let Some(p) = self.canvas.cursor {
                        ui.label(format!("x {:.1}  y {:.1}", p.x, p.y));
                    }
                    ui.label(egui::RichText::new(&self.status).weak());
                });
            });
        });
    }
}
