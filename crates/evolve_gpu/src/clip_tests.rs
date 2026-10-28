//! Clip groups (plan 4.1): the GPU renderer's stencil clipping against the CPU renderer's "draw the content on a layer, then put
//! it through the mask". The mask is the top child of a group with `clip` on; only its fill silhouette counts, not its colors,
//! stroke, opacity or visibility. With 4x multisampling the stencil is per sample, so an anti-aliased mask edge comes out as on
//! the CPU.

use crate::compare::Rule;
use crate::device::Gpu;
use crate::picture_tests::{add, assert_same, identity, polyline, shape, solid, stroke};
use crate::renderer::Renderer;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, LineCap, LineJoin, Paint};
use evolve_engine::Vec2;

fn node(scene: &mut Scene, name: &str, shape: Shape) -> Node {
    scene.new_node(name, NodeKind::Shape(shape))
}

/// A clip group of `content` (drawn first, in order) and `mask` (the top child), added to the scene.
fn clip_group(scene: &mut Scene, content: Vec<Node>, mask: Node) -> NodeId {
    let mut children = content;
    children.push(mask);
    let mut group = scene.new_node("clip", NodeKind::Group(children));
    group.clip = true;
    scene.add(group, None)
}

/// Content that spills over any mask: big overlapping see-through shapes and a thick stroke.
fn busy_content(scene: &mut Scene) -> Vec<Node> {
    vec![
        node(scene, "red", shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(150.0, 110.0)), solid(220, 40, 40, 200), None)),
        node(scene, "blue", shape(Path::ellipse(Vec2::new(110.0, 90.0), Vec2::new(80.0, 60.0)), solid(40, 70, 220, 160), Some(stroke(Color::rgb(0, 0, 0), 9.0, LineJoin::Round, LineCap::Butt)))),
        node(scene, "line", shape(polyline(&[(0.0, 150.0), (200.0, 10.0)], false), None, Some(stroke(Color::rgba(0, 160, 60, 180), 14.0, LineJoin::Miter, LineCap::Butt)))),
    ]
}

fn mask_shape(path: Path) -> Shape {
    let mut mask = shape(path, solid(255, 255, 255, 255), None);
    mask.fill_rule = FillRule::NonZero;
    mask
}

#[test]
fn content_is_cut_by_a_circle_and_by_a_turned_rectangle_with_anti_aliased_edges() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let content = busy_content(&mut scene);
        let mask = node(&mut scene, "circle", mask_shape(Path::ellipse(Vec2::new(70.0, 70.0), Vec2::new(45.3, 38.7))));
        clip_group(&mut scene, content, mask);
        // A second group: a rectangle turned by 30 degrees (its edges cut pixels at a slant).
        let content = vec![node(&mut scene, "wash", shape(Path::rect(Vec2::new(100.0, 0.0), Vec2::new(200.0, 160.0)), solid(250, 180, 30, 255), None))];
        let mut mask = node(&mut scene, "rect", mask_shape(Path::rect(Vec2::new(-30.0, -20.5), Vec2::new(30.0, 20.5))));
        mask.transform = Affine2::from_translation(Vec2::new(150.0, 80.0)) * Affine2::from_angle(0.52);
        clip_group(&mut scene, content, mask);
        assert_same(gpu, "clip circle and turned rectangle", &scene, identity(), Rule::default());
    });
}

/// The mask counts as a silhouette only (plan 4.1 and the inventory): no fill, a stroke only, 20% opacity and hidden masks all
/// clip exactly like a plain filled one.
#[test]
fn a_mask_clips_by_its_silhouette_whatever_its_own_paints_opacity_and_visibility_are() {
    Gpu::for_each_test_mode(|gpu| {
        let circle = |x: f32| Path::ellipse(Vec2::new(x, 50.0), Vec2::new(22.0, 30.0));
        type Change = Box<dyn Fn(&mut Node)>;
        let variants: Vec<(&str, Change)> = vec![
            ("no fill", Box::new(|n: &mut Node| {
                if let NodeKind::Shape(s) = &mut n.kind {
                    s.fill = None;
                }
            })),
            ("stroke only", Box::new(|n: &mut Node| {
                if let NodeKind::Shape(s) = &mut n.kind {
                    s.fill = None;
                    s.stroke = Some(stroke(Color::rgb(255, 0, 0), 12.0, LineJoin::Miter, LineCap::Butt));
                }
            })),
            ("translucent", Box::new(|n: &mut Node| n.opacity = 0.2)),
            ("hidden", Box::new(|n: &mut Node| n.visible = false)),
            ("opacity zero", Box::new(|n: &mut Node| n.opacity = 0.0)),
        ];
        let mut scene = Scene::default();
        let mut reference = Scene::default();
        for (i, (_, change)) in variants.iter().enumerate() {
            let x = 30.0 + i as f32 * 36.0;
            for (target, altered) in [(&mut scene, true), (&mut reference, false)] {
                let content = vec![node(target, "bar", shape(Path::rect(Vec2::new(x - 30.0, 10.0), Vec2::new(x + 30.0, 90.0)), solid(30, 120, 220, 255), Some(stroke(Color::rgb(0, 0, 0), 4.0, LineJoin::Miter, LineCap::Butt))))];
                let mut mask = node(target, "mask", mask_shape(circle(x)));
                if altered {
                    change(&mut mask);
                }
                clip_group(target, content, mask);
            }
        }
        // The CPU renderer is the reference for each; both scenes must also look the same as each other.
        assert_same(gpu, "mask variants", &scene, identity(), Rule::default());
        let mut renderer = Renderer::new(gpu).unwrap();
        let options = crate::picture_tests::options();
        let draw = |renderer: &mut Renderer, scene: &Scene| {
            renderer.render(scene, identity(), (crate::picture_tests::W, crate::picture_tests::H), &options).unwrap();
            gpu.read_texture_part(renderer.picture().unwrap(), 0, 0, crate::picture_tests::W, crate::picture_tests::H)
        };
        let altered = draw(&mut renderer, &scene);
        let plain = draw(&mut renderer, &reference);
        assert!(altered == plain, "a mask with other paints, opacity or visibility clips exactly like a plain one");
    });
}

#[test]
fn an_even_odd_mask_has_a_hole_and_a_mask_with_two_parts_clips_to_both() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut donut = Path::ellipse(Vec2::new(60.0, 80.0), Vec2::new(50.0, 50.0));
        donut.subpaths.extend(Path::ellipse(Vec2::new(60.0, 80.0), Vec2::new(25.0, 25.0)).subpaths);
        let mut mask_donut = mask_shape(donut);
        mask_donut.fill_rule = FillRule::EvenOdd;
        let content = busy_content(&mut scene);
        let mask = node(&mut scene, "donut", mask_donut);
        clip_group(&mut scene, content, mask);
        // The same two circles with the non-zero rule: no hole.
        let mut both = Path::ellipse(Vec2::new(150.0, 80.0), Vec2::new(40.0, 40.0));
        both.subpaths.extend(Path::ellipse(Vec2::new(150.0, 80.0), Vec2::new(20.0, 20.0)).subpaths);
        let content = vec![node(&mut scene, "fill", shape(Path::rect(Vec2::new(100.0, 20.0), Vec2::new(200.0, 140.0)), solid(40, 160, 90, 255), None))];
        let mask = node(&mut scene, "solid donut", mask_shape(both));
        clip_group(&mut scene, content, mask);
        assert_same(gpu, "even-odd mask", &scene, identity(), Rule::default());
    });
}

/// A clip inside a clip, content drawn after the clipped group (it must not be clipped: the mask comes out again), and a clip
/// group in a clip group's content that is drawn between ordinary shapes.
#[test]
fn nested_clips_and_what_is_drawn_after_a_clip_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 30.0)), solid(230, 230, 120, 255), None));
        // Outer clip: a big circle. Its content: a wash, an inner clip group (a small rectangle through the wash of another
        // color), and a shape drawn over both inside the outer clip.
        let wash = node(&mut scene, "wash", shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), solid(60, 90, 200, 255), None));
        let inner_content = vec![node(&mut scene, "inner content", shape(Path::ellipse(Vec2::new(70.0, 80.0), Vec2::new(70.0, 50.0)), solid(250, 120, 40, 220), Some(stroke(Color::rgb(80, 20, 0), 8.0, LineJoin::Round, LineCap::Butt))))];
        let mut inner_mask = node(&mut scene, "inner mask", mask_shape(Path::rect(Vec2::new(40.5, 55.3), Vec2::new(120.7, 140.2))));
        inner_mask.transform = Affine2::from_translation(Vec2::new(-5.0, 0.0));
        let mut inner = scene.new_node("inner", NodeKind::Group({
            let mut children = inner_content;
            children.push(inner_mask);
            children
        }));
        inner.clip = true;
        let over = node(&mut scene, "over", shape(polyline(&[(20.0, 20.0), (180.0, 140.0)], false), None, Some(stroke(Color::rgba(255, 255, 255, 200), 10.0, LineJoin::Miter, LineCap::Butt))));
        let outer_mask = node(&mut scene, "outer mask", mask_shape(Path::ellipse(Vec2::new(90.0, 85.0), Vec2::new(80.0, 66.0))));
        let mut outer = scene.new_node("outer", NodeKind::Group(vec![wash, inner, over, outer_mask]));
        outer.clip = true;
        scene.add(outer, None);
        // After the clips: ordinary shapes across the whole picture (not clipped), one of them a semi-transparent stroke.
        add(&mut scene, shape(polyline(&[(0.0, 140.0), (200.0, 120.0)], false), None, Some(stroke(Color::rgba(200, 30, 120, 150), 12.0, LineJoin::Miter, LineCap::Butt))));
        add(&mut scene, shape(Path::rect(Vec2::new(160.0, 100.0), Vec2::new(195.0, 150.0)), solid(30, 150, 80, 255), None));
        assert_same(gpu, "nested clips", &scene, identity(), Rule::default());
    });
}

#[test]
fn a_mask_that_is_outside_the_view_or_only_partly_inside_and_a_zoomed_in_curve_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // Far outside the view: nothing of the content is drawn (and the content must not be made either).
        let content = busy_content(&mut scene);
        let mask = node(&mut scene, "far", mask_shape(Path::ellipse(Vec2::new(5000.0, 5000.0), Vec2::new(40.0, 40.0))));
        clip_group(&mut scene, content, mask);
        // Half outside the left edge.
        let content = vec![node(&mut scene, "bar", shape(Path::rect(Vec2::new(-50.0, 20.0), Vec2::new(250.0, 140.0)), solid(180, 60, 180, 255), None))];
        let mask = node(&mut scene, "half", mask_shape(Path::ellipse(Vec2::new(0.0, 80.0), Vec2::new(70.0, 50.0))));
        clip_group(&mut scene, content, mask);
        assert_same(gpu, "partly outside", &scene, identity(), Rule::default());

        // The same curve looked at ten times closer: the mask's triangles have to be made for the zoom.
        let zoomed = Affine2::from_scale(Vec2::splat(10.0)) * Affine2::from_translation(Vec2::new(-4.0, -3.0));
        let mut scene = Scene::default();
        let content = vec![node(&mut scene, "wash", shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(40.0, 30.0)), solid(220, 120, 30, 255), None))];
        let mask = node(&mut scene, "circle", mask_shape(Path::ellipse(Vec2::new(12.0, 9.0), Vec2::new(6.0, 4.5))));
        clip_group(&mut scene, content, mask);
        assert_same(gpu, "zoomed mask", &scene, zoomed, Rule::default());
    });
}

/// A clip group whose top child is not a shape clips nothing (the CPU renderer ignores the flag), and one with no content, or
/// whose content draws nothing, draws nothing.
#[test]
fn a_clip_flag_without_a_shape_on_top_is_ignored_and_empty_clips_draw_nothing() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // The top child is a group: the flag is ignored and both shapes are drawn whole.
        let a = node(&mut scene, "a", shape(Path::rect(Vec2::new(10.0, 10.0), Vec2::new(90.0, 70.0)), solid(200, 50, 50, 255), None));
        let b = node(&mut scene, "b", shape(Path::rect(Vec2::new(40.0, 40.0), Vec2::new(120.0, 100.0)), solid(50, 50, 200, 200), None));
        let top = scene.new_node("top", NodeKind::Group(vec![b]));
        let mut group = scene.new_node("not a clip", NodeKind::Group(vec![a, top]));
        group.clip = true;
        scene.add(group, None);
        // No content at all, and a content that is hidden.
        let mask = node(&mut scene, "mask", mask_shape(Path::rect(Vec2::new(0.0, 100.0), Vec2::new(200.0, 160.0))));
        clip_group(&mut scene, vec![], mask);
        let mut hidden = node(&mut scene, "hidden", shape(Path::rect(Vec2::new(0.0, 100.0), Vec2::new(200.0, 160.0)), solid(0, 0, 0, 255), None));
        hidden.visible = false;
        let mask = node(&mut scene, "mask", mask_shape(Path::rect(Vec2::new(0.0, 100.0), Vec2::new(200.0, 160.0))));
        clip_group(&mut scene, vec![hidden], mask);
        // A mask with no area (a line of two points, in view): nothing of the content shows, and the content is drawn by
        // nobody although it is made.
        let content = vec![node(&mut scene, "bar", shape(Path::rect(Vec2::new(100.0, 100.0), Vec2::new(190.0, 150.0)), solid(10, 10, 10, 255), None))];
        let mask = node(&mut scene, "line", mask_shape(polyline(&[(100.0, 110.0), (190.0, 140.0)], false)));
        clip_group(&mut scene, content, mask);
        assert_same(gpu, "clip flag edge cases", &scene, identity(), Rule::default());
    });
}

/// The stencil has 8 bits and saturates at 255 (GPU_SAFETY.md B2): clips nested deeper than 250 are not made, and the frame says
/// so. Here every mask covers the whole view, so the picture is the same with or without them (and as the CPU renderer's).
#[test]
fn clips_nested_deeper_than_the_stencil_allows_are_left_out_and_counted() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut inner: Option<Node> = None;
        for i in 0..260 {
            let x = (i % 16) as f32 * 12.0;
            let mut children = vec![node(&mut scene, "dot", shape(Path::rect(Vec2::new(x, 10.0 + (i / 16) as f32 * 8.0), Vec2::new(x + 9.0, 15.0 + (i / 16) as f32 * 8.0)), solid(40, 90 + (i % 100) as u8, 200, 255), None))];
            children.extend(inner.take());
            children.push(node(&mut scene, "mask", mask_shape(Path::rect(Vec2::new(-10.0, -10.0), Vec2::new(300.0, 300.0)))));
            let mut group = scene.new_node("clip", NodeKind::Group(children));
            group.clip = true;
            inner = Some(group);
        }
        scene.add(inner.unwrap(), None);
        assert_same(gpu, "260 nested clips", &scene, identity(), Rule::default());
        let options = crate::renderer::RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: crate::targets::TargetPolicy::exact() };
        let stats = Renderer::new(gpu).unwrap().render(&scene, identity(), (crate::picture_tests::W, crate::picture_tests::H), &options).unwrap();
        assert_eq!(stats.clips_flattened, 260 - crate::draw_list::MAX_CLIP_LEVEL as usize, "{stats:?}");
        // Every dot is still drawn, and every clip that was made has its mask (a row for its transform).
        assert_eq!(stats.items, 260 + crate::draw_list::MAX_CLIP_LEVEL as usize, "{stats:?}");
    });
}

/// A mask with a gradient or semi-transparent fill clips like any other (only the shape counts); and the content under a mask
/// can itself be gradients and shapes with opacity.
#[test]
fn gradients_and_opacity_inside_a_clip_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut sky = shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), Some(Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0), Color::rgb(250, 200, 40), Color::rgb(40, 40, 160))), None);
        sky.stroke = None;
        let sky = node(&mut scene, "sky", sky);
        let mut faded = node(&mut scene, "faded", shape(Path::ellipse(Vec2::new(100.0, 80.0), Vec2::new(70.0, 50.0)), solid(255, 255, 255, 255), None));
        faded.opacity = 0.5;
        let mut mask = mask_shape(Path::ellipse(Vec2::new(100.0, 80.0), Vec2::new(90.0, 62.0)));
        mask.fill = Some(Paint::radial(Vec2::new(100.0, 80.0), 90.0, Color::rgba(0, 0, 0, 40), Color::rgba(0, 0, 0, 255)));
        let mask = node(&mut scene, "mask", mask);
        clip_group(&mut scene, vec![sky, faded], mask);
        assert_same(gpu, "gradients in a clip", &scene, identity(), Rule { interior_limit: 4, ..Rule::default() });
    });
}
