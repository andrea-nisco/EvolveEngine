//! CPU vector renderer built on tiny-skia (analytic anti-aliasing, exact strokes, gradients).
//!
//! Everything is rasterized straight from the Bézier data at the requested resolution,
//! so output stays sharp at any zoom level.

use crate::geometry::{Affine2, Rect, Vec2};
use crate::path::Path;
use crate::scene::{Node, NodeKind, Scene, Shape};
use crate::style::{self, Color, FillRule, GradientKind, LineCap, LineJoin, Paint};
use crate::symbol::{Overrides, Symbol};
use tiny_skia as sk;

/// Draws the camera's background inside its frame, then all visible nodes, into `target`.
/// `view` maps world space to pixels; the caller clears `target` beforehand.
pub fn render(scene: &Scene, view: Affine2, target: &mut sk::Pixmap) {
    let camera = scene.camera_view();
    if let Some(path) = to_skia_path(&camera.outline()) {
        let mut paint = sk::Paint::default();
        paint.set_color(to_skia_color(camera.background));
        target.fill_path(&path, &paint, sk::FillRule::Winding, to_skia_transform(&view), None);
    }
    render_content(scene, view, target);
}

/// Draws the visible nodes only, without the camera's background color.
pub fn render_content(scene: &Scene, view: Affine2, target: &mut sk::Pixmap) {
    render_nodes(scene, view, target, true);
}

/// Draws the visible nodes. With `cull` off, shapes outside the target are processed anyway
/// (the picture is the same; this exists so tests and benchmarks can compare).
pub fn render_nodes(scene: &Scene, view: Affine2, target: &mut sk::Pixmap, cull: bool) {
    let ctx = Ctx { scene, overrides: Overrides::default(), view, cull };
    for node in &scene.nodes {
        draw_node(&ctx, node, view, target);
    }
}

/// Renders a symbol's master artwork, fitted into a transparent `size` x `size` square.
pub fn thumbnail(scene: &Scene, symbol: &Symbol, size: u32) -> Option<sk::Pixmap> {
    let bounds = scene.symbol_bounds(symbol.id)?;
    let extent = bounds.size().max_element().max(1e-3);
    let scale = size as f32 * 0.85 / extent;
    let view = Affine2::from_translation(crate::geometry::Vec2::splat(size as f32 * 0.5))
        * Affine2::from_scale(crate::geometry::Vec2::splat(scale))
        * Affine2::from_translation(-bounds.center());
    let mut pixmap = sk::Pixmap::new(size, size)?;
    draw_node(&Ctx { scene, overrides: Overrides::default(), view, cull: true }, &symbol.root, view, &mut pixmap);
    Some(pixmap)
}

/// What is needed while drawing: the scene (to resolve symbols and bones), the active
/// color overrides, and the view transform (world to pixels).
#[derive(Clone, Copy)]
struct Ctx<'a> {
    scene: &'a Scene,
    overrides: Overrides,
    view: Affine2,
    /// Skip shapes (and layers) that cannot reach the target.
    cull: bool,
}

impl Ctx<'_> {
    /// A node's transform in scene (world) space, given its transform in pixel space.
    /// Bones live in world space, so skinned outlines must be deformed there.
    fn scene_world(&self, pixel_world: Affine2) -> Affine2 {
        self.view.inverse() * pixel_world
    }
}

fn draw_node(ctx: &Ctx, node: &Node, parent: Affine2, target: &mut sk::Pixmap) {
    if !node.visible || node.opacity <= 0.0 {
        return;
    }
    let world = parent * node.transform;
    if node.opacity >= 1.0 {
        draw_content(ctx, node, world, target);
        return;
    }
    // Partial opacity applies to the node as a whole, so draw it on its own layer first.
    if ctx.cull && !may_touch(node, world, target) {
        return;
    }
    let Some(mut layer) = sk::Pixmap::new(target.width(), target.height()) else { return };
    draw_content(ctx, node, world, &mut layer);
    let paint = sk::PixmapPaint { opacity: node.opacity, ..Default::default() };
    target.draw_pixmap(0, 0, layer.as_ref(), &paint, sk::Transform::identity(), None);
}

fn draw_content(ctx: &Ctx, node: &Node, world: Affine2, target: &mut sk::Pixmap) {
    if let Some((content, mask_node, mask_shape)) = node.clip_parts() {
        draw_clipped(ctx, content, mask_node, mask_shape, world, target);
        return;
    }
    match &node.kind {
        NodeKind::Group(children) => children.iter().for_each(|child| draw_node(ctx, child, world, target)),
        NodeKind::Shape(shape) => draw_shape(ctx, shape, world, target),
        NodeKind::Instance(instance) => {
            if let Some(symbol) = ctx.scene.symbol(instance.symbol) {
                let inner = Ctx { overrides: ctx.overrides.inside(instance), ..*ctx };
                draw_node(&inner, &symbol.root, world, target);
            }
        }
        NodeKind::Camera(_) => {} // only a frame in the editor, never part of the picture
    }
}

/// Draws `content` through the silhouette of the mask shape (anti-aliased edge).
fn draw_clipped(
    ctx: &Ctx,
    content: &[Node],
    mask_node: &Node,
    mask_shape: &Shape,
    world: Affine2,
    target: &mut sk::Pixmap,
) {
    let (w, h) = (target.width(), target.height());
    let mask_world = world * mask_node.transform;
    // Content only shows through the mask, so a mask outside the target means nothing to draw.
    if ctx.cull && !shape_may_touch(mask_shape, &mask_world, target) {
        return;
    }
    let mask_path = mask_shape.effective_path(ctx.scene, ctx.scene_world(mask_world));
    let (Some(mut layer), Some(mut mask), Some(path)) = (sk::Pixmap::new(w, h), sk::Mask::new(w, h), to_skia_path(&mask_path))
    else {
        return;
    };
    content.iter().for_each(|child| draw_node(ctx, child, world, &mut layer));
    let rule = match mask_shape.fill_rule {
        FillRule::NonZero => sk::FillRule::Winding,
        FillRule::EvenOdd => sk::FillRule::EvenOdd,
    };
    mask.fill_path(&path, rule, true, to_skia_transform(&mask_world));
    target.draw_pixmap(0, 0, layer.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), Some(&mask));
}

fn draw_shape(ctx: &Ctx, shape: &Shape, world: Affine2, target: &mut sk::Pixmap) {
    if ctx.cull && !shape_may_touch(shape, &world, target) {
        return;
    }
    let Some(path) = to_skia_path(&shape.effective_path(ctx.scene, ctx.scene_world(world))) else { return };
    let transform = to_skia_transform(&world);

    if let Some(shader) = ctx.overrides.fill_for(shape).as_deref().and_then(to_skia_shader) {
        let paint = sk::Paint { shader, anti_alias: true, ..Default::default() };
        let rule = match shape.fill_rule {
            FillRule::NonZero => sk::FillRule::Winding,
            FillRule::EvenOdd => sk::FillRule::EvenOdd,
        };
        target.fill_path(&path, &paint, rule, transform, None);
    }

    if let Some(stroke) = shape.stroke.as_ref().filter(|s| s.width > 0.0) {
        if let Some(shader) = to_skia_shader(&ctx.overrides.stroke_paint_for(stroke)) {
            let paint = sk::Paint { shader, anti_alias: true, ..Default::default() };
            target.stroke_path(&path, &paint, &to_skia_stroke(stroke), transform, None);
        }
    }
}

// --- Culling ---------------------------------------------------------------------------------

/// False only if nothing of `node` (and its children) can land on the target. `world` maps to pixels.
fn may_touch(node: &Node, world: Affine2, target: &sk::Pixmap) -> bool {
    if !node.visible || node.opacity <= 0.0 {
        return false;
    }
    match &node.kind {
        NodeKind::Group(children) => children.iter().any(|child| may_touch(child, world * child.transform, target)),
        NodeKind::Shape(shape) => shape_may_touch(shape, &world, target),
        NodeKind::Instance(_) => true, // the symbol's art is not looked at here
        NodeKind::Camera(_) => false,
    }
}

/// False only if the shape is certainly outside the target. Bounds come from the anchors and their
/// handles (a Bézier curve stays inside the hull of its control points), widened for the stroke
/// and for anti-aliasing. Skinned shapes are bent by bones, so they are never skipped.
fn shape_may_touch(shape: &Shape, world: &Affine2, target: &sk::Pixmap) -> bool {
    if shape.skin.is_some() {
        return true;
    }
    let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for anchor in shape.path.subpaths.iter().flat_map(|s| &s.anchors) {
        for p in [anchor.pos, anchor.pos + anchor.handle_in, anchor.pos + anchor.handle_out] {
            let p = world.transform_point2(p);
            min = min.min(p);
            max = max.max(p);
        }
    }
    let margin = match shape.stroke.as_ref().filter(|s| s.width > 0.0) {
        Some(stroke) => {
            // Longest a transformed unit vector can get, so the stroke width is never underestimated.
            let scale = (world.matrix2.x_axis.length_squared() + world.matrix2.y_axis.length_squared()).sqrt();
            let reach = if stroke.join == LineJoin::Miter { stroke.miter_limit.max(std::f32::consts::SQRT_2) } else { std::f32::consts::SQRT_2 };
            stroke.width * 0.5 * scale * reach + 1.0
        }
        None => 1.0,
    };
    if !(min.is_finite() && max.is_finite() && margin.is_finite()) {
        return true; // empty or broken geometry: let the rasterizer decide
    }
    let bounds = Rect { min, max }.expanded(margin);
    bounds.intersects(&Rect { min: Vec2::ZERO, max: Vec2::new(target.width() as f32, target.height() as f32) })
}

// --- Conversions to tiny-skia types ----------------------------------------------------------

pub fn to_skia_path(path: &Path) -> Option<sk::Path> {
    let mut builder = sk::PathBuilder::new();
    for sub in path.subpaths.iter().filter(|s| s.anchors.len() >= 2) {
        let start = sub.anchors[0].pos;
        builder.move_to(start.x, start.y);
        for i in 0..sub.segment_count() {
            let seg = sub.segment(i);
            if seg.is_line() {
                builder.line_to(seg.p3.x, seg.p3.y);
            } else {
                builder.cubic_to(seg.c1.x, seg.c1.y, seg.c2.x, seg.c2.y, seg.p3.x, seg.p3.y);
            }
        }
        if sub.closed {
            builder.close();
        }
    }
    builder.finish()
}

pub fn to_skia_transform(m: &Affine2) -> sk::Transform {
    sk::Transform::from_row(
        m.matrix2.x_axis.x,
        m.matrix2.x_axis.y,
        m.matrix2.y_axis.x,
        m.matrix2.y_axis.y,
        m.translation.x,
        m.translation.y,
    )
}

fn to_skia_color(c: Color) -> sk::Color {
    sk::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// The tiny-skia shader of a paint: what the CPU renderer fills with. `None` when it would draw nothing (a gradient with
/// no stops, or a transform that cannot be inverted); a gradient with a single stop, or a degenerate one, is a solid color.
pub fn to_skia_shader(paint: &Paint) -> Option<sk::Shader<'static>> {
    match paint {
        Paint::Solid(color) => Some(sk::Shader::SolidColor(to_skia_color(*color))),
        Paint::Gradient(gradient) => {
            let stops = gradient
                .stops
                .iter()
                .map(|s| sk::GradientStop::new(s.offset.clamp(0.0, 1.0), to_skia_color(s.color)))
                .collect();
            let transform = to_skia_transform(&gradient.transform);
            let point = |p: crate::geometry::Vec2| sk::Point::from_xy(p.x, p.y);
            match gradient.kind {
                GradientKind::Linear { start, end } => {
                    sk::LinearGradient::new(point(start), point(end), stops, sk::SpreadMode::Pad, transform)
                }
                GradientKind::Radial { center, radius } => sk::RadialGradient::new(
                    point(center),
                    0.0,
                    point(center),
                    radius,
                    stops,
                    sk::SpreadMode::Pad,
                    transform,
                ),
            }
        }
    }
}

fn to_skia_stroke(stroke: &style::Stroke) -> sk::Stroke {
    sk::Stroke {
        width: stroke.width,
        miter_limit: stroke.miter_limit,
        line_cap: match stroke.cap {
            LineCap::Butt => sk::LineCap::Butt,
            LineCap::Round => sk::LineCap::Round,
            LineCap::Square => sk::LineCap::Square,
        },
        line_join: match stroke.join {
            LineJoin::Miter => sk::LineJoin::Miter,
            LineJoin::Round => sk::LineJoin::Round,
            LineJoin::Bevel => sk::LineJoin::Bevel,
        },
        dash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec2;

    #[test]
    fn clip_group_hides_content_outside_the_mask() {
        let mut scene = Scene::default();
        let mut big = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(100.0)));
        big.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        big.stroke = None;
        let big = scene.new_node("big", NodeKind::Shape(big));
        let mask = scene.new_node("mask", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(50.0)))));
        let mut group = scene.new_node("clip", NodeKind::Group(vec![big, mask]));
        group.clip = true;
        scene.add(group, None);

        let mut pixmap = sk::Pixmap::new(100, 100).unwrap();
        render(&scene, Affine2::IDENTITY, &mut pixmap);
        assert_eq!(pixmap.pixel(25, 25).unwrap().red(), 255);
        // Outside the mask only the artboard background is left.
        assert_ne!(pixmap.pixel(75, 75).unwrap().green(), 0);
    }

    #[test]
    fn instances_share_geometry_but_not_colors() {
        let mut scene = Scene::default();
        let mut shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(20.0)));
        shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        shape.stroke = None;
        let node = scene.new_node("sq", NodeKind::Shape(shape));
        let id = scene.add(node, None);
        let (symbol, first) = scene.make_symbol(&[id], "Square").unwrap();
        let second = scene.place_instance(symbol, Vec2::new(60.0, 10.0)).unwrap();
        if let Some(mut node) = scene.find_mut(second) {
            if let NodeKind::Instance(i) = &mut node.kind {
                i.fill = Some(Color::rgb(0, 0, 255));
            }
        }
        assert_eq!(scene.symbol_usage(symbol), 2);
        let _ = first;

        let mut pixmap = sk::Pixmap::new(100, 40).unwrap();
        render(&scene, Affine2::IDENTITY, &mut pixmap);
        let (a, b) = (pixmap.pixel(10, 10).unwrap(), pixmap.pixel(60, 10).unwrap());
        assert_eq!((a.red(), a.blue()), (255, 0));
        assert_eq!((b.red(), b.blue()), (0, 255));
    }

    #[test]
    fn centering_the_origin_keeps_gradients_in_place() {
        let mut scene = Scene::default();
        let mut shape = Shape::new(Path::rect(Vec2::new(40.0, 0.0), Vec2::new(100.0, 40.0)));
        shape.fill = Some(Paint::linear(Vec2::new(40.0, 0.0), Vec2::new(100.0, 0.0), Color::rgb(255, 0, 0), Color::rgb(0, 0, 255)));
        shape.stroke = None;
        let node = scene.new_node("grad", NodeKind::Shape(shape));
        let id = scene.add(node, None);
        let draw = |scene: &Scene| {
            let mut pixmap = sk::Pixmap::new(120, 40).unwrap();
            render(scene, Affine2::IDENTITY, &mut pixmap);
            pixmap
        };
        let before = draw(&scene);
        assert!(scene.center_origin(id));
        let after = draw(&scene);
        for x in [45, 70, 95] {
            let (a, b) = (before.pixel(x, 20).unwrap(), after.pixel(x, 20).unwrap());
            assert_eq!((a.red(), a.blue()), (b.red(), b.blue()), "pixel {x} changed");
        }
    }

    #[test]
    fn a_posed_copy_draws_the_animated_look_and_leaves_the_scene_alone() {
        use crate::animation::{NodePose, Pose};
        let mut scene = Scene::default();
        let mut shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(20.0)));
        shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        shape.stroke = None;
        let node = scene.new_node("sq", NodeKind::Shape(shape));
        let id = scene.add(node, None);

        let mut pose = Pose::new();
        pose.insert(
            id,
            NodePose { transform: Some(Affine2::from_translation(Vec2::new(50.0, 0.0))), fill: Some(Color::rgb(0, 255, 0)), ..Default::default() },
        );
        let mut posed = scene.clone();
        posed.apply_pose(&pose);
        let mut pixmap = sk::Pixmap::new(100, 40).unwrap();
        render(&posed, Affine2::IDENTITY, &mut pixmap);
        let moved = pixmap.pixel(60, 10).unwrap();
        assert_eq!((moved.red(), moved.green()), (0, 255));
        assert_eq!(scene.find(id).unwrap().transform, Affine2::IDENTITY);
    }

    #[test]
    fn skinned_shapes_are_drawn_bent() {
        let mut scene = Scene::default();
        let upper = scene.add_bone(None, Vec2::new(10.0, 50.0), 0.0, 30.0);
        let lower = scene.add_bone(Some(upper), Vec2::new(40.0, 50.0), 0.0, 30.0);
        // A bar with extra anchors in the middle of its long edges, where it has to bend.
        let outline = [(10.0, 45.0), (40.0, 45.0), (70.0, 45.0), (70.0, 55.0), (40.0, 55.0), (10.0, 55.0)];
        let anchors = outline.iter().map(|&(x, y)| crate::path::Anchor::corner(Vec2::new(x, y))).collect();
        let mut bar = Shape::new(Path { subpaths: vec![crate::path::SubPath { anchors, closed: true }] });
        bar.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        bar.stroke = None;
        let node = scene.new_node("bar", NodeKind::Shape(bar));
        let id = scene.add(node, None);
        assert!(scene.bind_skin(id, &[upper, lower]));
        scene.set_bone_world(lower, Vec2::new(40.0, 50.0), std::f32::consts::FRAC_PI_2, None); // fold the far half downwards

        let mut pixmap = sk::Pixmap::new(100, 100).unwrap();
        render(&scene, Affine2::IDENTITY, &mut pixmap);
        let covered = |x, y| pixmap.pixel(x, y).unwrap().red() == 255 && pixmap.pixel(x, y).unwrap().green() == 0;
        assert!(covered(20, 50), "the near half stays put");
        assert!(covered(40, 70), "the far half bends down");
        assert!(!covered(65, 50), "the far end left its straight position");
    }

    #[test]
    fn an_ik_driven_skinned_tentacle_is_drawn() {
        use crate::path::{Anchor, SubPath};
        let mut scene = Scene::default();
        let up = -std::f32::consts::FRAC_PI_2;
        let base = scene.add_bone(None, Vec2::new(180.0, 690.0), up, 60.0);
        let middle = scene.add_bone(Some(base), Vec2::new(180.0, 630.0), up, 60.0);
        let tip = scene.add_bone(Some(middle), Vec2::new(180.0, 570.0), up, 60.0);
        let half_width = |k: usize| 16.0 - k as f32 * 0.9;
        let point = |k: usize, side: f32| Vec2::new(180.0 + side * half_width(k), 690.0 - k as f32 * 15.0);
        let anchors = (0..=12)
            .map(|k| point(k, -1.0))
            .chain((0..=12).rev().map(|k| point(k, 1.0)))
            .map(Anchor::corner)
            .collect();
        let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: true }] });
        shape.fill = Some(Paint::Solid(Color::rgb(0, 255, 0)));
        let mut outline = crate::style::Stroke::solid(Color::rgb(20, 90, 80), 3.0);
        outline.join = LineJoin::Round;
        shape.stroke = Some(outline);
        let node = scene.new_node("t", NodeKind::Shape(shape));
        let id = scene.add(node, None);
        assert!(scene.bind_skin(id, &[base, middle, tip]));
        let mut marker = scene.new_node("m", NodeKind::Group(Vec::new()));
        marker.transform = Affine2::from_translation(Vec2::new(180.0, 505.0));
        let target = scene.add(marker, None);
        scene.create_ik(tip, target, 3).unwrap();
        scene.solve_ik();

        // The editor draws through a camera: half size, shifted. Skinned outlines must land in the
        // same place relative to the bones.
        let view = Affine2::from_translation(Vec2::new(50.0, 20.0)) * Affine2::from_scale(Vec2::splat(0.5));
        let mut pixmap = sk::Pixmap::new(400, 400).unwrap();
        render(&scene, view, &mut pixmap);
        let at = view.transform_point2(Vec2::new(180.0, 650.0));
        let p = pixmap.pixel(at.x as u32, at.y as u32).unwrap();
        assert_eq!((p.red(), p.green()), (0, 255), "tentacle body missing; bounds: {:?}", scene.world_bounds(id));
    }


    #[test]
    fn culling_never_changes_the_picture() {
        use crate::style::Stroke;
        let mut scene = Scene::default();
        // A grid of shapes around and far outside a 100 x 100 view: thick mitered strokes that reach in
        // from outside, rotated and stretched ones, curves, and shapes at partial opacity.
        let mut seed = 7_u32;
        let mut random = move |low: f32, high: f32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            low + (seed >> 8) as f32 / (1u32 << 24) as f32 * (high - low)
        };
        for i in 0..120 {
            let center = Vec2::new(random(-300.0, 400.0), random(-300.0, 400.0));
            let mut shape = Shape::new(if i % 3 == 0 { Path::ellipse(Vec2::ZERO, Vec2::new(random(2.0, 40.0), random(2.0, 40.0))) } else { Path::rect(Vec2::ZERO, Vec2::new(random(2.0, 60.0), random(2.0, 60.0))) });
            shape.fill = (i % 4 != 0).then(|| Paint::Solid(Color::rgba(random(0.0, 255.0) as u8, 80, 160, 255)));
            let mut stroke = Stroke::solid(Color::rgba(10, random(0.0, 255.0) as u8, 30, 200), random(0.5, 30.0));
            stroke.join = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][i % 3];
            stroke.cap = [LineCap::Butt, LineCap::Round, LineCap::Square][i % 3];
            stroke.miter_limit = random(1.0, 10.0);
            shape.stroke = Some(stroke);
            let mut node = scene.new_node(format!("s{i}"), NodeKind::Shape(shape));
            node.transform = Affine2::from_scale_angle_translation(Vec2::new(random(0.3, 3.0), random(0.3, 3.0)), random(0.0, std::f32::consts::TAU), center);
            node.opacity = if i % 5 == 0 { 0.5 } else { 1.0 };
            scene.add(node, None);
        }
        // Shapes just outside each edge whose strokes reach into the view: a thick line, and a sharp
        // triangle whose miter spike points inward.
        for (i, offset) in [Vec2::new(-30.0, 50.0), Vec2::new(130.0, 50.0), Vec2::new(50.0, -30.0), Vec2::new(50.0, 130.0)].into_iter().enumerate() {
            let mut line = Shape::new(Path::rect(Vec2::new(-1.0, -40.0), Vec2::new(1.0, 40.0)));
            line.fill = None;
            line.stroke = Some(Stroke { width: 50.0, ..Stroke::solid(Color::rgb(200, 0, 0), 50.0) });
            let mut node = scene.new_node(format!("edge line {i}"), NodeKind::Shape(line));
            node.transform = Affine2::from_translation(offset) * Affine2::from_angle(i as f32 * 0.7);
            scene.add(node, None);

            let tip = Path { subpaths: vec![crate::path::SubPath { anchors: [(0.0, 0.0), (-40.0, 3.0), (-40.0, -3.0)].map(|(x, y)| crate::path::Anchor::corner(Vec2::new(x, y))).to_vec(), closed: true }] };
            let mut spike = Shape::new(tip);
            spike.fill = None;
            spike.stroke = Some(Stroke { miter_limit: 20.0, ..Stroke::solid(Color::rgb(0, 0, 200), 6.0) });
            let mut node = scene.new_node(format!("edge spike {i}"), NodeKind::Shape(spike));
            node.transform = Affine2::from_translation(offset * 0.9 + Vec2::splat(5.0)) * Affine2::from_angle(i as f32 * 1.57 + 0.3);
            scene.add(node, None);
        }
        // A clip group whose mask is far away, and one that overlaps the view.
        for (n, offset) in [(0, 500.0), (1, 40.0)] {
            let content = scene.new_node("content", NodeKind::Shape(Shape::new(Path::rect(Vec2::new(-1000.0, -1000.0), Vec2::new(1000.0, 1000.0)))));
            let mask = scene.new_node("mask", NodeKind::Shape(Shape::new(Path::ellipse(Vec2::new(offset, offset), Vec2::splat(30.0)))));
            let mut group = scene.new_node(format!("clip{n}"), NodeKind::Group(vec![content, mask]));
            group.clip = true;
            scene.add(group, None);
        }
        for view in [Affine2::IDENTITY, Affine2::from_translation(Vec2::new(-60.0, -40.0)) * Affine2::from_scale(Vec2::splat(2.5))] {
            let draw = |cull: bool| {
                let mut pixmap = sk::Pixmap::new(100, 100).unwrap();
                render_nodes(&scene, view, &mut pixmap, cull);
                pixmap
            };
            assert!(draw(true).data() == draw(false).data(), "culling changed the picture");
        }
    }

    #[test]
    fn renders_a_filled_circle() {
        let mut scene = Scene::default();
        let mut shape = Shape::new(Path::ellipse(Vec2::new(50.0, 50.0), Vec2::splat(20.0)));
        shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        shape.stroke = None;
        let node = scene.new_node("circle", NodeKind::Shape(shape));
        scene.add(node, None);

        let mut pixmap = sk::Pixmap::new(100, 100).unwrap();
        render(&scene, Affine2::IDENTITY, &mut pixmap);
        let center = pixmap.pixel(50, 50).unwrap();
        assert_eq!((center.red(), center.green(), center.blue()), (255, 0, 0));
    }
}
