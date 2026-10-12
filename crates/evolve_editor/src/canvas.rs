//! The viewport: renders the scene with the engine, handles camera navigation
//! and forwards pointer input to the active tool.

use crate::app::EditorApp;
use crate::theme;
use crate::tools::{CanvasInput, Tool};
use eframe::egui::{self, Key, PointerButton, Pos2, Rect, Sense, TextureOptions};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::render;
use evolve_engine::scene::Scene;
use evolve_engine::symbol::SymbolId;
use evolve_engine::tiny_skia::{Color, Pixmap};

fn to_vec2(v: egui::Vec2) -> Vec2 {
    Vec2::new(v.x, v.y)
}

impl EditorApp {
    pub fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        self.camera.viewport = to_vec2(rect.size());
        if self.canvas.needs_frame {
            self.frame_camera();
            self.canvas.needs_frame = false;
        }

        if let (Some(symbol), Some(pointer)) = (response.dnd_release_payload::<SymbolId>(), ui.input(|i| i.pointer.latest_pos())) {
            let at = self.camera.screen_to_world(to_vec2(pointer - rect.min));
            self.place_symbol(*symbol, at);
        }
        self.navigate(ui, &response);
        let input = self.read_input(ui, &response, rect);
        self.canvas.cursor = input.pos;
        let game_running = self.play.is_some();
        let clip_playing = self.anim.playing;
        if !game_running && !clip_playing && !ui.input(|i| i.key_down(Key::Space)) {
            self.run_tool(&input);
        }
        self.set_cursor(ui, &input, &response);

        // What to draw: the running game, an animation preview, or the scene being edited.
        let clip_preview = self.playback_scene();
        let game = self.play.as_ref().map(|p| p.runtime.scene());
        let shown = game.as_deref().or(clip_preview.as_ref()).unwrap_or(&self.scene);
        let ppp = ui.ctx().pixels_per_point();
        let view = self.camera.view_matrix();
        self.canvas.scene_target.paint(
            &painter,
            rect,
            theme::WORKSPACE,
            shown,
            |_| Affine2::from_scale(Vec2::splat(ppp)) * view,
            TextureOptions::NEAREST,
        );
        drop(game);

        if game_running {
            self.paint_game_overlays(&painter, rect, &self.camera, false);
        } else if !clip_playing {
            self.paint_overlays(&painter, rect, &input);
        }
    }

    /// Mouse wheel zooms at the cursor, middle drag (or Space + drag) pans, right drag rotates the view.
    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let rect = response.rect;
        let cursor = ui.input(|i| i.pointer.latest_pos()).map_or(self.camera.viewport * 0.5, |p| to_vec2(p - rect.min));
        let space = ui.input(|i| i.key_down(Key::Space));
        let delta = to_vec2(response.drag_delta());

        if response.dragged_by(PointerButton::Middle) || (space && response.dragged_by(PointerButton::Primary)) {
            self.camera.pan_by_screen(delta);
        }
        if response.dragged_by(PointerButton::Secondary) {
            self.camera.rotate_at(cursor, delta.x * 0.005);
        }
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.0025).exp() * pinch;
            if (factor - 1.0).abs() > 1e-4 {
                self.camera.zoom_at(cursor, factor);
            }
        }
    }

    fn read_input(&self, ui: &egui::Ui, response: &egui::Response, rect: Rect) -> CanvasInput {
        let to_screen = |p: Pos2| to_vec2(p - rect.min);
        let (latest, origin, mods, down) =
            ui.input(|i| (i.pointer.latest_pos(), i.pointer.press_origin(), i.modifiers, i.pointer.any_down()));
        let latest = latest.filter(|p| rect.contains(*p) || down);
        let screen = latest.map(to_screen).unwrap_or_default();
        let screen_origin = origin.map(to_screen).unwrap_or(screen);
        let primary = PointerButton::Primary;
        CanvasInput {
            pos: latest.map(|_| self.camera.screen_to_world(screen)),
            origin: self.camera.screen_to_world(screen_origin),
            screen,
            screen_origin,
            drag_started: response.drag_started_by(primary),
            dragging: response.dragged_by(primary),
            drag_stopped: response.drag_stopped_by(primary),
            clicked: response.clicked_by(primary),
            double_clicked: response.double_clicked_by(primary),
            shift: mods.shift,
            alt: mods.alt,
            tolerance: 5.0 / self.camera.zoom,
        }
    }

    fn run_tool(&mut self, input: &CanvasInput) {
        match self.tool {
            Tool::Select => self.select_tool(input),
            Tool::Node => self.node_tool(input),
            Tool::Pen => self.pen_tool(input),
            Tool::Rectangle => self.shape_tool(input, false),
            Tool::Ellipse => self.shape_tool(input, true),
            Tool::Pencil => self.pencil_tool(input),
            Tool::Bone => self.bone_tool(input),
            Tool::Weights => self.weights_tool(input),
        }
    }

    fn set_cursor(&self, ui: &egui::Ui, input: &CanvasInput, response: &egui::Response) {
        if !response.hovered() && !response.dragged() {
            return;
        }
        let icon = match self.tool {
            Tool::Select => self.handle_cursor(input.screen),
            Tool::Node => egui::CursorIcon::Default,
            _ => egui::CursorIcon::Crosshair,
        };
        if icon != egui::CursorIcon::Default {
            ui.ctx().set_cursor_icon(icon);
        }
    }
}

/// A picture the scene is rasterized into, and the texture that shows it. Reused between frames.
#[derive(Default)]
pub struct RenderTarget {
    pixmap: Option<Pixmap>,
    texture: Option<egui::TextureHandle>,
}

impl RenderTarget {
    /// Rasterizes `scene` at physical-pixel resolution and paints it into `rect` (egui points).
    /// `view` turns the picture's size in pixels into the transform from world space to pixels.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        background: egui::Color32,
        scene: &Scene,
        view: impl Fn(Vec2) -> Affine2,
        filter: TextureOptions,
    ) {
        let ctx = painter.ctx();
        let ppp = ctx.pixels_per_point();
        let (w, h) = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);

        if self.pixmap.as_ref().is_none_or(|p| p.width() != w || p.height() != h) {
            self.pixmap = Pixmap::new(w, h);
        }
        let Some(pixmap) = self.pixmap.as_mut() else { return };

        let [r, g, b, _] = background.to_array();
        pixmap.fill(Color::from_rgba8(r, g, b, 255));
        render::render(scene, view(Vec2::new(w as f32, h as f32)), pixmap);

        let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
        match &mut self.texture {
            Some(texture) => texture.set(image, filter),
            None => self.texture = Some(ctx.load_texture("view", image, filter)),
        }
        if let Some(texture) = &self.texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
        }
    }
}