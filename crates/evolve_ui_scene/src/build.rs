//! The UI layer: the temporary scene a draw list becomes, kept between frames.

use crate::shadow;
use crate::shapes::{convert_path, rounded_rect};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::io;
use evolve_engine::render;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, Gradient, GradientKind, GradientStop, Paint, Stroke};
use evolve_engine::symbol::{Instance, SymbolId};
use evolve_engine::tiny_skia::Pixmap;
use evolve_ui_core::{DrawCmd, DrawList, Fill, FontId, ImageId, Rect, MAX_GRADIENT_STOPS};
use evolve_ui_text::TextSystem;
use std::collections::HashMap;

fn color(c: evolve_ui_core::Color) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

pub(crate) fn paint(fill: &Fill) -> Option<Paint> {
    match fill {
        Fill::Solid(c) => Some(Paint::Solid(color(*c))),
        Fill::LinearGradient { start, end, stops } => {
            let mut stops: Vec<GradientStop> = stops
                .iter()
                .filter(|(offset, _)| offset.is_finite())
                .take(MAX_GRADIENT_STOPS)
                .map(|(offset, c)| GradientStop { offset: offset.clamp(0.0, 1.0), color: color(*c) })
                .collect();
            stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            let line = [start.x, start.y, end.x, end.y];
            if stops.is_empty() || line.iter().any(|v| !v.is_finite()) {
                return None;
            }
            let kind = GradientKind::Linear { start: Vec2::new(start.x, start.y), end: Vec2::new(end.x, end.y) };
            Some(Paint::Gradient(Gradient { kind, stops, transform: Affine2::IDENTITY }))
        }
    }
}

/// The view that maps logical pixels to physical ones: `scale` pixels for each logical pixel (the screen's density times the
/// user's UI scale).
pub fn logical_view(scale: f32) -> Affine2 {
    Affine2::from_scale(Vec2::splat(scale))
}

/// A group being filled: the root of the scene, or the content of a clip that is still open.
struct Frame {
    nodes: Vec<Node>,
    clip: Option<(Rect, [f32; 4])>,
}

struct Image {
    symbol: SymbolId,
    /// The size of the drawing, in its own units.
    size: Vec2,
}

/// The scene of the UI, kept from one frame to the next. It is **not** the user's scene (ADR 0001): nothing in it is saved or
/// shown in the hierarchy. A frame whose draw list equals the last one leaves it untouched, so its change stamps stay as they were
/// and a renderer that looks at them draws nothing again; a list that differs replaces the nodes (the symbols of letters and
/// images stay, so what a renderer made of them is not made again).
pub struct UiLayer {
    scene: Scene,
    /// One symbol per glyph of a font, white; instances give it the color of the text. `None` for a glyph that draws nothing.
    glyphs: HashMap<(FontId, u16), Option<SymbolId>>,
    images: Vec<Image>,
    last: Option<DrawList>,
}

impl Default for UiLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl UiLayer {
    pub fn new() -> Self {
        Self { scene: Scene::default(), glyphs: HashMap::new(), images: Vec::new(), last: None }
    }

    /// The scene to draw, in logical pixels (y down), with [`logical_view`].
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn into_scene(self) -> Scene {
        self.scene
    }

    /// Makes an SVG known; `Image` commands name it by the id returned. The drawing is imported as vector paths.
    pub fn add_svg(&mut self, data: &[u8]) -> Result<ImageId, String> {
        let root = io::import_svg(&mut self.scene, data, "image").map_err(|e| e.to_string())?;
        let (symbol, instance) = self.scene.make_symbol(&[root], "image").ok_or("the SVG has no shapes")?;
        self.scene.remove(instance);
        let size = self.scene.symbol_bounds(symbol).map(|b| b.size()).ok_or("the SVG has no size")?;
        self.images.push(Image { symbol, size });
        Ok(ImageId(self.images.len() as u32 - 1))
    }

    /// The size of the drawing of an image, in its own units.
    pub fn image_size(&self, image: ImageId) -> Option<(f32, f32)> {
        self.images.get(image.0 as usize).map(|i| (i.size.x, i.size.y))
    }

    /// Makes the scene show `list`. Returns whether anything was done (false: the list is the one of the last call).
    pub fn update(&mut self, list: &DrawList, text: &mut TextSystem) -> bool {
        if self.last.as_ref() == Some(list) {
            return false;
        }
        let old: Vec<NodeId> = self.scene.nodes.iter().map(|n| n.id).collect();
        self.scene.remove_all(&old);
        let mut stack = vec![Frame { nodes: Vec::new(), clip: None }];
        for cmd in &list.cmds {
            self.command(text, &mut stack, cmd);
        }
        // Clips left open at the end of the list are closed, so any list gives a scene.
        while stack.len() > 1 {
            self.command(text, &mut stack, &DrawCmd::PopClip);
        }
        for node in stack.pop().expect("the root frame").nodes {
            self.scene.add(node, None);
        }
        self.last = Some(list.clone());
        true
    }

    /// Draws the layer over what `target` holds, with the CPU renderer. `scale` is physical pixels per logical pixel.
    pub fn draw_cpu(&self, target: &mut Pixmap, scale: f32) {
        render::render_content(&self.scene, logical_view(scale), target);
    }

    fn shape(&mut self, name: &str, shape: Shape, transform: Affine2) -> Node {
        let mut node = self.scene.new_node(name, NodeKind::Shape(shape));
        node.transform = transform;
        node
    }

    fn filled(path: evolve_engine::path::Path, fill: Option<Paint>, stroke: Option<Stroke>) -> Shape {
        Shape { path, fill, stroke, fill_rule: FillRule::NonZero, skin: None }
    }

    /// The symbol of one glyph, made on first use. `None` for a glyph that draws nothing.
    fn glyph_symbol(&mut self, text: &mut TextSystem, font: FontId, glyph: u16) -> Option<SymbolId> {
        if let Some(found) = self.glyphs.get(&(font, glyph)) {
            return *found;
        }
        let made = text.outline(font, glyph).map(|outline| {
            let shape = Self::filled(convert_path(&outline), Some(Paint::Solid(Color::WHITE)), None);
            let node = self.shape("glyph", shape, Affine2::IDENTITY);
            self.scene.add_symbol("glyph", vec![node])
        });
        self.glyphs.insert((font, glyph), made);
        made
    }

    fn command(&mut self, text: &mut TextSystem, stack: &mut Vec<Frame>, cmd: &DrawCmd) {
        match cmd {
            DrawCmd::RoundedRect { rect, radii, fill, border } => {
                // A border is a stroke centered on the path, so the path is pulled in by half its width to keep the
                // outside of the stroke on the rectangle's edge.
                let (path, stroke) = match border {
                    Some((width, c)) if *width > 0.0 => {
                        let half = width / 2.0;
                        let inner = Rect::new(rect.x + half, rect.y + half, (rect.w - width).max(0.0), (rect.h - width).max(0.0));
                        (rounded_rect(inner, radii.map(|r| (r - half).max(0.0))), Some(Stroke::solid(color(*c), *width)))
                    }
                    _ => (rounded_rect(*rect, *radii), None),
                };
                let node = self.shape("rect", Self::filled(path, fill.as_ref().and_then(paint), stroke), Affine2::IDENTITY);
                Self::emit(stack, node);
            }
            DrawCmd::Shadow { rect, radii, offset, blur, spread, color: shadow_color } => {
                for layer in shadow::layers(*rect, *radii, *offset, *blur, *spread) {
                    let alpha = (f32::from(shadow_color.a) * layer.alpha).round() as u8;
                    if alpha == 0 {
                        continue;
                    }
                    let c = Color::rgba(shadow_color.r, shadow_color.g, shadow_color.b, alpha);
                    let shape = Self::filled(rounded_rect(layer.rect, layer.radii), Some(Paint::Solid(c)), None);
                    let node = self.shape("shadow", shape, Affine2::IDENTITY);
                    Self::emit(stack, node);
                }
            }
            DrawCmd::Image { rect, image, tint } => {
                let Some(found) = self.images.get(image.0 as usize) else { return };
                let (symbol, size) = (found.symbol, found.size);
                if !(size.x > 0.0 && size.y > 0.0 && rect.w > 0.0 && rect.h > 0.0) {
                    return;
                }
                let fit = (rect.w / size.x).min(rect.h / size.y);
                // The symbol's artwork is centered on its own origin.
                let center = Vec2::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                let tint = tint.map(color);
                let mut node = self.scene.new_node("image", NodeKind::Instance(Instance { symbol, fill: tint, stroke: tint }));
                node.transform = Affine2::from_scale_angle_translation(Vec2::splat(fit), 0.0, center);
                Self::emit(stack, node);
            }
            DrawCmd::Path { origin, path, fill } => {
                let at = Affine2::from_translation(Vec2::new(origin.x, origin.y));
                let node = self.shape("path", Self::filled(convert_path(path), fill.as_ref().and_then(paint), None), at);
                Self::emit(stack, node);
            }
            DrawCmd::Text { origin, run } => {
                let ink = color(run.color);
                for glyph in &run.glyphs {
                    let Some(symbol) = self.glyph_symbol(text, run.font, glyph.id) else { continue };
                    let at = Vec2::new(origin.x + glyph.x, origin.y + glyph.y);
                    let mut node = self.scene.new_node("", NodeKind::Instance(Instance { symbol, fill: Some(ink), stroke: None }));
                    node.transform = Affine2::from_scale_angle_translation(Vec2::splat(run.size), 0.0, at);
                    Self::emit(stack, node);
                }
            }
            DrawCmd::PushClip { rect, radii } => stack.push(Frame { nodes: Vec::new(), clip: Some((*rect, *radii)) }),
            DrawCmd::PopClip => {
                // An unbalanced list must not crash the UI: a pop with nothing to close is ignored.
                if stack.len() > 1 {
                    let Frame { mut nodes, clip } = stack.pop().expect("checked above");
                    let (rect, radii) = clip.expect("only clip frames are above the root");
                    // The mask is the top child; a clip group with no content draws nothing, so it is left out.
                    if !nodes.is_empty() {
                        let mask = self.shape("clip", Self::filled(rounded_rect(rect, radii), Some(Paint::Solid(Color::WHITE)), None), Affine2::IDENTITY);
                        nodes.push(mask);
                        let mut group = self.scene.new_node("clip", NodeKind::Group(nodes));
                        group.clip = true;
                        Self::emit(stack, group);
                    }
                }
            }
        }
    }

    fn emit(stack: &mut [Frame], node: Node) {
        stack.last_mut().expect("the root frame is always there").nodes.push(node);
    }
}

/// The temporary scene for `list`, in logical pixels (y down), made from nothing. For a UI that is drawn every frame keep a
/// [`UiLayer`] instead.
pub fn build_scene(list: &DrawList, text: &mut TextSystem) -> Scene {
    let mut layer = UiLayer::new();
    layer.update(list, text);
    layer.into_scene()
}

/// Draws `list` over what is already in `target` with the CPU renderer. `scale` is the physical pixels per logical pixel.
pub fn render_cpu(list: &DrawList, text: &mut TextSystem, scale: f32, target: &mut Pixmap) {
    let mut layer = UiLayer::new();
    layer.update(list, text);
    layer.draw_cpu(target, scale);
}
