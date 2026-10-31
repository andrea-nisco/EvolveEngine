//! The Game view: exactly what the player sees through the game camera, at the camera's aspect
//! ratio. It shows a live preview while editing and the running game in Play mode.

use crate::app::EditorApp;
use crate::theme::ACCENT;
use eframe::egui::{self, Color32, Sense, TextureOptions};
use evolve_engine::camera::Camera;
use evolve_engine::geometry::Vec2;
use evolve_player_core::app::fit;

impl EditorApp {
    pub fn game_pane(&mut self, ui: &mut egui::Ui) {
        self.game_toolbar(ui);

        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let area = response.rect;
        painter.rect_filled(area, 0.0, Color32::from_gray(10));

        // What is shown: the running game, an animation preview, or the scene being edited.
        let clip_preview = self.anim.preview.clone();
        let game = self.play.as_ref().map(|p| p.runtime.scene());
        let shown = game.as_deref().or(clip_preview.as_deref()).unwrap_or(&self.scene);
        let camera = shown.camera_view();

        // The camera's frame, as large as fits, centered (bars where the shapes differ).
        let (frame, scale) = fit(camera.size, area.shrink(4.0));
        let drawing = std::time::Instant::now();
        self.canvas.game_target.paint(&painter, frame, Color32::BLACK, shown, |pixels| camera.view_matrix(pixels), TextureOptions::LINEAR, &self.renderers, "Game view");
        // The game's UI goes over the picture of the game, drawn last (ADR 0001).
        if self.view.ui_demo {
            let ppp = ui.ctx().pixels_per_point();
            let (mode, user_scale) = (self.view.ui_mode, self.view.ui_scale);
            let metrics = move |pixels: (f32, f32)| evolve_ui_core::ScreenMetrics::new(pixels).with_mode(mode).with_user_scale(user_scale);
            let pixels = (frame.width() * ppp, frame.height() * ppp);
            // A notch at the top and a bar at the bottom, as large as a phone's (about 6% and 4% of the height).
            let safe_area = if self.view.ui_safe_area { evolve_ui_core::Insets::new(0.0, pixels.1 * 0.06, 0.0, pixels.1 * 0.04) } else { evolve_ui_core::Insets::ZERO };
            let screen = metrics(pixels).with_safe_area(safe_area);
            let demo = self.canvas.ui_demo.get_or_insert_with(evolve_ui_scene::demo::UiDemo::new);
            let layer = evolve_ui_scene::UiSource::frame(demo, &screen);
            self.canvas.ui_target.paint_overlay(&painter, frame, layer, |px| evolve_ui_scene::logical_view(metrics((px.x, px.y)).scale()), &self.renderers, "UI overlay");
        }

        // Where the pointer is in the world, through the camera: this is the game's mouse. A drag with the right button that began over
        // the picture goes on being followed outside it (the game would otherwise see the mouse jump away).
        // (`hover_pos` is empty while a button is held on something else; `contains_pointer` says where the pointer is regardless.)
        let hovering = if response.contains_pointer() { ui.input(|i| i.pointer.latest_pos()) } else { None };
        let (pressed, held) = ui.input(|i| (i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.button_down(egui::PointerButton::Secondary)));
        if hovering.is_some() && pressed {
            self.canvas.game_drag = true;
        }
        if !held {
            self.canvas.game_drag = false;
        }
        let pointer = hovering.or_else(|| if self.canvas.game_drag { ui.input(|i| i.pointer.latest_pos()) } else { None });
        self.canvas.game_cursor = pointer.map(|p| {
            let in_frame = Vec2::new((p.x - frame.left()) / scale, (p.y - frame.top()) / scale);
            camera.view_matrix(camera.size).inverse().transform_point2(in_frame)
        });
        drop(game);
        self.frame_stats.add("Game view: drawing the scene", drawing.elapsed());

        if self.play.is_some() {
            let overlays = std::time::Instant::now();
            let view = Camera::matching(&camera, Vec2::new(frame.width(), frame.height()));
            self.paint_game_overlays(&painter, frame, &view, true);
            self.frame_stats.add("Game view: overlays", overlays.elapsed());
        }
    }

    /// Play controls above the picture.
    fn game_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let running = self.play.is_some();
            let play = egui::Button::new(if running { "Stop" } else { "Play" }).selected(running).min_size(egui::vec2(56.0, 0.0));
            if ui.add(play).on_hover_text("Run the game from this scene (F5)").clicked() {
                self.toggle_game();
            }
            if self.project.scenes.len() > 1
                && ui.add_enabled(!running, egui::Button::new("From start")).on_hover_text("Run the game from its start scene (Shift+F5)").clicked()
            {
                self.start_game_from_start();
            }
            let paused = self.play.as_ref().is_some_and(|p| p.paused);
            if ui.add_enabled(running, egui::Button::new(if paused { "Resume" } else { "Pause" })).on_hover_text("F6").clicked() {
                self.toggle_game_pause();
            }
            let size = self.scene.camera_view().size;
            let running_scene = self.play.as_ref().map(|p| p.runtime.scene_name());
            if let Some(name) = running_scene {
                ui.colored_label(ACCENT, name);
            }
            ui.weak(format!("{} x {}", size.x.round(), size.y.round()));
        });
        ui.separator();
    }
}
