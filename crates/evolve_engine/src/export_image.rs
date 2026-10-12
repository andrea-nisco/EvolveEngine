//! Exporting pictures: what the camera sees as a PNG, an animation as a PNG sequence, and the scene as SVG.

use crate::animation::Clip;
use crate::geometry::{Affine2, Vec2};
use crate::path::Path;
use crate::render;
use crate::scene::{Node, NodeKind, Scene};
use crate::style::{Color, FillRule, Gradient, GradientKind, LineCap, LineJoin, Paint, Stroke};
use crate::symbol::Overrides;
use std::fmt::Write;
use tiny_skia::Pixmap;

// --- PNG ---------------------------------------------------------------------------------------

/// Renders what the camera sees at `scale` pixels per unit. Everything outside its frame is cropped.
/// With `transparent`, the camera's background color is left out.
pub fn render_artboard(scene: &Scene, scale: f32, transparent: bool) -> Option<Pixmap> {
    let scale = scale.clamp(0.05, 16.0);
    let camera = scene.camera_view();
    let (width, height) = ((camera.size.x * scale).ceil() as u32, (camera.size.y * scale).ceil() as u32);
    let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
    let view = camera.view_matrix(Vec2::new(width.max(1) as f32, height.max(1) as f32));
    if transparent {
        render::render_content(scene, view, &mut pixmap);
    } else {
        render::render(scene, view, &mut pixmap);
    }
    Some(pixmap)
}

/// The scene as it looks `time` seconds into clip `clip` (with IK solved). The input is not changed.
pub fn scene_at(scene: &Scene, clip: usize, time: f32) -> Scene {
    let mut posed = scene.clone();
    if let Some(clip) = scene.clips.get(clip) {
        let pose = clip.pose_at(scene, time);
        posed.apply_pose(&pose);
    }
    posed.solve_ik();
    posed
}

/// Number of frames a clip renders to: one per frame period, so a looping clip repeats seamlessly.
pub fn frame_count(clip: &Clip) -> usize {
    ((clip.duration * clip.fps).round() as usize).max(1)
}

// --- SVG ---------------------------------------------------------------------------------------

/// Formats a number compactly (at most 3 decimals, no trailing zeros).
fn num(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" { "0".into() } else { text.into() }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn matrix(m: &Affine2) -> String {
    let c = &m.matrix2;
    format!("matrix({} {} {} {} {} {})", num(c.x_axis.x), num(c.x_axis.y), num(c.y_axis.x), num(c.y_axis.y), num(m.translation.x), num(m.translation.y))
}

fn rgb(color: Color) -> String {
    format!("rgb({},{},{})", color.r, color.g, color.b)
}

fn path_data(path: &Path) -> String {
    let mut d = String::new();
    for sub in path.subpaths.iter().filter(|s| s.anchors.len() >= 2) {
        let start = sub.anchors[0].pos;
        let _ = write!(d, "M{} {}", num(start.x), num(start.y));
        for i in 0..sub.segment_count() {
            let s = sub.segment(i);
            if s.is_line() {
                let _ = write!(d, "L{} {}", num(s.p3.x), num(s.p3.y));
            } else {
                let _ = write!(d, "C{} {} {} {} {} {}", num(s.c1.x), num(s.c1.y), num(s.c2.x), num(s.c2.y), num(s.p3.x), num(s.p3.y));
            }
        }
        if sub.closed {
            d.push('Z');
        }
    }
    d
}

struct SvgWriter<'a> {
    scene: &'a Scene,
    defs: String,
    body: String,
    next_id: usize,
}

impl SvgWriter<'_> {
    fn fresh_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// Returns the attribute value for a paint (`rgb(...)` or `url(#id)`) and its opacity.
    fn paint(&mut self, paint: &Paint) -> (String, f32) {
        match paint {
            Paint::Solid(color) => (rgb(*color), color.a as f32 / 255.0),
            Paint::Gradient(gradient) => (format!("url(#{})", self.gradient(gradient)), 1.0),
        }
    }

    fn gradient(&mut self, gradient: &Gradient) -> String {
        let id = self.fresh_id("g");
        let transform = if gradient.transform == Affine2::IDENTITY { String::new() } else { format!(" gradientTransform=\"{}\"", matrix(&gradient.transform)) };
        let _ = match gradient.kind {
            GradientKind::Linear { start, end } => write!(
                self.defs,
                "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{transform}>",
                num(start.x), num(start.y), num(end.x), num(end.y)
            ),
            GradientKind::Radial { center, radius } => write!(
                self.defs,
                "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\"{transform}>",
                num(center.x), num(center.y), num(radius)
            ),
        };
        for stop in &gradient.stops {
            let _ = write!(
                self.defs,
                "<stop offset=\"{}\" stop-color=\"{}\" stop-opacity=\"{}\"/>",
                num(stop.offset), rgb(stop.color), num(stop.color.a as f32 / 255.0)
            );
        }
        self.defs.push_str(match gradient.kind {
            GradientKind::Linear { .. } => "</linearGradient>",
            GradientKind::Radial { .. } => "</radialGradient>",
        });
        id
    }

    /// Fill and stroke attributes of a shape, with instance color overrides applied.
    fn style(&mut self, shape: &crate::scene::Shape, overrides: &Overrides) -> String {
        let mut attrs = String::new();
        match overrides.fill_for(shape) {
            Some(paint) => {
                let (value, opacity) = self.paint(&paint);
                let _ = write!(attrs, " fill=\"{value}\"");
                if opacity < 1.0 {
                    let _ = write!(attrs, " fill-opacity=\"{}\"", num(opacity));
                }
            }
            None => attrs.push_str(" fill=\"none\""),
        }
        if shape.fill_rule == FillRule::EvenOdd {
            attrs.push_str(" fill-rule=\"evenodd\"");
        }
        if let Some(stroke) = shape.stroke.as_ref().filter(|s| s.width > 0.0) {
            let (value, opacity) = self.paint(&overrides.stroke_paint_for(stroke));
            self.stroke_attrs(&mut attrs, stroke, &value, opacity);
        }
        attrs
    }

    fn stroke_attrs(&self, attrs: &mut String, stroke: &Stroke, value: &str, opacity: f32) {
        let join = match stroke.join {
            LineJoin::Miter => "miter",
            LineJoin::Round => "round",
            LineJoin::Bevel => "bevel",
        };
        let cap = match stroke.cap {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::Square => "square",
        };
        let _ = write!(attrs, " stroke=\"{value}\" stroke-width=\"{}\" stroke-linejoin=\"{join}\" stroke-linecap=\"{cap}\"", num(stroke.width));
        if stroke.join == LineJoin::Miter {
            let _ = write!(attrs, " stroke-miterlimit=\"{}\"", num(stroke.miter_limit));
        }
        if opacity < 1.0 {
            let _ = write!(attrs, " stroke-opacity=\"{}\"", num(opacity));
        }
    }

    /// Writes `node`, whose parent's world transform is `parent_world`.
    fn node(&mut self, node: &Node, parent_world: Affine2, overrides: &Overrides) {
        if !node.visible || node.bone.is_some() {
            return;
        }
        let world = parent_world * node.transform;
        let mut attrs = String::new();
        if node.transform != Affine2::IDENTITY {
            let _ = write!(attrs, " transform=\"{}\"", matrix(&node.transform));
        }
        if node.opacity < 1.0 {
            let _ = write!(attrs, " opacity=\"{}\"", num(node.opacity));
        }
        let name = if node.name.is_empty() { String::new() } else { format!(" data-name=\"{}\"", xml_escape(&node.name)) };

        match &node.kind {
            NodeKind::Camera(_) => {}
            NodeKind::Shape(shape) => {
                let path = shape.effective_path(self.scene, world);
                let style = self.style(shape, overrides);
                let _ = writeln!(self.body, "<path d=\"{}\"{style}{attrs}{name}/>", path_data(&path));
            }
            NodeKind::Group(children) => {
                if let Some((content, mask, mask_shape)) = node.clip_parts() {
                    let id = self.fresh_id("clip");
                    let mask_world = world * mask.transform;
                    let mask_path = mask_shape.effective_path(self.scene, mask_world);
                    let _ = write!(
                        self.defs,
                        "<clipPath id=\"{id}\"><path d=\"{}\" transform=\"{}\"/></clipPath>",
                        path_data(&mask_path),
                        matrix(&mask.transform)
                    );
                    let _ = writeln!(self.body, "<g clip-path=\"url(#{id})\"{attrs}{name}>");
                    for child in content {
                        self.node(child, world, overrides);
                    }
                } else {
                    let _ = writeln!(self.body, "<g{attrs}{name}>");
                    for child in children {
                        self.node(child, world, overrides);
                    }
                }
                self.body.push_str("</g>\n");
            }
            NodeKind::Instance(instance) => {
                let Some(symbol) = self.scene.symbol(instance.symbol) else { return };
                let _ = writeln!(self.body, "<g{attrs}{name}>");
                let inner = overrides.inside(instance);
                self.node(&symbol.root, world, &inner);
                self.body.push_str("</g>\n");
            }
        }
    }
}

/// The scene as an SVG document: the camera's frame, its background rectangle, then all visible objects.
/// Bones are left out; skinned shapes are written as they currently look.
pub fn to_svg(scene: &Scene) -> String {
    let mut writer = SvgWriter { scene, defs: String::new(), body: String::new(), next_id: 0 };
    for node in &scene.nodes {
        writer.node(node, Affine2::IDENTITY, &Overrides::default());
    }
    let camera = scene.camera_view();
    let (w, h) = (num(camera.size.x), num(camera.size.y));
    // The objects are written in world coordinates; one transform puts the camera's frame at (0, 0).
    let to_frame = camera.view_matrix(camera.size);
    writer.body = format!("<g transform=\"{}\">\n{}</g>\n", matrix(&to_frame), writer.body);
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n");
    if !writer.defs.is_empty() {
        let _ = writeln!(svg, "<defs>{}</defs>", writer.defs);
    }
    let _ = writeln!(svg, "<rect width=\"{w}\" height=\"{h}\" fill=\"{}\"/>", rgb(camera.background));
    svg.push_str(&writer.body);
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Property, Value};
    use crate::scene::Shape;
    use crate::style::Paint;

    fn rect(scene: &mut Scene, name: &str, min: Vec2, max: Vec2, color: Color) -> crate::scene::NodeId {
        let mut shape = Shape::new(Path::rect(min, max));
        shape.fill = Some(Paint::Solid(color));
        shape.stroke = None;
        let node = scene.new_node(name, NodeKind::Shape(shape));
        scene.add(node, None)
    }

    #[test]
    fn png_rendering_uses_the_scale_and_can_be_transparent() {
        let mut scene = Scene::with_camera(Vec2::new(100.0, 50.0));
        rect(&mut scene, "red", Vec2::ZERO, Vec2::splat(20.0), Color::rgb(255, 0, 0));
        let opaque = render_artboard(&scene, 2.0, false).unwrap();
        assert_eq!((opaque.width(), opaque.height()), (200, 100));
        assert_eq!(opaque.pixel(100, 60).unwrap().alpha(), 255, "background fills the artboard");
        let transparent = render_artboard(&scene, 1.0, true).unwrap();
        assert_eq!(transparent.pixel(80, 40).unwrap().alpha(), 0, "no background");
        assert_eq!(transparent.pixel(5, 5).unwrap().red(), 255);
    }

    #[test]
    fn sequence_frames_follow_the_clip_without_touching_the_scene() {
        let mut scene = Scene::default();
        let id = rect(&mut scene, "box", Vec2::splat(-5.0), Vec2::splat(5.0), Color::BLACK);
        let mut clip = Clip::new("move");
        clip.duration = 2.0;
        clip.fps = 10.0;
        clip.set_key(id, Property::Position, 0.0, Value::Vec2(Vec2::ZERO), None);
        clip.set_key(id, Property::Position, 2.0, Value::Vec2(Vec2::new(100.0, 0.0)), None);
        clip.tracks[0].keys[0].easing = crate::animation::Easing::LINEAR;
        scene.clips.push(clip);
        assert_eq!(frame_count(&scene.clips[0]), 20);
        let halfway = scene_at(&scene, 0, 1.0);
        let x = halfway.world_transform(id).unwrap().translation.x;
        assert!((x - 50.0).abs() < 1.0, "halfway: {x}");
        assert_eq!(scene.world_transform(id).unwrap().translation.x, 0.0);
    }

    #[test]
    fn svg_output_reads_back_with_the_same_look() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let a = rect(&mut scene, "a & b", Vec2::ZERO, Vec2::new(40.0, 30.0), Color::rgb(10, 20, 30));
        let mut ellipse = Shape::new(Path::ellipse(Vec2::new(100.0, 50.0), Vec2::splat(20.0)));
        ellipse.fill = Some(Paint::radial(Vec2::new(100.0, 50.0), 20.0, Color::rgb(255, 0, 0), Color::rgb(0, 0, 255)));
        let node = scene.new_node("ball", NodeKind::Shape(ellipse));
        let ball = scene.add(node, None);
        let group = scene.group(&[a, ball]).unwrap();
        scene.find_mut(group).unwrap().transform = Affine2::from_translation(Vec2::new(10.0, 5.0));
        let hidden = rect(&mut scene, "hidden", Vec2::ZERO, Vec2::splat(5.0), Color::BLACK);
        scene.find_mut(hidden).unwrap().visible = false;

        let svg = to_svg(&scene);
        assert!(svg.contains("a &amp; b") && svg.contains("<radialGradient") && !svg.contains("hidden"));

        // Re-import with the SVG reader and render both: the pictures should agree.
        let mut again = Scene::with_camera(scene.camera_view().size);
        crate::io::import_svg(&mut again, svg.as_bytes(), "x").unwrap();
        let original = render_artboard(&scene, 1.0, false).unwrap();
        let copy = render_artboard(&again, 1.0, false).unwrap();
        for (x, y) in [(20, 20), (110, 55), (150, 90)] {
            let (p, q) = (original.pixel(x, y).unwrap(), copy.pixel(x, y).unwrap());
            let close = |a: u8, b: u8| a.abs_diff(b) <= 3;
            assert!(close(p.red(), q.red()) && close(p.green(), q.green()) && close(p.blue(), q.blue()), "pixel ({x}, {y}): {p:?} vs {q:?}");
        }
    }

    #[test]
    fn clip_groups_and_symbol_instances_are_exported() {
        let mut scene = Scene::default();
        let content = rect(&mut scene, "content", Vec2::ZERO, Vec2::splat(100.0), Color::rgb(255, 0, 0));
        let mask = rect(&mut scene, "mask", Vec2::ZERO, Vec2::splat(40.0), Color::BLACK);
        scene.make_clip_group(&[content, mask]).unwrap();
        let shape = rect(&mut scene, "sym", Vec2::ZERO, Vec2::splat(10.0), Color::rgb(0, 255, 0));
        let (symbol, _) = scene.make_symbol(&[shape], "S").unwrap();
        scene.place_instance(symbol, Vec2::new(300.0, 300.0));
        let svg = to_svg(&scene);
        assert!(svg.contains("<clipPath"), "{svg}");
        assert_eq!(svg.matches("rgb(0,255,0)").count(), 2, "both instances draw the symbol");
    }
}
