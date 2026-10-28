//! Picture tests: the same scene drawn by the CPU renderer (tiny-skia, the reference) and by the GPU renderer, compared
//! with the edge-aware rule of `compare.rs`, on every kind of device the machine has (plan section 6, Phase 2).
//!
//! Scenes here use only what the GPU renderer draws in Phase 2: solid fills and strokes, with transforms and groups. The
//! CPU picture is made with `render_content` (no camera frame) unless a test is about the frame.

use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::renderer::{RenderOptions, Renderer};
use crate::targets::TargetPolicy;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_engine::render;
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, LineCap, LineJoin, Paint, Stroke};
use evolve_engine::tiny_skia::Pixmap;
use evolve_engine::Vec2;

pub(crate) const W: u32 = 200;
pub(crate) const H: u32 = 160;

pub(crate) fn options() -> RenderOptions {
    RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() }
}

pub(crate) fn solid(r: u8, g: u8, b: u8, a: u8) -> Option<Paint> {
    Some(Paint::Solid(Color::rgba(r, g, b, a)))
}

pub(crate) fn shape(path: Path, fill: Option<Paint>, stroke: Option<Stroke>) -> Shape {
    Shape { path, fill, stroke, fill_rule: FillRule::NonZero, skin: None }
}

pub(crate) fn stroke(color: Color, width: f32, join: LineJoin, cap: LineCap) -> Stroke {
    Stroke { paint: Paint::Solid(color), width, join, cap, miter_limit: 4.0 }
}

pub(crate) fn polyline(points: &[(f32, f32)], closed: bool) -> Path {
    Path { subpaths: vec![SubPath { anchors: points.iter().map(|&(x, y)| Anchor::corner(Vec2::new(x, y))).collect(), closed }] }
}

pub(crate) fn add(scene: &mut Scene, shape: Shape) -> NodeId {
    let node = scene.new_node("shape", NodeKind::Shape(shape));
    scene.add(node, None)
}

fn cpu_picture(scene: &Scene, view: Affine2, with_frame: bool) -> Vec<u8> {
    let mut pixmap = Pixmap::new(W, H).unwrap();
    if with_frame {
        render::render(scene, view, &mut pixmap);
    } else {
        render::render_content(scene, view, &mut pixmap);
    }
    pixmap.take()
}

/// Draws `scene` on the GPU and returns the picture.
fn gpu_picture(gpu: &Gpu, renderer: &mut Renderer, scene: &Scene, view: Affine2, with_frame: bool) -> Vec<u8> {
    let options = RenderOptions { camera_frame: with_frame, ..options() };
    renderer.render(scene, view, (W, H), &options).unwrap_or_else(|e| panic!("{e}"));
    gpu.read_texture_part(renderer.picture().expect("a picture"), 0, 0, W, H)
}

/// Asserts that both renderers agree on `scene` under the rule, and returns the comparison.
pub(crate) fn assert_same(gpu: &Gpu, what: &str, scene: &Scene, view: Affine2, rule: Rule) -> crate::compare::Comparison {
    let mut renderer = Renderer::new(gpu).expect("a renderer");
    let cpu = cpu_picture(scene, view, false);
    let picture = gpu_picture(gpu, &mut renderer, scene, view, false);
    let result = compare_pictures(&cpu, &picture, W, rule);
    if !result.passes() {
        dump_pictures(what, &cpu, &picture);
    }
    assert!(result.passes(), "{what} ({}): {result}", gpu.mode.name());
    result
}

/// When `EVOLVE_DUMP_PICTURES` names a folder, a failed comparison leaves both pictures there as PNG files, to look at.
fn dump_pictures(what: &str, cpu: &[u8], gpu: &[u8]) {
    let Ok(folder) = std::env::var("EVOLVE_DUMP_PICTURES") else { return };
    let name: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    for (who, data) in [("cpu", cpu), ("gpu", gpu)] {
        if let Some(pixmap) = Pixmap::from_vec(data.to_vec(), evolve_engine::tiny_skia::IntSize::from_wh(W, H).expect("a size")) {
            let _ = pixmap.save_png(std::path::Path::new(&folder).join(format!("{name}-{who}.png")));
        }
    }
}

pub(crate) fn identity() -> Affine2 {
    Affine2::IDENTITY
}

/// The same small drawing at `origin` in the scene's space, seen from a camera that follows it at `zoom`. In exact arithmetic the
/// picture is the same whatever the origin is; in single precision the shapes wobble when the numbers are large (plan 5.5).
fn drawing_at(origin: Vec2) -> Scene {
    let mut scene = Scene::default();
    add(&mut scene, shape(Path::ellipse(origin + Vec2::new(0.0, 0.0), Vec2::new(3.0, 2.0)), solid(60, 120, 220, 255), Some(stroke(Color::rgb(0, 0, 0), 0.25, LineJoin::Round, LineCap::Round))));
    add(&mut scene, shape(Path::rect(origin + Vec2::new(4.0, -2.0), origin + Vec2::new(8.0, 2.0)), solid(220, 80, 60, 255), Some(stroke(Color::rgb(20, 20, 20), 0.2, LineJoin::Miter, LineCap::Butt))));
    add(&mut scene, shape(polyline(&[(origin.x - 3.0, origin.y + 3.0), (origin.x + 2.0, origin.y + 4.0), (origin.x + 7.0, origin.y + 3.0)], false), None, Some(stroke(Color::rgb(20, 140, 60), 0.15, LineJoin::Round, LineCap::Round))));
    scene
}

/// The same idea with straight sides only (nothing to flatten): where the error is the transform's alone.
fn straight_drawing_at(origin: Vec2) -> Scene {
    let mut scene = Scene::default();
    add(&mut scene, shape(Path::rect(origin + Vec2::new(-4.0, -2.0), origin + Vec2::new(3.0, 2.0)), solid(60, 120, 220, 255), Some(stroke(Color::rgb(0, 0, 0), 0.25, LineJoin::Miter, LineCap::Butt))));
    add(&mut scene, shape(polyline(&[(origin.x + 3.0, origin.y - 2.0), (origin.x + 8.0, origin.y), (origin.x + 3.0, origin.y + 2.0)], true), solid(220, 80, 60, 255), Some(stroke(Color::rgb(20, 20, 20), 0.2, LineJoin::Miter, LineCap::Butt))));
    scene
}

/// Plan 5.5: a drawing far from the origin must look like the same drawing at the origin (the camera follows it). The shader works
/// relative to a point near what it shows, so that no two large numbers are subtracted (the measurement below shows the numbers).
#[test]
fn a_drawing_far_from_the_origin_looks_like_the_same_drawing_at_the_origin() {
    Gpu::for_each_test_mode(|gpu| {
        let mean_difference = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u64).sum::<u64>() as f64 / a.len() as f64;
        for (distance, zoom, limit) in [(10_000.0f32, 100.0f32, 0.02), (10_000.0, 500.0, 0.3), (1_000_000.0, 10.0, 0.3)] {
            let view_at = |origin: Vec2| Affine2::from_translation(Vec2::new(W as f32 / 2.0, H as f32 / 2.0)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(-(origin + Vec2::new(3.0, 0.0)));
            let (near, far) = (Vec2::ZERO, Vec2::new(distance, distance * 0.7));
            let mut renderer = Renderer::new(gpu).expect("a renderer");
            let at_zero = gpu_picture(gpu, &mut renderer, &straight_drawing_at(near), view_at(near), false);
            let mut renderer = Renderer::new(gpu).expect("a renderer");
            let at_distance = gpu_picture(gpu, &mut renderer, &straight_drawing_at(far), view_at(far), false);
            let mean = mean_difference(&at_zero, &at_distance);
            assert!(mean < limit, "{distance} units from the origin at zoom {zoom}: the mean difference is {mean:.3} of 255 (limit {limit})");
        }
    });
}

/// A measurement, not a check: how far from the origin the numbers of single precision stop being good enough, for the GPU renderer
/// and for the CPU renderer it is compared with. Prints the mean and the largest difference (0 to 255) from the same drawing at the
/// origin. `cargo test --release -p evolve_gpu --features egui how_far_from_the_origin -- --ignored --nocapture`
#[test]
#[ignore]
fn how_far_from_the_origin_can_a_shape_be_before_it_wobbles() {
    Gpu::for_each_test_mode(|gpu| {
        println!("{:>10} {:>6} | GPU: mean max | CPU: mean max", "origin", "zoom");
        for zoom in [10.0f32, 100.0, 500.0, 1000.0] {
            // Centered on the ellipse's right edge, where its outline is, so that there is something to see at any zoom.
            let view_at = |origin: Vec2| Affine2::from_translation(Vec2::new(W as f32 / 2.0, H as f32 / 2.0)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(-(origin + Vec2::new(3.0, 0.0)));
            let at_zero = if std::env::var("STRAIGHT").is_ok() { straight_drawing_at(Vec2::ZERO) } else { drawing_at(Vec2::ZERO) };
            let mut renderer = Renderer::new(gpu).expect("a renderer");
            let gpu_zero = gpu_picture(gpu, &mut renderer, &at_zero, view_at(Vec2::ZERO), false);
            let cpu_zero = cpu_picture(&at_zero, view_at(Vec2::ZERO), false);
            for distance in [0.0f32, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0] {
                let origin = Vec2::new(distance, distance * 0.7);
                let scene = if std::env::var("STRAIGHT").is_ok() { straight_drawing_at(origin) } else { drawing_at(origin) };
                let mut renderer = Renderer::new(gpu).expect("a renderer");
                let gpu_far = gpu_picture(gpu, &mut renderer, &scene, view_at(origin), false);
                let cpu_far = cpu_picture(&scene, view_at(origin), false);
                let stats = |a: &[u8], b: &[u8]| {
                    let total: u64 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u64).sum();
                    (total as f64 / a.len() as f64, a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0))
                };
                let (g, c) = (stats(&gpu_zero, &gpu_far), stats(&cpu_zero, &cpu_far));
                println!("{distance:>10} {zoom:>6} | {:>10.3} {:>3} | {:>10.3} {:>3}", g.0, g.1, c.0, c.1);
            }
        }
    });
}

#[test]
fn filled_rectangles_and_circles_match_the_cpu_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::rect(Vec2::new(20.0, 20.0), Vec2::new(90.4, 70.6)), solid(200, 40, 30, 255), None));
        add(&mut scene, shape(Path::ellipse(Vec2::new(130.0, 90.0), Vec2::new(40.0, 28.0)), solid(30, 80, 220, 255), None));
        add(&mut scene, shape(polyline(&[(20.0, 120.0), (80.0, 150.0), (110.0, 110.0)], true), solid(20, 160, 60, 255), None));
        assert_same(gpu, "fills", &scene, identity(), Rule::default());
    });
}

#[test]
fn strokes_with_every_join_and_cap_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let zigzag = |y: f32| polyline(&[(15.0, y + 18.0), (50.0, y), (85.0, y + 18.0), (120.0, y), (155.0, y + 18.0)], false);
        let mut y = 8.0;
        for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                let _ = cap;
            }
            add(&mut scene, shape(zigzag(y), None, Some(stroke(Color::rgb(30, 30, 60), 7.0, join, LineCap::Butt))));
            y += 26.0;
        }
        for (i, cap) in [LineCap::Butt, LineCap::Round, LineCap::Square].into_iter().enumerate() {
            let x = 20.0 + i as f32 * 60.0;
            add(&mut scene, shape(polyline(&[(x, 100.0), (x + 35.0, 100.0)], false), None, Some(stroke(Color::rgb(160, 30, 30), 10.0, LineJoin::Miter, cap))));
        }
        // A closed shape stroked all the way round, and a curve.
        add(&mut scene, shape(polyline(&[(20.0, 125.0), (60.0, 125.0), (60.0, 150.0), (20.0, 150.0)], true), None, Some(stroke(Color::rgb(20, 120, 40), 5.0, LineJoin::Miter, LineCap::Butt))));
        add(&mut scene, shape(Path::ellipse(Vec2::new(130.0, 138.0), Vec2::new(30.0, 16.0)), None, Some(stroke(Color::rgb(90, 20, 140), 4.0, LineJoin::Round, LineCap::Round))));
        assert_same(gpu, "strokes", &scene, identity(), Rule::default());
    });
}

#[test]
fn fill_and_stroke_together_and_painters_order_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // Later shapes cover earlier ones; each shape's stroke covers its own fill's edge and the shapes below.
        for i in 0..5 {
            let at = 15.0 + i as f32 * 22.0;
            let color = Color::rgb(40 + i as u8 * 40, 200 - i as u8 * 30, 90);
            add(&mut scene, shape(Path::rect(Vec2::new(at, at), Vec2::new(at + 60.0, at + 50.0)), Some(Paint::Solid(color)), Some(stroke(Color::rgb(10, 10, 30), 6.0, LineJoin::Miter, LineCap::Butt))));
        }
        assert_same(gpu, "painter's order", &scene, identity(), Rule::default());
    });
}

#[test]
fn semi_transparent_fills_and_strokes_blend_like_the_cpu_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::rect(Vec2::new(20.0, 20.0), Vec2::new(110.0, 100.0)), solid(220, 30, 30, 128), None));
        add(&mut scene, shape(Path::ellipse(Vec2::new(110.0, 90.0), Vec2::new(55.0, 45.0)), solid(30, 60, 220, 128), Some(stroke(Color::rgba(0, 0, 0, 100), 12.0, LineJoin::Round, LineCap::Butt))));
        add(&mut scene, shape(polyline(&[(10.0, 140.0), (190.0, 20.0)], false), None, Some(stroke(Color::rgba(0, 150, 0, 90), 16.0, LineJoin::Miter, LineCap::Butt))));
        assert_same(gpu, "alpha", &scene, identity(), Rule::default());
    });
}

/// Plan D11: a semi-transparent stroke that crosses itself must not be darker where it overlaps itself (tiny-skia fills the
/// stroke outline as one shape). The depth test is what does this.
#[test]
fn a_semi_transparent_stroke_does_not_blend_over_itself() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // A figure of eight with sharp inner joins: the stroke's own pieces overlap at the crossing and in the joins, and
        // two corners are sharp enough to go over the miter limit.
        add(&mut scene, shape(polyline(&[(30.0, 30.0), (170.0, 130.0), (30.0, 130.0), (170.0, 30.0), (100.0, 76.0), (60.0, 140.0)], false), None, Some(stroke(Color::rgba(40, 40, 200, 128), 14.0, LineJoin::Miter, LineCap::Butt))));
        let result = assert_same(gpu, "self overlap", &scene, identity(), Rule::default());
        assert_eq!(result.worst_interior.0, 0, "not a single interior pixel differs: {result}");
    });
}

/// Where a path turns back on itself by exactly 180 degrees, a miter join is infinitely long: tiny-skia cuts it flat (a
/// bevel) and lyon draws a spike. The tessellator cuts such strokes into pieces with flat ends (plan 3.12, fixed). Nothing
/// in the editor's own tools makes such a corner on purpose, but imported maps and SVGs can.
#[test]
fn an_exact_reversal_with_a_miter_join_matches() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(polyline(&[(30.0, 30.0), (170.0, 130.0), (30.0, 130.0), (170.0, 30.0), (100.0, 80.0), (60.0, 140.0)], false), None, Some(stroke(Color::rgba(40, 40, 200, 128), 14.0, LineJoin::Miter, LineCap::Butt))));
        assert_same(gpu, "exact reversal", &scene, identity(), Rule::default());
    });
}

/// More turns that go back on themselves: closed paths, every cap, several in a row, curves, and a path that has both a
/// reversal and ordinary corners (which keep their miters).
#[test]
fn reversals_in_closed_paths_with_every_cap_and_with_curves_match() {
    Gpu::for_each_test_mode(|gpu| {
        let blue = Color::rgba(40, 40, 200, 140);
        for (what, cap) in [("butt", LineCap::Butt), ("square", LineCap::Square), ("round", LineCap::Round)] {
            let mut scene = Scene::default();
            // An open path with a reversal in the middle and an ordinary corner after it.
            add(&mut scene, shape(polyline(&[(20.0, 40.0), (120.0, 40.0), (50.0, 40.0), (50.0, 100.0)], false), None, Some(stroke(blue, 12.0, LineJoin::Miter, cap))));
            // A closed path: a triangle with one side folded back over itself.
            add(&mut scene, shape(polyline(&[(130.0, 60.0), (190.0, 60.0), (190.0, 130.0), (160.0, 60.0)], true), None, Some(stroke(Color::rgba(200, 40, 40, 140), 10.0, LineJoin::Miter, cap))));
            // Two reversals in a row.
            add(&mut scene, shape(polyline(&[(20.0, 150.0), (100.0, 150.0), (60.0, 150.0), (140.0, 150.0)], false), None, Some(stroke(Color::rgba(20, 140, 60, 140), 8.0, LineJoin::Miter, cap))));
            assert_same(gpu, &format!("reversals, {what} caps"), &scene, identity(), Rule::default());
        }
        // A curve whose end direction is opposite to the next line's start direction.
        let mut curve = Path::default();
        curve.subpaths.push(SubPath {
            anchors: vec![
                Anchor { pos: Vec2::new(30.0, 120.0), handle_in: Vec2::ZERO, handle_out: Vec2::new(40.0, -60.0) },
                Anchor { pos: Vec2::new(130.0, 60.0), handle_in: Vec2::new(-30.0, -30.0), handle_out: Vec2::new(30.0, 30.0) },
                Anchor::corner(Vec2::new(80.0, 30.0)),
            ],
            closed: false,
        });
        let mut scene = Scene::default();
        add(&mut scene, shape(curve, None, Some(stroke(blue, 12.0, LineJoin::Miter, LineCap::Butt))));
        assert_same(gpu, "a curve that ends in a reversal", &scene, identity(), Rule::default());
    });
}

/// A shape scaled up ten times by its own transform and then looked at six times closer: its circle and its round joins and
/// caps must still be smooth (made for the bucket of the effective scale, 60, not of the view's 6).
#[test]
fn a_shape_scaled_up_by_its_transform_stays_smooth_at_a_deep_zoom() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let circle = shape(Path::ellipse(Vec2::new(0.0, 0.0), Vec2::new(7.0, 5.0)), solid(60, 120, 220, 255), Some(stroke(Color::rgba(0, 0, 0, 255), 0.8, LineJoin::Round, LineCap::Round)));
        let node = add(&mut scene, circle);
        scene.find_mut(node).unwrap().set_transform(Affine2::from_scale(Vec2::splat(10.0)) * Affine2::from_translation(Vec2::new(8.0, 6.0)));
        let line = shape(polyline(&[(2.0, 12.0), (12.0, 13.0), (4.0, 15.0)], false), None, Some(stroke(Color::rgba(200, 30, 30, 255), 0.6, LineJoin::Round, LineCap::Round)));
        let node = add(&mut scene, line);
        scene.find_mut(node).unwrap().set_transform(Affine2::from_scale(Vec2::splat(10.0)));
        for zoom in [1.0f32, 3.0, 6.0] {
            // Look at the middle of the circle's edge at that zoom.
            let view = Affine2::from_translation(Vec2::new(100.0, 80.0)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(Vec2::new(-150.0, -60.0));
            assert_same(gpu, &format!("scaled by the transform, zoom {zoom}"), &scene, view, Rule::default());
        }
    });
}

/// Mutation check of the picture tests themselves, on the card: a view moved by half a pixel is a bug that the rule must
/// catch (it is the classic way of getting pixel centers wrong), while honest MSAA differences must not be mistaken for it.
#[test]
fn a_view_that_is_off_by_half_a_pixel_is_noticed() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        for i in 0..6 {
            let at = 12.0 + i as f32 * 28.0;
            add(&mut scene, shape(Path::rect(Vec2::new(at, 20.0), Vec2::new(at + 18.0, 20.0 + 40.0 + i as f32 * 12.0)), solid(30, 30, 30, 255), None));
        }
        let mut renderer = Renderer::new(gpu).unwrap();
        let cpu = cpu_picture(&scene, identity(), false);
        let honest = compare_pictures(&cpu, &gpu_picture(gpu, &mut renderer, &scene, identity(), false), W, Rule::default());
        assert!(honest.passes(), "{honest}");
        for shift in [Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5), Vec2::new(1.0, 0.0)] {
            let moved = gpu_picture(gpu, &mut renderer, &scene, Affine2::from_translation(shift), false);
            let result = compare_pictures(&cpu, &moved, W, Rule::default());
            assert!(!result.passes(), "a shift of {shift:?} went unnoticed: {result}");
        }
    });
}

#[test]
fn transforms_groups_and_non_uniform_scale_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let inner = shape(Path::rect(Vec2::new(-20.0, -10.0), Vec2::new(20.0, 10.0)), solid(230, 140, 20, 255), Some(stroke(Color::rgb(60, 20, 0), 4.0, LineJoin::Miter, LineCap::Butt)));
        let a = {
            let mut node = scene.new_node("inner", NodeKind::Shape(inner.clone()));
            node.transform = Affine2::from_scale_angle_translation(Vec2::new(2.0, 1.0), 0.4, Vec2::new(30.0, 20.0));
            node
        };
        let b = {
            let mut node = scene.new_node("inner 2", NodeKind::Shape(inner));
            node.transform = Affine2::from_translation(Vec2::new(0.0, 45.0));
            node
        };
        let mut group = scene.new_node("group", NodeKind::Group(vec![a, b]));
        group.transform = Affine2::from_scale_angle_translation(Vec2::new(1.3, 0.8), -0.2, Vec2::new(80.0, 50.0));
        scene.add(group, None);
        // The stroke is drawn in the shape's own space, so a stretched shape has a stretched stroke, on both renderers.
        assert_same(gpu, "transforms", &scene, identity(), Rule::default());
    });
}

#[test]
fn the_camera_and_the_zoom_of_the_view_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::ellipse(Vec2::new(50.0, 40.0), Vec2::new(30.0, 20.0)), solid(30, 130, 90, 255), Some(stroke(Color::rgb(0, 0, 0), 2.0, LineJoin::Round, LineCap::Round))));
        add(&mut scene, shape(polyline(&[(10.0, 10.0), (40.0, 70.0), (90.0, 20.0)], false), None, Some(stroke(Color::rgb(200, 30, 30), 3.0, LineJoin::Miter, LineCap::Butt))));
        for (zoom, shift) in [(1.0, Vec2::ZERO), (2.5, Vec2::new(-20.0, -10.0)), (0.5, Vec2::new(60.0, 40.0)), (6.0, Vec2::new(-90.0, -60.0))] {
            let view = Affine2::from_translation(shift) * Affine2::from_scale(Vec2::splat(zoom));
            assert_same(gpu, &format!("zoom {zoom}"), &scene, view, Rule::default());
        }
        // A rotated view.
        let view = Affine2::from_translation(Vec2::new(100.0, 40.0)) * Affine2::from_angle(0.5) * Affine2::from_scale(Vec2::splat(1.5));
        assert_same(gpu, "rotated view", &scene, view, Rule::default());
    });
}

#[test]
fn the_camera_frame_background_matches_render() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::with_camera(Vec2::new(120.0, 90.0));
        let camera = scene.main_camera().unwrap();
        if let Some(mut node) = scene.find_mut(camera) {
            if let NodeKind::Camera(c) = &mut node.kind {
                c.background = Color::rgb(250, 240, 200);
            }
            node.set_transform(Affine2::from_angle_translation(0.15, Vec2::new(100.0, 80.0)));
        }
        add(&mut scene, shape(Path::ellipse(Vec2::new(90.0, 70.0), Vec2::new(25.0, 25.0)), solid(180, 40, 40, 255), None));
        let mut renderer = Renderer::new(gpu).unwrap();
        let cpu = cpu_picture(&scene, identity(), true);
        let picture = gpu_picture(gpu, &mut renderer, &scene, identity(), true);
        let result = compare_pictures(&cpu, &picture, W, Rule::default());
        assert!(result.passes(), "camera frame ({}): {result}", gpu.mode.name());
        // The frame really is drawn: a pixel inside it is the background, one outside it is not.
        assert_eq!(&picture[((50 * W + 60) * 4) as usize..][..4], &[250, 240, 200, 255]);
        assert_eq!(&picture[((10 * W + 10) * 4) as usize..][..4], &[0, 0, 0, 0]);
    });
}

#[test]
fn even_odd_compound_shapes_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut path = Path::ellipse(Vec2::new(70.0, 80.0), Vec2::new(50.0, 50.0));
        path.subpaths.extend(Path::ellipse(Vec2::new(70.0, 80.0), Vec2::new(25.0, 25.0)).subpaths);
        let mut ring = shape(path, solid(40, 40, 180, 255), None);
        ring.fill_rule = FillRule::EvenOdd;
        add(&mut scene, ring);
        let mut same_way = shape(Path::rect(Vec2::new(120.0, 30.0), Vec2::new(190.0, 130.0)), solid(180, 40, 40, 255), None);
        same_way.path.subpaths.extend(Path::rect(Vec2::new(140.0, 50.0), Vec2::new(170.0, 110.0)).subpaths);
        add(&mut scene, same_way); // nonzero: the hole that runs the same way is filled
        assert_same(gpu, "fill rules", &scene, identity(), Rule::default());
    });
}

#[test]
fn what_the_cpu_renderer_does_not_draw_is_not_drawn() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut hidden = scene.new_node("hidden", NodeKind::Shape(shape(Path::rect(Vec2::new(10.0, 10.0), Vec2::new(60.0, 60.0)), solid(255, 0, 0, 255), None)));
        hidden.visible = false;
        scene.add(hidden, None);
        let mut invisible = scene.new_node("invisible", NodeKind::Shape(shape(Path::rect(Vec2::new(70.0, 10.0), Vec2::new(120.0, 60.0)), solid(0, 255, 0, 255), None)));
        invisible.opacity = 0.0;
        scene.add(invisible, None);
        add(&mut scene, shape(Path::rect(Vec2::new(10.0, 80.0), Vec2::new(60.0, 130.0)), None, Some(stroke(Color::BLACK, 0.0, LineJoin::Miter, LineCap::Butt))));
        add(&mut scene, shape(polyline(&[(100.0, 100.0)], true), solid(0, 0, 255, 255), None));
        add(&mut scene, shape(Path::rect(Vec2::new(130.0, 90.0), Vec2::new(180.0, 140.0)), solid(10, 10, 10, 255), None));
        assert_same(gpu, "nothing extra", &scene, identity(), Rule::default());
    });
}

/// Plan D5c and task 3.1: a stroke thinner than a pixel is drawn one pixel wide with its opacity multiplied by the real
/// width, as tiny-skia does, so thin lines neither vanish nor shimmer. Most strokes of a map seen whole are like this.
#[test]
fn strokes_thinner_than_a_pixel_look_like_tiny_skias_hairlines() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let widths = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0];
        for (i, width) in widths.into_iter().enumerate() {
            let y = 12.0 + i as f32 * 18.0;
            let color = Color::rgb(20, 20, 20);
            add(&mut scene, shape(polyline(&[(10.0, y), (90.0, y + 7.3)], false), None, Some(stroke(color, width, LineJoin::Round, LineCap::Round))));
            add(&mut scene, shape(polyline(&[(110.0, y), (190.0, y)], false), None, Some(stroke(color, width, LineJoin::Miter, LineCap::Butt))));
        }
        // Zoomed out, so that ordinary widths become hairlines: the same strokes at a quarter of the size.
        for zoom in [1.0, 0.25, 0.1] {
            let view = Affine2::from_scale(Vec2::splat(zoom));
            let mut renderer = Renderer::new(gpu).unwrap();
            let cpu = cpu_picture(&scene, view, false);
            let picture = gpu_picture(gpu, &mut renderer, &scene, view, false);
            let result = compare_pictures(&cpu, &picture, W, Rule::default());
            println!("hairlines at zoom {zoom} ({}): {result}", gpu.mode.name());
            assert!(result.passes(), "hairlines at zoom {zoom} ({}): {result}", gpu.mode.name());
            // Not dim, not missing: the total amount of ink is close to what the CPU renderer puts down.
            let ink = |p: &[u8]| p.as_chunks::<4>().0.iter().map(|px| px[3] as u64).sum::<u64>() as f64;
            let ratio = ink(&picture) / ink(&cpu).max(1.0);
            assert!((0.9..1.1).contains(&ratio), "zoom {zoom}: the GPU puts down {ratio:.2} times the ink of the CPU");
        }
    });
}

/// lyon counts the miter limit as twice what the SVG rule (and tiny-skia) says and clips a miter that is too long, where
/// tiny-skia cuts it flat. The tessellator makes the bevel itself at such corners; this is the picture that proves it, for
/// corners just either side of the limit, with opaque and with transparent strokes (the pieces overlap each other).
#[test]
fn sharp_miter_corners_around_the_limit_match() {
    Gpu::for_each_test_mode(|gpu| {
        for limit in [1.0f32, 2.0, 4.0, 8.0] {
            let mut scene = Scene::default();
            // Corners opening by 17, 23, 30, 45 and 90 degrees, walking down and then back up and to the left.
            for (n, spread) in [17.0f32, 23.0, 30.0, 45.0, 90.0].into_iter().enumerate() {
                let x = 20.0 + n as f32 * 38.0;
                let rise = 70.0;
                let back = x - rise * spread.to_radians().tan();
                let mut stroke = stroke(Color::rgba(200, 40, 40, if n % 2 == 0 { 255 } else { 150 }), 8.0, LineJoin::Miter, LineCap::Butt);
                stroke.miter_limit = limit;
                add(&mut scene, shape(polyline(&[(x, 20.0), (x, 20.0 + rise), (back.max(2.0), 20.0)], false), None, Some(stroke)));
            }
            // A closed shape with a sharp corner, and a thick short one where the pieces overlap a lot.
            let mut closed = stroke(Color::rgba(30, 60, 200, 150), 14.0, LineJoin::Miter, LineCap::Butt);
            closed.miter_limit = limit;
            add(&mut scene, shape(polyline(&[(30.0, 120.0), (150.0, 140.0), (40.0, 150.0)], true), None, Some(closed)));
            assert_same(gpu, &format!("sharp miter corners, limit {limit}"), &scene, identity(), Rule::default());
        }
    });
}

// --- Gradients (plan 3.3 to 3.5) ---------------------------------------------------------------------------------------

use evolve_engine::style::{Gradient, GradientKind, GradientStop};

fn stop(offset: f32, r: u8, g: u8, b: u8, a: u8) -> GradientStop {
    GradientStop { offset, color: Color::rgba(r, g, b, a) }
}

fn linear(start: (f32, f32), end: (f32, f32), stops: Vec<GradientStop>) -> Paint {
    Paint::Gradient(Gradient { kind: GradientKind::Linear { start: Vec2::new(start.0, start.1), end: Vec2::new(end.0, end.1) }, stops, transform: Affine2::IDENTITY })
}

fn radial(center: (f32, f32), radius: f32, stops: Vec<GradientStop>) -> Paint {
    Paint::Gradient(Gradient { kind: GradientKind::Radial { center: Vec2::new(center.0, center.1), radius }, stops, transform: Affine2::IDENTITY })
}

fn with_transform(paint: Paint, transform: Affine2) -> Paint {
    match paint {
        Paint::Gradient(gradient) => Paint::Gradient(Gradient { transform, ..gradient }),
        other => other,
    }
}

fn rect_with(paint: Paint, x: f32, y: f32, w: f32, h: f32) -> Shape {
    shape(Path::rect(Vec2::new(x, y), Vec2::new(x + w, y + h)), Some(paint), None)
}

fn rainbow() -> Vec<GradientStop> {
    vec![stop(0.0, 220, 30, 30, 255), stop(0.3, 240, 220, 40, 255), stop(0.6, 40, 180, 80, 255), stop(1.0, 40, 60, 220, 255)]
}

/// Gradients are smooth, so the interior rule is the same as for flat colors: the ramp has 256 steps and tiny-skia's own
/// 8-bit rounding can differ by one or two from the lerp between two texels.
fn gradient_rule() -> Rule {
    Rule { interior_limit: 4, ..Rule::default() }
}

#[test]
fn linear_gradients_in_every_direction_with_several_stops_and_alpha_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // A checkered background shows the alpha of the gradients on top of it.
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(230, 230, 230)), 0.0, 0.0, 220.0, 160.0));
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(40, 40, 40)), 0.0, 80.0, 220.0, 20.0));
        add(&mut scene, rect_with(linear((10.0, 0.0), (90.0, 0.0), rainbow()), 10.0, 10.0, 100.0, 30.0)); // across, with a pad on both sides
        add(&mut scene, rect_with(linear((0.0, 50.0), (0.0, 130.0), vec![stop(0.0, 20, 20, 20, 255), stop(1.0, 250, 250, 250, 255)]), 120.0, 10.0, 90.0, 60.0));
        add(&mut scene, rect_with(linear((10.0, 60.0), (110.0, 140.0), vec![stop(0.0, 255, 0, 0, 255), stop(0.5, 255, 255, 0, 0), stop(1.0, 0, 0, 255, 255)]), 10.0, 60.0, 100.0, 90.0)); // diagonal, a transparent stop
        add(&mut scene, rect_with(linear((120.0, 100.0), (210.0, 100.0), vec![stop(0.0, 0, 0, 0, 255), stop(1.0, 0, 0, 0, 0)]), 120.0, 80.0, 90.0, 70.0)); // black fading out
        assert_same(gpu, "linear gradients", &scene, identity(), gradient_rule());
    });
}

#[test]
fn radial_gradients_with_transforms_on_the_gradient_and_on_the_shape_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(235, 235, 235)), 0.0, 0.0, 220.0, 160.0));
        let fire = vec![stop(0.0, 255, 250, 200, 255), stop(0.4, 250, 140, 20, 255), stop(1.0, 120, 0, 0, 255)];
        add(&mut scene, rect_with(radial((60.0, 50.0), 45.0, fire.clone()), 10.0, 5.0, 100.0, 90.0));
        // A stretched and turned gradient: an ellipse tilted by 30 degrees.
        let squash = Affine2::from_translation(Vec2::new(165.0, 50.0)) * Affine2::from_angle(0.5) * Affine2::from_scale(Vec2::new(1.6, 0.6)) * Affine2::from_translation(Vec2::new(-165.0, -50.0));
        add(&mut scene, rect_with(with_transform(radial((165.0, 50.0), 40.0, fire.clone()), squash), 115.0, 5.0, 100.0, 90.0));
        // The shape itself is scaled non-uniformly, turned and moved: the gradient goes with it.
        let node = add(&mut scene, rect_with(radial((10.0, 10.0), 12.0, vec![stop(0.0, 255, 255, 255, 255), stop(1.0, 20, 60, 160, 255)]), 0.0, 0.0, 30.0, 20.0));
        scene.find_mut(node).unwrap().set_transform(Affine2::from_translation(Vec2::new(60.0, 105.0)) * Affine2::from_angle(-0.3) * Affine2::from_scale(Vec2::new(2.0, 1.3)));
        // A gradient with a transform that skews it, on a circle.
        let skew = Affine2::from_cols_array(&[1.0, 0.0, 0.7, 1.0, 0.0, 0.0]);
        add(&mut scene, shape(Path::ellipse(Vec2::new(165.0, 120.0), Vec2::new(35.0, 28.0)), Some(with_transform(linear((130.0, 100.0), (200.0, 100.0), rainbow()), skew)), None));
        assert_same(gpu, "radial gradients and transforms", &scene, identity(), gradient_rule());
    });
}

#[test]
fn gradient_strokes_thin_wide_and_round_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(245, 245, 245)), 0.0, 0.0, 220.0, 160.0));
        let ramp = || linear((10.0, 0.0), (210.0, 0.0), rainbow());
        let line = |y: f32| polyline(&[(10.0, y), (110.0, y + 8.0), (210.0, y)], false);
        let gradient_stroke = |paint: Paint, width: f32, join: LineJoin, cap: LineCap| Stroke { paint, width, join, cap, miter_limit: 4.0 };
        add(&mut scene, shape(line(15.0), None, Some(gradient_stroke(ramp(), 14.0, LineJoin::Round, LineCap::Round))));
        add(&mut scene, shape(line(45.0), None, Some(gradient_stroke(ramp(), 4.0, LineJoin::Miter, LineCap::Butt))));
        add(&mut scene, shape(line(70.0), None, Some(gradient_stroke(ramp(), 0.4, LineJoin::Miter, LineCap::Butt)))); // thinner than a pixel: the hairline rule
        // A circle stroked with a radial gradient, and the same shape filled with another one.
        let ring = shape(
            Path::ellipse(Vec2::new(60.0, 118.0), Vec2::new(40.0, 28.0)),
            Some(radial((60.0, 118.0), 40.0, vec![stop(0.0, 255, 255, 255, 255), stop(1.0, 60, 90, 200, 255)])),
            Some(gradient_stroke(radial((60.0, 118.0), 45.0, vec![stop(0.6, 0, 0, 0, 255), stop(1.0, 220, 40, 40, 255)]), 9.0, LineJoin::Round, LineCap::Butt)),
        );
        add(&mut scene, ring);
        add(&mut scene, shape(polyline(&[(120.0, 100.0), (160.0, 150.0), (200.0, 100.0)], false), None, Some(gradient_stroke(ramp(), 10.0, LineJoin::Bevel, LineCap::Square))));
        assert_same(gpu, "gradient strokes", &scene, identity(), gradient_rule());
    });
}

#[test]
fn a_gradient_follows_the_view_when_it_is_zoomed_and_turned() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::ellipse(Vec2::new(110.0, 80.0), Vec2::new(70.0, 45.0)), Some(radial((100.0, 70.0), 60.0, rainbow())), Some(stroke(Color::BLACK, 3.0, LineJoin::Round, LineCap::Round))));
        add(&mut scene, rect_with(linear((20.0, 20.0), (60.0, 60.0), vec![stop(0.0, 255, 255, 255, 255), stop(1.0, 0, 0, 0, 255)]), 20.0, 20.0, 40.0, 40.0));
        for view in [
            Affine2::from_scale(Vec2::splat(0.5)),
            Affine2::from_translation(Vec2::new(110.0, 80.0)) * Affine2::from_angle(0.6) * Affine2::from_scale(Vec2::splat(1.4)) * Affine2::from_translation(Vec2::new(-110.0, -80.0)),
            Affine2::from_translation(Vec2::new(-200.0, -80.0)) * Affine2::from_scale(Vec2::splat(3.0)),
        ] {
            assert_same(gpu, "a gradient in a moved view", &scene, view, gradient_rule());
        }
    });
}

#[test]
fn gradients_that_are_really_one_color_or_nothing_are_drawn_as_tiny_skia_does() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(235, 235, 235)), 0.0, 0.0, 220.0, 160.0));
        let two = vec![stop(0.0, 200, 40, 40, 255), stop(1.0, 40, 40, 200, 255)];
        let cases = [
            ("one stop", linear((0.0, 0.0), (100.0, 0.0), vec![stop(0.5, 30, 160, 70, 255)])),
            ("no stops", linear((0.0, 0.0), (100.0, 0.0), vec![])),
            ("start and end the same", linear((50.0, 50.0), (50.0, 50.0), two.clone())),
            ("radius zero", radial((50.0, 50.0), 0.0, two.clone())),
            ("a transform that cannot be inverted", with_transform(linear((0.0, 0.0), (100.0, 0.0), two.clone()), Affine2::from_scale(Vec2::new(0.0, 1.0)))),
            ("stops out of order and beyond 0 to 1", linear((0.0, 0.0), (100.0, 0.0), vec![stop(1.4, 255, 0, 0, 255), stop(-0.3, 0, 0, 255, 255), stop(0.5, 0, 255, 0, 255)])),
            ("two stops at the same place", linear((0.0, 0.0), (100.0, 0.0), vec![stop(0.0, 255, 255, 0, 255), stop(0.5, 255, 0, 0, 255), stop(0.5, 0, 0, 255, 255), stop(1.0, 0, 255, 255, 255)])),
        ];
        for (n, (what, paint)) in cases.into_iter().enumerate() {
            let (x, y) = (5.0 + (n % 4) as f32 * 52.0, 5.0 + (n / 4) as f32 * 60.0);
            let node = add(&mut scene, rect_with(paint, 0.0, 0.0, 48.0, 50.0));
            scene.find_mut(node).unwrap().set_transform(Affine2::from_translation(Vec2::new(x, y)));
            let _ = what;
        }
        assert_same(gpu, "gradients that are one color or nothing", &scene, identity(), gradient_rule());
    });
}

#[test]
fn many_different_gradients_make_the_ramp_atlas_grow_and_all_still_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // 90 gradients with different stops (more than the atlas starts with), laid out in a grid of small squares.
        for i in 0..90u32 {
            let (x, y) = (4.0 + (i % 15) as f32 * 14.5, 4.0 + (i / 15) as f32 * 25.0);
            let stops = vec![stop(0.0, (i * 3) as u8, 255 - (i * 2) as u8, 40, 255), stop(1.0, 250, (i * 2) as u8, (i * 5 % 256) as u8, 255)];
            add(&mut scene, rect_with(linear((x, y), (x + 12.0, y + 20.0), stops), x, y, 12.0, 20.0));
        }
        let mut renderer = Renderer::new(gpu).expect("a renderer");
        let cpu = cpu_picture(&scene, identity(), false);
        let stats = renderer.render(&scene, identity(), (W, H), &RenderOptions { camera_frame: false, ..options() }).unwrap();
        assert_eq!(stats.ramps_in_use, 90);
        let picture = gpu_picture(gpu, &mut renderer, &scene, identity(), false);
        let result = compare_pictures(&cpu, &picture, W, gradient_rule());
        assert!(result.passes(), "many gradients ({}): {result}", gpu.mode.name());
    });
}

#[test]
fn a_linear_gradient_with_a_moved_and_turned_transform_and_a_see_through_single_color_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(235, 235, 235)), 0.0, 0.0, 220.0, 160.0));
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(20, 20, 20)), 100.0, 0.0, 20.0, 160.0));
        // The gradient's own transform moves it (to the right and down) and turns it by 40 degrees.
        let placed = Affine2::from_translation(Vec2::new(70.0, 40.0)) * Affine2::from_angle(0.7);
        add(&mut scene, rect_with(with_transform(linear((0.0, 0.0), (60.0, 0.0), rainbow()), placed), 5.0, 5.0, 90.0, 150.0));
        // One stop that is half transparent: a flat see-through color, premultiplied, over the dark bar.
        add(&mut scene, rect_with(linear((0.0, 0.0), (100.0, 0.0), vec![stop(0.5, 200, 30, 30, 128)]), 90.0, 20.0, 120.0, 100.0));
        assert_same(gpu, "a moved and turned linear gradient, a see-through single color", &scene, identity(), gradient_rule());
    });
}

/// The ramp has 256 texels and is read between two of them at the center of a texel. A ramp that rises by 8 levels per texel
/// shows a half-texel mistake, a missing blend or a step count of 255 as a difference of 4 levels or more, where an ordinary
/// gradient (one level per texel) would hide it. The rule is therefore much tighter than for the other gradients.
#[test]
fn a_steep_ramp_is_read_at_the_right_place_and_blended_between_texels() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let steep = vec![stop(0.0, 0, 0, 0, 255), stop(0.125, 255, 255, 255, 255)];
        add(&mut scene, rect_with(linear((0.0, 0.0), (220.0, 0.0), steep.clone()), 0.0, 0.0, 220.0, 70.0));
        add(&mut scene, rect_with(radial((110.0, 115.0), 200.0, steep), 0.0, 80.0, 220.0, 70.0));
        let rule = Rule { interior_limit: 2, interior_exceeding: 0.0, mean_difference: 0.5, ..Rule::default() };
        assert_same(gpu, "a steep ramp", &scene, identity(), rule);
    });
}

// --- Opacity on a single shape (plan 3.6) -------------------------------------------------------------------------------

fn faded(scene: &mut Scene, shape: Shape, opacity: f32) -> NodeId {
    let id = add(scene, shape);
    scene.find_mut(id).unwrap().set_opacity(opacity);
    id
}

#[test]
fn a_shape_with_only_a_fill_or_only_a_stroke_and_opacity_matches_the_layer_of_the_cpu_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(230, 230, 230)), 0.0, 0.0, 220.0, 160.0));
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(30, 30, 30)), 0.0, 70.0, 220.0, 20.0));
        faded(&mut scene, rect_with(Paint::Solid(Color::rgb(220, 40, 40)), 10.0, 10.0, 60.0, 130.0), 0.5);
        faded(&mut scene, rect_with(Paint::Solid(Color::rgba(40, 40, 220, 200)), 50.0, 30.0, 60.0, 100.0), 0.3); // a see-through fill that is also faded
        faded(&mut scene, rect_with(radial((165.0, 50.0), 40.0, rainbow()), 120.0, 10.0, 90.0, 100.0), 0.6);
        faded(&mut scene, shape(polyline(&[(20.0, 20.0), (110.0, 140.0), (200.0, 30.0)], false), None, Some(stroke(Color::rgb(20, 120, 60), 16.0, LineJoin::Round, LineCap::Round))), 0.5);
        faded(&mut scene, shape(polyline(&[(120.0, 140.0), (210.0, 100.0)], false), None, Some(Stroke { paint: linear((120.0, 0.0), (210.0, 0.0), rainbow()), width: 12.0, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 })), 0.7);
        faded(&mut scene, shape(polyline(&[(10.0, 150.0), (100.0, 152.0)], false), None, Some(stroke(Color::BLACK, 0.5, LineJoin::Miter, LineCap::Butt))), 0.8); // a thin line and an opacity
        assert_same(gpu, "opacity on one part", &scene, identity(), gradient_rule());
    });
}

/// The shape of plan 3.6: a fat half-transparent stroke over a fill. The CPU renderer draws the shape on a layer, so the
/// stroke covers the inner half of the fill; fading each part by itself would let the fill show through the stroke. Both
/// paints are opaque, so the GPU renderer draws the stroke first and the fill at the same depth.
#[test]
fn a_fat_stroke_over_a_fill_with_opacity_covers_the_fill_like_the_layer_does() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(235, 235, 235)), 0.0, 0.0, 220.0, 160.0));
        add(&mut scene, rect_with(Paint::Solid(Color::rgb(40, 40, 40)), 0.0, 60.0, 220.0, 15.0));
        let fat = |fill: Paint, stroke_paint: Paint, width: f32| shape(Path::ellipse(Vec2::new(0.0, 0.0), Vec2::new(40.0, 28.0)), Some(fill), Some(Stroke { paint: stroke_paint, width, join: LineJoin::Round, cap: LineCap::Round, miter_limit: 4.0 }));
        let a = faded(&mut scene, fat(Paint::Solid(Color::rgb(230, 190, 30)), Paint::Solid(Color::rgb(30, 60, 160)), 30.0), 0.5);
        scene.find_mut(a).unwrap().set_transform(Affine2::from_translation(Vec2::new(55.0, 45.0)));
        let b = faded(&mut scene, fat(radial((0.0, 0.0), 40.0, vec![stop(0.0, 255, 255, 255, 255), stop(1.0, 200, 30, 30, 255)]), linear((-40.0, 0.0), (40.0, 0.0), rainbow()), 14.0), 0.35);
        scene.find_mut(b).unwrap().set_transform(Affine2::from_translation(Vec2::new(160.0, 45.0)));
        // A square with a miter join, with a stroke thinner than the fill's edge detail, and one with a stroke wider than the shape.
        let c = faded(&mut scene, shape(Path::rect(Vec2::new(-25.0, -20.0), Vec2::new(25.0, 20.0)), Some(Paint::Solid(Color::rgb(60, 180, 90))), Some(stroke(Color::rgb(90, 20, 120), 8.0, LineJoin::Miter, LineCap::Butt))), 0.6);
        scene.find_mut(c).unwrap().set_transform(Affine2::from_translation(Vec2::new(55.0, 118.0)) * Affine2::from_angle(0.3));
        let d = faded(&mut scene, shape(Path::rect(Vec2::new(-6.0, -6.0), Vec2::new(6.0, 6.0)), Some(Paint::Solid(Color::rgb(255, 255, 255))), Some(stroke(Color::rgb(0, 0, 0), 20.0, LineJoin::Round, LineCap::Butt))), 0.5);
        scene.find_mut(d).unwrap().set_transform(Affine2::from_translation(Vec2::new(160.0, 118.0)));
        assert_same(gpu, "a fat stroke over a fill with opacity", &scene, identity(), gradient_rule());
    });
}

/// The stroke style grid of plan 3.9: every join with every cap at three widths and two miter limits, on a zigzag with a
/// sharp turn, a right angle, a curve and a short closed shape.
#[test]
fn a_grid_of_stroke_styles_matches() {
    Gpu::for_each_test_mode(|gpu| {
        let joins = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel];
        let caps = [LineCap::Butt, LineCap::Round, LineCap::Square];
        let mut scene = Scene::default();
        for (j, join) in joins.into_iter().enumerate() {
            for (c, cap) in caps.into_iter().enumerate() {
                for (w, width) in [1.5f32, 5.0, 11.0].into_iter().enumerate() {
                    let (x, y) = (6.0 + (j * 3 + c) as f32 * 24.0, 6.0 + w as f32 * 52.0);
                    let mut style = stroke(Color::rgba(30 + (c * 70) as u8, 60 + (j * 60) as u8, 200 - (w * 60) as u8, if (j + c + w) % 2 == 0 { 255 } else { 170 }), width, join, cap);
                    style.miter_limit = if w == 1 { 2.0 } else { 4.0 };
                    // a sharp turn, then a right angle, then back down
                    add(&mut scene, shape(polyline(&[(x, y + 40.0), (x + 14.0, y + 2.0), (x + 9.0, y + 26.0), (x + 18.0, y + 26.0), (x + 18.0, y + 44.0)], false), None, Some(style)));
                }
            }
        }
        // Wide enough for 9 columns of 24 units.
        let view = Affine2::from_scale(Vec2::new(220.0 / 224.0, 1.0));
        assert_same(gpu, "stroke styles", &scene, view, Rule::default());
    });
}
