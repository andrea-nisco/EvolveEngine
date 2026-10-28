//! Play mode: runs the scene as a game inside the viewport, with debug overlays, a console,
//! and the Physics and Script sections of the properties panel.

use crate::app::EditorApp;
use crate::layout::Pane;
use crate::overlay::Screen;
use crate::theme::ACCENT;
use eframe::egui::{self, Color32, Painter, Rect, Stroke};
use evolve_engine::camera::Camera;
use evolve_engine::physics::{Body, BodyKind};
use evolve_engine::runtime::{LogLine, Runtime};
use evolve_player_core::input::collect_input_over;
use evolve_engine::scene::{Node, NodeKind};
use evolve_engine::script::{check_script, SCRIPT_TEMPLATE};

pub struct PlayState {
    pub runtime: Runtime,
    pub paused: bool,
    /// Smoothed frames per second.
    pub fps: f32,
}

/// What the debug overlays show while the game runs.
pub struct GameView {
    pub colliders: bool,
    pub anchors: bool,
    pub stats: bool,
}

impl Default for GameView {
    fn default() -> Self {
        Self { colliders: true, anchors: false, stats: true }
    }
}

impl EditorApp {
    // --- Running -------------------------------------------------------------------------------

    /// Plays the scene being edited (and the scenes after it, if scripts go on to them).
    pub fn start_game(&mut self) {
        self.start_game_at(self.open);
    }

    /// Plays from the project's start scene, as the exported game does.
    pub fn start_game_from_start(&mut self) {
        self.start_game_at(self.project.start);
    }

    fn start_game_at(&mut self, scene: usize) {
        self.sync_open_scene();
        self.finish_pen(false);
        self.canvas.drag = crate::tools::Drag::None;
        self.anim.playing = false;
        let runtime = Runtime::start(&self.project, scene);
        let has_scripts = runtime.stats().scripts > 0 || runtime.log().iter().any(|l| l.error);
        self.play = Some(PlayState { runtime, paused: false, fps: 60.0 });
        self.show_pane(Pane::Game);
        if has_scripts {
            self.show_pane(Pane::Console);
        }
        self.status = "Game running. F5 stops, F6 pauses".into();
    }

    pub fn stop_game(&mut self) {
        if let Some(play) = self.play.take() {
            self.last_log = play.runtime.log().clone();
            self.status = "Game stopped".into();
        }
    }

    pub fn toggle_game(&mut self) {
        if self.play.is_some() {
            self.stop_game();
        } else {
            self.start_game();
        }
    }

    pub fn toggle_game_pause(&mut self) {
        if let Some(play) = &mut self.play {
            play.paused = !play.paused;
        }
    }

    /// Feeds the keyboard and pointer to the running game and advances it.
    pub fn update_game(&mut self, ctx: &egui::Context) {
        // The game's mouse is the pointer over the Game view (or over the Scene view when the Game view is hidden).
        let mouse = self.canvas.game_cursor.or(self.canvas.cursor).unwrap_or_default();
        // The wheel and a right button that goes down belong to the game only over the Game view (the Scene view keeps them for moving
        // its own camera).
        let over_game = self.canvas.game_cursor.is_some();
        let Some(play) = &mut self.play else { return };
        ctx.request_repaint();
        let dt = ctx.input(|i| i.stable_dt);
        play.fps = play.fps * 0.9 + (1.0 / dt.max(1e-4)) * 0.1;
        if play.paused {
            return;
        }
        play.runtime.update(dt, collect_input_over(ctx, mouse, over_game));
    }

    // --- Overlays ------------------------------------------------------------------------------

    /// Collision shapes and anchors (and, in the Game view, statistics) drawn over the running game.
    /// `camera` maps the world onto `rect`.
    pub fn paint_game_overlays(&self, painter: &Painter, rect: Rect, camera: &Camera, stats: bool) {
        let Some(play) = &self.play else { return };
        let painter = painter.with_clip_rect(rect);
        let screen = Screen { camera, origin: rect.min };

        if self.game_view.colliders {
            for body in play.runtime.debug_bodies() {
                let color = match body.kind {
                    BodyKind::Static => Color32::from_rgb(90, 220, 120),
                    BodyKind::Trigger => Color32::from_rgb(255, 210, 70),
                    BodyKind::Dynamic => Color32::from_rgb(80, 200, 255),
                };
                for outline in &body.outlines {
                    painter.add(crate::overlay::outline_shape(&screen.pts(&outline.points), outline.closed, Stroke::new(1.5, color)));
                }
                if let Some((center, radius)) = body.circle {
                    painter.circle_stroke(screen.pt(center), radius * camera.zoom, Stroke::new(1.5, color));
                }
            }
        }
        if self.game_view.anchors {
            let scene = play.runtime.scene();
            for id in scene.all_ids() {
                let (Some(Node { kind: NodeKind::Shape(shape), .. }), Some(world)) = (scene.find(id), scene.world_transform(id)) else { continue };
                for anchor in shape.effective_path(&scene, world).transformed(&world).subpaths.iter().flat_map(|s| &s.anchors) {
                    painter.circle_filled(screen.pt(anchor.pos), 2.5, ACCENT);
                }
            }
        }
        if stats && self.game_view.stats {
            let stats = play.runtime.stats();
            let text = format!(
                "{}   {:.0} fps   {} shapes drawn   {} bodies   {} scripts{}",
                play.runtime.scene_name(),
                play.fps,
                stats.shapes,
                stats.bodies,
                stats.scripts,
                if play.paused { "   PAUSED" } else { "" }
            );
            painter.text(rect.min + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, text, egui::FontId::monospace(12.0), Color32::WHITE);
        }
    }

    // --- Console tab ---------------------------------------------------------------------------

    pub fn console_contents(&mut self, ui: &mut egui::Ui) {
        let running = self.play.is_some();
        ui.horizontal(|ui| {
            ui.label(if running { "Game running" } else { "Game stopped" });
            if ui.add_enabled(!running, egui::Button::new("Clear")).clicked() {
                self.last_log.clear();
            }
        });
        ui.separator();
        let lines: Vec<LogLine> = match &self.play {
            Some(play) => play.runtime.log().clone(),
            None => self.last_log.clone(),
        };
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if lines.is_empty() {
                ui.weak("Output of print() and script errors appear here while the game runs (F5).");
            }
            for line in &lines {
                let text = format!("[{:>6.2}s] {}", line.time, line.text);
                let text = egui::RichText::new(text).monospace();
                ui.label(if line.error { text.color(Color32::from_rgb(255, 110, 100)) } else { text });
            }
        });
    }

    // --- Properties panel ----------------------------------------------------------------------

    pub fn game_properties(&mut self, ui: &mut egui::Ui) {
        self.physics_section(ui);
        self.script_section(ui);
    }

    fn physics_section(&mut self, ui: &mut egui::Ui) {
        let [id] = self.selection[..] else { return };
        // Edit a copy and write it back only when it changed: the panel runs every frame, and writing to the node
        // every frame would count as a change every frame.
        let Some(original) = self.scene.find(id).filter(|n| n.bone.is_none()).map(|n| n.body) else { return };
        ui.separator();
        ui.strong("Physics");
        let mut body = original;
        let mut enabled = body.is_some();
        if ui.checkbox(&mut enabled, "Collision body").on_hover_text("Lets this object collide in the game").changed() {
            body = enabled.then(|| Body::new(BodyKind::Static));
        }
        if let Some(body) = &mut body {
            egui::ComboBox::from_id_salt("body_kind")
                .selected_text(match body.kind {
                    BodyKind::Static => "Static (solid, fixed)",
                    BodyKind::Dynamic => "Dynamic (a moving circle)",
                    BodyKind::Trigger => "Trigger (reports overlaps)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut body.kind, BodyKind::Static, "Static (solid, fixed)");
                    ui.selectable_value(&mut body.kind, BodyKind::Dynamic, "Dynamic (a moving circle)");
                    ui.selectable_value(&mut body.kind, BodyKind::Trigger, "Trigger (reports overlaps)");
                });
            if body.kind == BodyKind::Dynamic {
                ui.add(egui::Slider::new(&mut body.restitution, 0.0..=1.0).text("Bounce"));
                ui.add(egui::Slider::new(&mut body.friction, 0.0..=1.0).text("Friction"));
                ui.add(egui::Slider::new(&mut body.gravity_scale, -2.0..=3.0).text("Gravity"));
            }
        }
        if body != original {
            if let Some(mut node) = self.scene.find_mut(id) {
                node.set_body(body);
            }
        }
    }

    fn script_section(&mut self, ui: &mut egui::Ui) {
        let [id] = self.selection[..] else { return };
        let Some(original) = self.scene.find(id).filter(|n| n.bone.is_none()).map(|n| n.script.clone()) else { return };
        ui.separator();
        ui.strong("Script");
        let mut script = original.clone();
        match &mut script {
            None => {
                if ui.button("Add script").clicked() {
                    script = Some(SCRIPT_TEMPLATE.to_string());
                }
            }
            Some(source) => {
                ui.add(egui::TextEdit::multiline(source).code_editor().desired_rows(10).desired_width(f32::INFINITY));
                match check_script(source) {
                    Ok(()) => {
                        ui.weak("Handlers: on_init(me), on_update(me, dt), on_input(me, key), on_collision(me, other), on_trigger(me, other, entered)");
                    }
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(255, 110, 100), error);
                    }
                }
                if ui.button("Remove script").clicked() {
                    script = None;
                }
            }
        }
        if script != original {
            if let Some(mut node) = self.scene.find_mut(id) {
                node.set_script(script);
            }
        }
    }
}
