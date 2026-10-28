//! A stroke's width is in the shape table and the vertex shader pushes the triangles out by half of it, so a small change of a stroke's
//! width needs no new triangles (plan task C1): a game that restyles the borders of a province costs rows, not
//! tessellation. A stroke far from the width its triangles were made for (more than 10% wider, or under a quarter of it) is made again.
//! These tests change widths and compare the long-lived renderer's picture with what a fresh renderer draws for the same scene, and
//! count what was made.

use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::renderer::{FrameStats, RenderOptions, Renderer};
use crate::targets::TargetPolicy;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, LineCap, LineJoin, Paint, Stroke};
use evolve_engine::Vec2;

const SIZE: (u32, u32) = (320, 240);
const ZOOM: f32 = 5.0;
/// Shapes in the scene: 25 zigzags and 5 hexagons with a fill.
const SHAPES: usize = 30;

fn options() -> RenderOptions {
    RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() }
}

fn view() -> Affine2 {
    Affine2::from_scale(Vec2::splat(ZOOM))
}

/// Zigzags of segments about two units long (as long as the widest stroke below is wide, like the borders of the map), and every sixth
/// shape a hexagon with a fill, in a grid. Every stroke has the given join and cap and is `width` wide.
fn scene(join: LineJoin, cap: LineCap, width: f32) -> (Scene, Vec<NodeId>) {
    let mut scene = Scene::default();
    let mut ids = Vec::new();
    for i in 0..SHAPES {
        let (x, y) = ((i % 6) as f32 * 10.0 + 0.5, (i / 6) as f32 * 9.0 + 1.5);
        let path = if i % 6 == 5 {
            let corners = (0..6).map(|k| {
                let angle = k as f32 * std::f32::consts::TAU / 6.0;
                Anchor::corner(Vec2::new(x + 2.5 + 2.2 * angle.cos(), y + 2.5 + 2.2 * angle.sin()))
            });
            Path { subpaths: vec![SubPath { anchors: corners.collect(), closed: true }] }
        } else {
            let anchors = (0..7).map(|j| Anchor::corner(Vec2::new(x + j as f32 * 1.5, y + if j % 2 == 0 { 0.0 } else { 1.2 + (i % 3) as f32 * 0.4 })));
            Path { subpaths: vec![SubPath { anchors: anchors.collect(), closed: false }] }
        };
        let mut shape = Shape::new(path);
        shape.fill = (i % 6 == 5).then_some(Paint::Solid(Color::rgb(230, 190, 60)));
        shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgb(20, 20, 120)), width, join, cap, miter_limit: 4.0 });
        let node = scene.new_node(format!("s{i}"), NodeKind::Shape(shape));
        ids.push(scene.add(node, None));
    }
    (scene, ids)
}

fn set_width(scene: &mut Scene, ids: &[NodeId], width: f32) {
    for &id in ids {
        assert!(scene.find_mut(id).unwrap().set_stroke_width(width));
    }
}

fn picture(gpu: &Gpu, renderer: &Renderer) -> Vec<u8> {
    gpu.read_picture(renderer.picture().expect("a picture"))
}

/// Draws `scene` with the long-lived `renderer`, and with a fresh one. Returns the first's stats and the number of pixels that differ
/// at all. The pictures must agree as two correct drawings of the scene do (the edge-aware rule).
fn draw_both(gpu: &Gpu, renderer: &mut Renderer, scene: &Scene, what: &str) -> (FrameStats, usize) {
    let stats = renderer.render(scene, view(), SIZE, &options()).unwrap();
    let mut fresh = Renderer::new(gpu).unwrap();
    fresh.render(scene, view(), SIZE, &options()).unwrap();
    let (a, b) = (picture(gpu, renderer), picture(gpu, &fresh));
    let result = compare_pictures(&a, &b, SIZE.0, Rule { interior_limit: 2, ..Rule::default() });
    assert!(result.passes(), "{what}: not the picture a fresh renderer draws ({result}) ({stats:?})");
    let differ = a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).filter(|(p, q)| p != q).count();
    (stats, differ)
}

const KINDS: [(LineJoin, LineCap); 8] = [
    (LineJoin::Miter, LineCap::Butt),
    (LineJoin::Miter, LineCap::Square),
    (LineJoin::Miter, LineCap::Round),
    (LineJoin::Bevel, LineCap::Butt),
    (LineJoin::Bevel, LineCap::Round),
    (LineJoin::Round, LineCap::Butt),
    (LineJoin::Round, LineCap::Round),
    (LineJoin::Bevel, LineCap::Square),
];

#[test]
fn a_small_change_of_width_needs_no_new_triangles_and_draws_as_a_fresh_renderer_does() {
    Gpu::for_each_test_mode(|gpu| {
        for (join, cap) in KINDS {
            let (mut scene, ids) = scene(join, cap, 2.0);
            let mut renderer = Renderer::new(gpu).unwrap();
            let (first, _) = draw_both(gpu, &mut renderer, &scene, "first");
            assert_eq!(first.shapes_tessellated, SHAPES);
            // Up to 10% wider than the 2 the triangles were made for, and down to a quarter of it: thicker and thinner, in any order.
            for width in [2.2, 1.0, 2.0, 0.6, 2.1, 0.5, 2.2] {
                set_width(&mut scene, &ids, width);
                let what = format!("{join:?}/{cap:?} to {width}");
                let (stats, _) = draw_both(gpu, &mut renderer, &scene, &what);
                assert_eq!(stats.shapes_tessellated, 0, "{what}: {stats:?}");
                assert_eq!(stats.geometry_bytes_uploaded, 0, "{what}: the triangles stay where they are");
                assert!(stats.table_bytes_uploaded > 0, "{what}: the widths are sent");
                assert!(!stats.reused_frame, "{what}: the frame changed");
            }
        }
    });
}

#[test]
fn a_stroke_far_from_the_width_its_triangles_were_made_for_is_made_again_and_then_draws_exactly_as_a_fresh_renderer_does() {
    Gpu::for_each_test_mode(|gpu| {
        // (Once patching the last frame, once walking the scene in every frame: the cache decides in both.)
        for (join, cap, reuse) in KINDS.into_iter().flat_map(|(join, cap)| [(join, cap, true), (join, cap, false)]) {
            let (mut scene, ids) = scene(join, cap, 1.0);
            let mut renderer = Renderer::new(gpu).unwrap();
            renderer.set_frame_reuse(reuse);
            draw_both(gpu, &mut renderer, &scene, "first");
            // (the width, whether the triangles are made again): made for 1, then for 1.2, 3, 0.5 and 2.
            let steps = [(1.1, false), (1.2, true), (3.0, true), (2.9, false), (3.3, false), (3.4, true), (0.5, true), (0.13, false), (0.12, true), (2.0, true)];
            for (width, made_again) in steps {
                set_width(&mut scene, &ids, width);
                let what = format!("{join:?}/{cap:?} to {width}, reuse {reuse}");
                let (stats, differ) = draw_both(gpu, &mut renderer, &scene, &what);
                assert_eq!(stats.shapes_tessellated, if made_again { SHAPES } else { 0 }, "{what}: {stats:?}");
                assert_eq!(stats.geometry_bytes_uploaded > 0, made_again, "{what}: {stats:?}");
                if made_again {
                    assert_eq!(differ, 0, "{what}: made for this width, so the same pixels as a fresh renderer's");
                }
            }
        }
    });
}

#[test]
fn a_stroke_that_appears_or_vanishes_gets_or_loses_its_triangles() {
    Gpu::for_each_test_mode(|gpu| {
        for (join, cap) in [(LineJoin::Miter, LineCap::Butt), (LineJoin::Round, LineCap::Round)] {
            let (mut scene, ids) = scene(join, cap, 0.0);
            let mut renderer = Renderer::new(gpu).unwrap();
            let (first, _) = draw_both(gpu, &mut renderer, &scene, "no strokes");
            assert_eq!(first.items, 5, "only the five hexagons have a fill to draw: {first:?}");
            set_width(&mut scene, &ids, 2.0);
            let (stats, _) = draw_both(gpu, &mut renderer, &scene, "strokes appear");
            assert_eq!(stats.items, 30 + 5, "{join:?}: a stroke for each shape, a fill for five: {stats:?}");
            set_width(&mut scene, &ids, 0.0);
            let (stats, _) = draw_both(gpu, &mut renderer, &scene, "strokes vanish");
            assert_eq!(stats.items, 5, "{join:?}: {stats:?}");
            set_width(&mut scene, &ids, 1.5);
            let (stats, _) = draw_both(gpu, &mut renderer, &scene, "strokes come back");
            assert_eq!(stats.items, 35, "{join:?}: {stats:?}");
        }
    });
}

#[test]
fn only_the_shapes_whose_width_changed_are_touched_and_a_border_that_goes_bold_and_thin_again_costs_nothing_the_second_time() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = scene(LineJoin::Round, LineCap::Round, 1.0);
        let mut renderer = Renderer::new(gpu).unwrap();
        draw_both(gpu, &mut renderer, &scene, "first");
        // Three borders become bold (3 times wider: made again).
        set_width(&mut scene, &ids[..3], 3.0);
        let (stats, _) = draw_both(gpu, &mut renderer, &scene, "three bold");
        assert_eq!((stats.shapes_tessellated, stats.patched_items), (3, 3), "{stats:?}");
        // Thin again, bold again, thin again: the triangles made for 3 serve (1 is a third of it, above the quarter).
        for width in [1.0, 3.0, 1.0] {
            set_width(&mut scene, &ids[..3], width);
            let (stats, _) = draw_both(gpu, &mut renderer, &scene, &format!("{width}"));
            assert_eq!((stats.shapes_tessellated, stats.patched_items, stats.geometry_bytes_uploaded), (0, 3, 0), "{width}: {stats:?}");
        }
        // A colour alone is one row.
        assert!(scene.find_mut(ids[4]).unwrap().set_solid_stroke_color(Color::rgb(200, 0, 0)));
        let (stats, _) = draw_both(gpu, &mut renderer, &scene, "a colour");
        assert_eq!((stats.shapes_tessellated, stats.patched_items), (0, 1), "{stats:?}");
    });
}

#[test]
fn only_a_stroke_with_a_width_above_zero_is_drawn_and_a_width_is_good_for_the_triangles_made_within_its_range() {
    use crate::tessellate::{stroke_width_holds, visible_stroke_width};
    let shape_with = |width: Option<f32>| {
        let mut shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::ONE));
        shape.stroke = width.map(|width| Stroke::solid(Color::BLACK, width));
        shape
    };
    for (width, drawn) in [(Some(2.5), 2.5), (Some(0.0), 0.0), (Some(-1.0), 0.0), (Some(f32::NAN), 0.0), (Some(f32::INFINITY), 0.0), (None, 0.0)] {
        assert_eq!(visible_stroke_width(&shape_with(width)), drawn, "{width:?}");
    }
    let holds = |made: f32, now: Option<f32>| stroke_width_holds(made, &shape_with(now));
    assert!(holds(2.0, Some(2.0)) && holds(2.0, Some(2.2)) && holds(2.0, Some(0.5)));
    assert!(!holds(2.0, Some(2.3)) && !holds(2.0, Some(0.49)));
    assert!(holds(0.0, None) && holds(0.0, Some(0.0)) && holds(0.0, Some(-3.0)), "no stroke before, none now");
    assert!(!holds(0.0, Some(1.0)), "there are no triangles to widen");
    assert!(!holds(2.0, Some(f32::NAN)), "a stroke that stops being drawn is made again, which is cheap");
}

/// How much a mesh made for one width differs from a fresh one at another, for each kind of join and cap. A probe, not a test: it prints.
/// `cargo test --release -p evolve_gpu what_a_width_change_does_to_each_kind_of_stroke -- --ignored --nocapture`
#[test]
#[ignore]
fn what_a_width_change_does_to_each_kind_of_stroke() {
    Gpu::for_each_test_mode(|gpu| {
        for (join, cap) in KINDS {
            for (from, to) in [(1.0, 1.1), (3.0, 1.0), (3.0, 0.75), (3.0, 0.5)] {
                let (mut scene, ids) = scene(join, cap, from);
                let mut renderer = Renderer::new(gpu).unwrap();
                renderer.render(&scene, view(), SIZE, &options()).unwrap();
                set_width(&mut scene, &ids, to);
                let stats = renderer.render(&scene, view(), SIZE, &options()).unwrap();
                let mut fresh = Renderer::new(gpu).unwrap();
                fresh.render(&scene, view(), SIZE, &options()).unwrap();
                let (a, b) = (picture(gpu, &renderer), picture(gpu, &fresh));
                let result = compare_pictures(&a, &b, SIZE.0, Rule { interior_limit: 2, ..Rule::default() });
                let differ = a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 40)).count();
                println!("{join:?}/{cap:?} {from} -> {to}: made {}, {differ} pixels differ clearly, passes {}", stats.shapes_tessellated, result.passes());
            }
        }
    });
}
