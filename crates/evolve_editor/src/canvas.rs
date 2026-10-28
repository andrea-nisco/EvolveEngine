//! The viewport: renders the scene with the engine, handles camera navigation
//! and forwards pointer input to the active tool.

use crate::app::EditorApp;
use crate::renderers::{GpuOutcome, GpuTarget, RendererChoice, RendererSettings};
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
        let clip_preview = self.anim.preview.clone();
        let game = self.play.as_ref().map(|p| p.runtime.scene());
        let shown = game.as_deref().or(clip_preview.as_deref()).unwrap_or(&self.scene);
        let ppp = ui.ctx().pixels_per_point();
        let view = self.camera.view_matrix();
        let drawing = std::time::Instant::now();
        self.canvas.scene_target.paint(
            &painter,
            rect,
            theme::WORKSPACE,
            shown,
            |_| Affine2::from_scale(Vec2::splat(ppp)) * view,
            TextureOptions::NEAREST,
            &self.renderers,
            "Scene view",
        );
        drop(game);
        self.frame_stats.add("Scene view: drawing the scene", drawing.elapsed());

        let overlays = std::time::Instant::now();
        if game_running {
            self.paint_game_overlays(&painter, rect, &self.camera, false);
        } else if !clip_playing {
            self.paint_overlays(&painter, rect, &input);
        }
        self.frame_stats.add("Scene view: overlays", overlays.elapsed());
    }

    /// Mouse wheel zooms at the cursor, a trackpad's two-finger scroll pans and its pinch zooms, middle drag
    /// (or Space + drag) pans, right drag rotates the view (and pans it while a game runs).
    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let rect = response.rect;
        let cursor = ui.input(|i| i.pointer.latest_pos()).map_or(self.camera.viewport * 0.5, |p| to_vec2(p - rect.min));
        let space = ui.input(|i| i.key_down(Key::Space));
        let delta = to_vec2(response.drag_delta());

        if response.dragged_by(PointerButton::Middle) || (space && response.dragged_by(PointerButton::Primary)) {
            self.camera.pan_by_screen(delta);
        }
        if response.dragged_by(PointerButton::Secondary) {
            // Rotating the view is of no use while a game runs, and the right button is the easy way to look around the map.
            if self.play.is_some() {
                self.camera.pan_by_screen(delta);
            } else {
                self.camera.rotate_at(cursor, delta.x * 0.005);
            }
        }
        if response.hovered() {
            let units = ui.input(|i| {
                i.events.iter().filter_map(|e| if let egui::Event::MouseWheel { unit, .. } = e { Some(*unit) } else { None }).collect::<Vec<_>>()
            });
            self.canvas.trackpad_scroll = is_trackpad_scroll(self.canvas.trackpad_scroll, &units);
            // Pinch and Cmd / Ctrl + scroll arrive as zoom_delta; plain scrolling as smooth_scroll_delta.
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            let mut factor = pinch;
            if self.canvas.trackpad_scroll {
                self.camera.pan_by_screen(to_vec2(scroll));
            } else {
                factor *= (scroll.y * 0.0025).exp();
            }
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

/// The camera's frame filled with its background color, as the GPU renderer's picture is shown over it: what the card would draw
/// first, without its being part of the scene it draws.
pub(crate) fn paint_camera_frame(painter: &egui::Painter, rect: Rect, scene: &Scene, view: &dyn Fn(Vec2) -> Affine2) {
    let ppp = painter.ctx().pixels_per_point();
    let pixels = Vec2::new((rect.width() * ppp).round(), (rect.height() * ppp).round());
    if pixels.x < 1.0 || pixels.y < 1.0 {
        return;
    }
    let camera = scene.camera_view();
    let to_pixels = view(pixels);
    let corners = camera.corners().map(|c| {
        let p = to_pixels.transform_point2(c);
        rect.min + egui::vec2(p.x, p.y) / ppp
    });
    let color = camera.background;
    let fill = egui::Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a);
    painter.add(egui::Shape::convex_polygon(corners.to_vec(), fill, egui::Stroke::NONE));
}

/// A view of the scene: the CPU renderer's bitmap and the texture that shows it, or the GPU renderer's picture. Reused
/// between frames.
#[derive(Default)]
pub struct RenderTarget {
    pixmap: Option<Pixmap>,
    texture: Option<egui::TextureHandle>,
    gpu: Option<GpuTarget>,
    /// Why this view is drawn by the CPU renderer although the GPU renderer was asked for.
    notice: Option<String>,
}

impl RenderTarget {
    /// The notice shown in the view when the CPU renderer is used in place of the GPU renderer.
    #[cfg(test)]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// How many times the GPU renderer's feature gate looked through the scene for this view.
    #[cfg(test)]
    pub fn gpu_gates(&self) -> Option<u64> {
        self.gpu.as_ref().map(GpuTarget::gates_made)
    }

    /// How many times the GPU renderer really drew this view, and how many paints reused an unchanged picture.
    #[cfg(test)]
    pub fn gpu_counts(&self) -> Option<(u64, u64)> {
        self.gpu.as_ref().map(GpuTarget::counts)
    }

    /// Draws `scene` and paints it into `rect` (egui points), with the renderer that `renderers` chooses: the GPU renderer
    /// when it is asked for and can draw the scene, the CPU renderer otherwise (with a notice when the GPU renderer was
    /// the wish). `view` turns the picture's size in pixels into the transform from world space to pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        background: egui::Color32,
        scene: &Scene,
        view: impl Fn(Vec2) -> Affine2,
        filter: TextureOptions,
        renderers: &RendererSettings,
        name: &'static str,
    ) {
        self.notice = None;
        if let Some(context) = renderers.usable_context() {
            if self.gpu.is_none() {
                match GpuTarget::new(context, name) {
                    Ok(target) => self.gpu = Some(target),
                    Err(why) => self.notice = Some(why),
                }
            }
            if let Some(target) = &mut self.gpu {
                painter.rect_filled(rect, 0.0, background);
                paint_camera_frame(painter, rect, scene, &view);
                match target.paint(painter, rect, scene, &view) {
                    GpuOutcome::Shown | GpuOutcome::Nothing => return,
                    GpuOutcome::Fallback(why) => self.notice = Some(why),
                }
            }
        } else if renderers.choice != RendererChoice::Cpu {
            self.notice = match (renderers.disabled_because(), renderers.adapter()) {
                (Some(why), _) => Some(why.to_string()),
                (None, None) => Some("CPU renderer: no usable graphics card".to_string()),
                (None, Some(_)) => None,
            };
        }
        self.paint_cpu(painter, rect, background, scene, view, filter);
        if let Some(notice) = &self.notice {
            let font = egui::FontId::proportional(11.0);
            let galley = painter.layout_no_wrap(notice.clone(), font, egui::Color32::from_gray(210));
            let at = rect.left_bottom() + egui::vec2(6.0, -6.0 - galley.size().y);
            painter.rect_filled(egui::Rect::from_min_size(at - egui::vec2(4.0, 2.0), galley.size() + egui::vec2(8.0, 4.0)), 3.0, egui::Color32::from_black_alpha(150));
            painter.galley(at, galley, egui::Color32::WHITE);
        }
    }

    /// The CPU renderer: rasterizes `scene` at physical-pixel resolution and paints the bitmap into `rect`.
    fn paint_cpu(&mut self, painter: &egui::Painter, rect: Rect, background: egui::Color32, scene: &Scene, view: impl Fn(Vec2) -> Affine2, filter: TextureOptions) {
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

/// Whether scrolling comes from a trackpad: those report pixel-precise deltas, a mouse wheel reports lines.
/// The device that sent the most recent wheel event decides (trackpads go on sending events for a moment
/// after the fingers lift, as kinetic scrolling).
fn is_trackpad_scroll(previous: bool, units: &[egui::MouseWheelUnit]) -> bool {
    units.last().map_or(previous, |unit| *unit == egui::MouseWheelUnit::Point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::MouseWheelUnit::{Line, Point};

    #[test]
    fn the_latest_wheel_event_decides_between_trackpad_and_mouse_wheel() {
        assert!(is_trackpad_scroll(false, &[Point]));
        assert!(!is_trackpad_scroll(true, &[Line]));
        assert!(!is_trackpad_scroll(false, &[Point, Line]), "a wheel notch right after trackpad events is the wheel");
        assert!(is_trackpad_scroll(true, &[]), "no events this frame: the smoothing tail keeps its meaning");
        assert!(!is_trackpad_scroll(false, &[]));
    }
}
