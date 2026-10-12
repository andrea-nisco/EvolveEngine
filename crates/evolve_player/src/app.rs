//! The player window: runs a game fullscreen-style, scaled to fit the window.

use crate::input::collect_input;
use eframe::egui::{self, Color32, Frame, Key, Pos2, Rect, TextureOptions};
use evolve_engine::geometry::Vec2;
use evolve_engine::render;
use evolve_engine::runtime::Runtime;
use evolve_engine::project::Project;
use evolve_engine::tiny_skia::{Color, Pixmap};

pub struct PlayerApp {
    /// `Err` holds a message to show instead of a game.
    game: Result<Runtime, String>,
    pixmap: Option<Pixmap>,
    texture: Option<egui::TextureHandle>,
    fullscreen: bool,
}

impl PlayerApp {
    /// Plays project from its start scene.
    pub fn new(project: &Project) -> Self {
        Self { game: Ok(Runtime::start(project, project.start)), pixmap: None, texture: None, fullscreen: false }
    }

    pub fn with_error(message: impl Into<String>) -> Self {
        Self { game: Err(message.into()), pixmap: None, texture: None, fullscreen: false }
    }
}

/// How the game screen sits in the window: its rectangle (egui points) and the scale from game units.
pub fn fit(game_size: Vec2, available: egui::Rect) -> (Rect, f32) {
    let scale = (available.width() / game_size.x).min(available.height() / game_size.y).max(0.01);
    (Rect::from_center_size(available.center(), egui::vec2(game_size.x * scale, game_size.y * scale)), scale)
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }

        egui::CentralPanel::default().frame(Frame::NONE.fill(Color32::BLACK)).show(ui, |ui| {
            let runtime = match &mut self.game {
                Ok(runtime) => runtime,
                Err(message) => {
                    ui.centered_and_justified(|ui| ui.colored_label(Color32::from_rgb(255, 120, 110), message.as_str()));
                    return;
                }
            };
            // The game is what the main camera sees.
            let camera = runtime.scene().camera_view();
            let (rect, scale) = fit(camera.size, ui.max_rect());

            // Pointer position in world coordinates: first inside the camera's frame, then through the camera.
            let pointer = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(Pos2::ZERO);
            let in_frame = Vec2::new((pointer.x - rect.left()) / scale, (pointer.y - rect.top()) / scale);
            let mouse = camera.view_matrix(camera.size).inverse().transform_point2(in_frame);
            let dt = ctx.input(|i| i.stable_dt);
            runtime.update(dt, collect_input(&ctx, mouse));

            // Draw exactly the game screen, in physical pixels.
            let ppp = ctx.pixels_per_point();
            let (w, h) = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
            if self.pixmap.as_ref().is_none_or(|p| p.width() != w || p.height() != h) {
                self.pixmap = Pixmap::new(w, h);
            }
            let Some(pixmap) = self.pixmap.as_mut() else { return };
            pixmap.fill(Color::TRANSPARENT);
            render::render(&runtime.scene(), camera.view_matrix(Vec2::new(w as f32, h as f32)), pixmap);

            let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("game", image, TextureOptions::LINEAR)),
            }
            if let Some(texture) = &self.texture {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
            }
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_screen_is_letterboxed_and_centered() {
        let window = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 400.0));
        let (rect, scale) = fit(Vec2::new(1280.0, 720.0), window);
        assert!((scale - 400.0 / 720.0).abs() < 1e-5, "limited by the height");
        assert!((rect.center().x - 500.0).abs() < 1e-3 && (rect.height() - 400.0).abs() < 1e-3);
        assert!(rect.width() < window.width(), "bars on the sides");
    }
}
