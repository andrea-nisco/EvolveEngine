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
        let clip_preview = self.playback_scene();
        let game = self.play.as_ref().map(|p| p.runtime.scene());
        let shown = game.as_deref().or(clip_preview.as_ref()).unwrap_or(&self.scene);
        let camera = shown.camera_view();

        // The camera's frame, as large as fits, centered (bars where the shapes differ).
        let (frame, scale) = fit(camera.size, area.shrink(4.0));
        self.canvas.game_target.paint(&painter, frame, Color32::BLACK, shown, |pixels| camera.view_matrix(pixels), TextureOptions::LINEAR);

        // Where the pointer is in the world, through the camera: this is the game's mouse.
        let pointer = response.hover_pos();
        self.canvas.game_cursor = pointer.map(|p| {
            let in_frame = Vec2::new((p.x - frame.left()) / scale, (p.y - frame.top()) / scale);
            camera.view_matrix(camera.size).inverse().transform_point2(in_frame)
        });
        drop(game);

        if self.play.is_some() {
            let view = Camera::matching(&camera, Vec2::new(frame.width(), frame.height()));
            self.paint_game_overlays(&painter, frame, &view, true);
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
