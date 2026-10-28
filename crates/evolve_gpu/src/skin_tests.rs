//! Skinned shapes on the GPU (plan 6.1): a shape bent by bones is drawn from its outline as the bones bend it now, which is what the
//! CPU renderer draws (`Shape::effective_path`), and its triangles are made again only when the pose changes.

use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::draw_list::Unsupported;
use crate::picture_tests::{assert_same, identity, options, polyline, shape, solid, stroke, H, W};
use crate::renderer::{RenderError, Renderer};
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::render;
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, LineCap, LineJoin};
use evolve_engine::tiny_skia::Pixmap;
use evolve_engine::Vec2;

/// A bar from x = 20 to 170 with a point every 15 pixels along both edges (so that it bends), bound to three bones along it. Returns the
/// scene, the bar and the bones from the base to the tip.
fn bar() -> (Scene, NodeId, [NodeId; 3]) {
    let mut scene = Scene::default();
    let mut points = Vec::new();
    for i in 0..11 {
        points.push((20.0 + i as f32 * 15.0, 72.0));
    }
    for i in (0..11).rev() {
        points.push((20.0 + i as f32 * 15.0, 90.0));
    }
    let bar = shape(polyline(&points, true), solid(220, 80, 40, 255), Some(stroke(Color::rgb(30, 20, 20), 3.0, LineJoin::Round, LineCap::Round)));
    let node = scene.new_node("bar", NodeKind::Shape(bar));
    let bar = scene.add(node, None);
    let base = scene.add_bone(None, Vec2::new(20.0, 81.0), 0.0, 50.0);
    let middle = scene.add_bone(Some(base), Vec2::new(70.0, 81.0), 0.0, 50.0);
    let tip = scene.add_bone(Some(middle), Vec2::new(120.0, 81.0), 0.0, 50.0);
    assert!(scene.bind_skin(bar, &[base, middle, tip]));
    (scene, bar, [base, middle, tip])
}

/// Bends the bar: the middle bone by `a` about its origin, and the tip bone by `b` more, about the tip of the middle one.
fn pose(scene: &mut Scene, bones: &[NodeId; 3], a: f32, b: f32) {
    scene.set_bone_world(bones[1], Vec2::new(70.0, 81.0), a, None);
    let (_, tip) = scene.bone_segment(bones[1]).expect("a bone");
    scene.set_bone_world(bones[2], tip, a + b, None);
}

#[test]
fn a_skinned_shape_looks_in_every_pose_like_the_cpu_renderers() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, _, bones) = bar();
        assert!(Renderer::unsupported(&scene).is_empty(), "a skinned shape is not in the way of the GPU renderer any more");
        for (a, b) in [(0.0, 0.0), (0.5, 0.0), (-0.7, 0.3), (1.1, 0.6), (0.3, -1.2)] {
            pose(&mut scene, &bones, a, b);
            assert_same(gpu, &format!("bar bent by {a} and {b}"), &scene, identity(), Rule::default());
        }
    });
}

#[test]
fn a_skin_that_does_not_match_its_path_is_drawn_in_its_bind_pose_as_the_cpu_renderer_does() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut skinned = shape(polyline(&[(30.0, 30.0), (150.0, 30.0), (90.0, 110.0)], true), solid(40, 120, 220, 255), None);
        skinned.skin = Some(Box::new(evolve_engine::rig::Skin { bones: vec![], inverse_bind: vec![], bind_world: Affine2::IDENTITY, weights: vec![] }));
        let node = scene.new_node("skinned", NodeKind::Shape(skinned));
        scene.add(node, None);
        assert_same(gpu, "a skin that matches nothing", &scene, identity(), Rule::default());
    });
}

/// A picture of the CPU renderer for the same scene, to compare an animation frame with.
fn cpu(scene: &Scene) -> Vec<u8> {
    let mut pixmap = Pixmap::new(W, H).unwrap();
    render::render_content(scene, identity(), &mut pixmap);
    pixmap.take()
}

#[test]
fn an_animated_skinned_shape_is_made_again_only_when_its_pose_changes_and_every_frame_is_the_picture_a_new_renderer_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, bar, bones) = bar();
        // Twenty plain shapes around it: they must never be made again.
        for i in 0..20 {
            let at = (i % 10) as f32 * 18.0 + 6.0;
            let node = scene.new_node("plain", NodeKind::Shape(shape(Path::rect(Vec2::new(at, 120.0 + (i / 10) as f32 * 14.0), Vec2::new(at + 12.0, 130.0 + (i / 10) as f32 * 14.0)), solid(60, 160, 90, 255), None)));
            scene.add(node, None);
        }
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = renderer.render(&scene, identity(), (W, H), &options()).unwrap();
        assert_eq!(first.shapes_tessellated, 21, "{first:?}");
        for frame in 1..=24 {
            let (a, b) = (0.6 * (frame as f32 * 0.3).sin(), 0.5 * (frame as f32 * 0.45).cos());
            pose(&mut scene, &bones, a, b);
            let stats = renderer.render(&scene, identity(), (W, H), &options()).unwrap();
            assert_eq!(stats.shapes_tessellated, 1, "frame {frame}: only the skinned shape is made again: {stats:?}");
            let picture = gpu.read_texture_part(renderer.picture().unwrap(), 0, 0, W, H);
            let mut fresh = Renderer::new(gpu).unwrap();
            fresh.render(&scene, identity(), (W, H), &options()).unwrap();
            assert!(picture == gpu.read_texture_part(fresh.picture().unwrap(), 0, 0, W, H), "frame {frame}: not the picture a new renderer draws");
            if frame % 6 == 0 {
                let result = compare_pictures(&cpu(&scene), &picture, W, Rule::default());
                assert!(result.passes(), "frame {frame} against the CPU renderer: {result}");
            }
        }
        // The same pose again: nothing is made, nothing is sent.
        let again = renderer.render(&scene, identity(), (W, H), &options()).unwrap();
        assert_eq!((again.shapes_tessellated, again.vertices_uploaded), (0, 0), "{again:?}");
        // A pose it has been in before is made again: the cache keeps one outline, the one it was last asked for.
        pose(&mut scene, &bones, 0.2, 0.2);
        assert_eq!(renderer.render(&scene, identity(), (W, H), &options()).unwrap().shapes_tessellated, 1);
        // A bone that moves without touching the shape's pose (a bone the skin is not bound to) leaves the skinned shape alone.
        let loose = scene.add_bone(None, Vec2::new(5.0, 5.0), 0.0, 10.0);
        renderer.render(&scene, identity(), (W, H), &options()).unwrap();
        scene.set_bone_world(loose, Vec2::new(5.0, 5.0), 1.0, None);
        let stats = renderer.render(&scene, identity(), (W, H), &options()).unwrap();
        assert_eq!(stats.shapes_tessellated, 0, "{stats:?}");
        let _ = bar;
    });
}

#[test]
fn a_skinned_shape_that_is_bent_into_the_view_from_outside_it_is_drawn() {
    Gpu::for_each_test_mode(|gpu| {
        // The bind pose is far to the right of the view (200 wide): the shape is judged by its bind pose nowhere.
        let mut scene = Scene::default();
        let base = scene.add_bone(None, Vec2::new(400.0, 80.0), 0.0, 60.0);
        let mut points = Vec::new();
        for i in 0..9 {
            points.push((400.0 + i as f32 * 8.0, 70.0));
        }
        for i in (0..9).rev() {
            points.push((400.0 + i as f32 * 8.0, 90.0));
        }
        let node = scene.new_node("arm", NodeKind::Shape(shape(polyline(&points, true), solid(200, 60, 160, 255), Some(stroke(Color::BLACK, 2.0, LineJoin::Miter, LineCap::Butt)))));
        let arm = scene.add(node, None);
        assert!(scene.bind_skin(arm, &[base]));
        // The bone swings round to the left, onto the view.
        scene.set_bone_world(base, Vec2::new(150.0, 80.0), std::f32::consts::PI, None);
        let mut renderer = Renderer::new(gpu).unwrap();
        let stats = renderer.render(&scene, identity(), (W, H), &options()).unwrap();
        assert!(stats.items > 0, "{stats:?}");
        assert_same(gpu, "a bar bent into the view", &scene, identity(), Rule::default());
    });
}

#[test]
fn a_skinned_shape_with_opacity_gets_a_layer_where_the_bones_put_it_and_one_inside_a_clip_is_clipped() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, bar, bones) = bar();
        pose(&mut scene, &bones, 0.9, 0.7);
        // Half see-through with a fill and a stroke that are not opaque: it is drawn on a layer, whose size is the bent outline's.
        scene.find_mut(bar).unwrap().set_opacity(0.5);
        if let Some(NodeKind::Shape(s)) = scene.find_mut(bar).map(|mut n| std::mem::replace(&mut n.kind, NodeKind::Group(Vec::new()))) {
            let mut s = s;
            s.fill = solid(220, 80, 40, 160);
            scene.find_mut(bar).unwrap().kind = NodeKind::Shape(s);
        }
        let stats = Renderer::new(gpu).unwrap().render(&scene, identity(), (W, H), &options()).unwrap();
        assert!(stats.layers >= 1, "{stats:?}");
        assert_same(gpu, "a half see-through bent bar", &scene, identity(), Rule::default());

        // A clip group with a plain mask over the bent bar.
        let (mut scene, bar, bones) = bar_in_a_clip();
        pose(&mut scene, &bones, -0.8, 0.5);
        let _ = bar;
        assert_same(gpu, "a bent bar inside a clip", &scene, identity(), Rule::default());
    });
}

/// The bar as the content of a clip group whose mask is an ellipse over its middle.
fn bar_in_a_clip() -> (Scene, NodeId, [NodeId; 3]) {
    let (mut scene, bar, bones) = bar();
    let content = scene.remove(bar).expect("the bar");
    let mask = scene.new_node("mask", NodeKind::Shape(shape(Path::ellipse(Vec2::new(95.0, 85.0), Vec2::new(70.0, 40.0)), solid(0, 0, 0, 255), None)));
    let mut group = scene.new_node("clip", NodeKind::Group(vec![content, mask]));
    group.clip = true;
    scene.add(group, None);
    (scene, bar, bones)
}

#[test]
fn a_skinned_clip_mask_and_a_skinned_shape_inside_an_instance_are_the_limits_that_remain() {
    Gpu::for_each_test_mode(|gpu| {
        // A skinned mask.
        let (mut scene, _, bones) = bar();
        let mut mask: Shape = shape(Path::ellipse(Vec2::new(95.0, 85.0), Vec2::new(70.0, 40.0)), solid(0, 0, 0, 255), None);
        mask.skin = Some(Box::new(evolve_engine::rig::Skin { bones: vec![], inverse_bind: vec![], bind_world: Affine2::IDENTITY, weights: vec![] }));
        let content = scene.new_node("content", NodeKind::Shape(shape(Path::rect(Vec2::new(10.0, 10.0), Vec2::new(60.0, 40.0)), solid(10, 90, 200, 255), None)));
        let mask = scene.new_node("mask", NodeKind::Shape(mask));
        let mut group = scene.new_node("clip", NodeKind::Group(vec![content, mask]));
        group.clip = true;
        scene.add(group, None);
        let gate = Renderer::unsupported(&scene);
        assert_eq!(gate.keys().copied().collect::<Vec<_>>(), [Unsupported::Skinned], "{gate:?}");
        match Renderer::new(gpu).unwrap().render(&scene, identity(), (W, H), &options()) {
            Err(RenderError::Unsupported(what)) => assert_eq!(what, gate),
            other => panic!("expected a refusal, got {:?}", other.map(|s| s.items)),
        }
        // A skinned shape inside the art of a symbol.
        let (mut scene, bar, _) = bar();
        scene.make_symbol(&[bar], "Skinned art").expect("a symbol");
        let gate = Renderer::unsupported(&scene);
        assert_eq!(gate.keys().copied().collect::<Vec<_>>(), [Unsupported::Skinned], "{gate:?}");
        let _ = bones;
    });
}
