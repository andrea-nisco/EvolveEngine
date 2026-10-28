//! Opacity groups and layers (plan 4.2 to 4.4 and 4.6): the GPU renderer against the CPU renderer, which draws every node with
//! partial opacity on a layer of its own and composites that. The pictures must agree where groups overlap themselves (a layer
//! is needed there), where they do not (the opacity goes into the children), with nesting, with clips, and at the edges of the
//! view.

use crate::compare::Rule;
use crate::device::Gpu;
use crate::picture_tests::{add, assert_same, identity, polyline, shape, solid, stroke, H, W};
use crate::renderer::{FrameStats, RenderOptions, Renderer};
use crate::targets::TargetPolicy;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, LineCap, LineJoin, Paint};
use evolve_engine::Vec2;

fn node(scene: &mut Scene, name: &str, shape: Shape) -> Node {
    scene.new_node(name, NodeKind::Shape(shape))
}

/// A group of `children` with `opacity`, added to the scene.
fn faded(scene: &mut Scene, children: Vec<Node>, opacity: f32) -> NodeId {
    let mut group = scene.new_node("faded", NodeKind::Group(children));
    group.opacity = opacity;
    scene.add(group, None)
}

fn rect(scene: &mut Scene, x0: f32, y0: f32, x1: f32, y1: f32, color: (u8, u8, u8, u8)) -> Node {
    node(scene, "rect", shape(Path::rect(Vec2::new(x0, y0), Vec2::new(x1, y1)), solid(color.0, color.1, color.2, color.3), None))
}

/// The scene drawn by the GPU renderer, and what the frame did.
fn stats_of(gpu: &Gpu, scene: &Scene, view: Affine2) -> FrameStats {
    let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
    Renderer::new(gpu).unwrap().render(scene, view, (W, H), &options).unwrap()
}

/// A backdrop that makes every blend visible: a gradient and a few stripes under whatever is drawn after it.
fn backdrop(scene: &mut Scene) {
    let sky = shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), Some(Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Color::rgb(250, 230, 60), Color::rgb(30, 60, 200))), None);
    add(scene, sky);
    for i in 0..4 {
        let y = 20.0 + i as f32 * 35.0;
        add(scene, shape(polyline(&[(0.0, y), (200.0, y + 10.0)], false), None, Some(stroke(Color::rgb(20, 20, 20), 5.0, LineJoin::Miter, LineCap::Butt))));
    }
}

#[test]
fn overlapping_children_of_a_faded_group_are_not_blended_twice_where_they_overlap() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        // Two opaque shapes of different colors overlapping: through a layer the one on top hides the other where they overlap,
        // and the whole is 50% over the backdrop.
        let children = vec![rect(&mut scene, 20.0, 20.0, 120.0, 100.0, (220, 40, 40, 255)), rect(&mut scene, 70.0, 50.0, 180.0, 140.0, (40, 60, 220, 255))];
        faded(&mut scene, children, 0.5);
        assert_same(gpu, "faded overlapping rects", &scene, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &scene, identity()).layers, 1);
    });
}

#[test]
fn a_faded_group_of_children_that_do_not_touch_gets_no_layer_and_the_same_picture() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        let children = vec![
            rect(&mut scene, 10.0, 10.0, 60.0, 60.0, (220, 40, 40, 255)),
            node(&mut scene, "circle", shape(Path::ellipse(Vec2::new(120.0, 40.0), Vec2::new(30.0, 25.0)), solid(40, 160, 60, 255), Some(stroke(Color::rgb(0, 0, 0), 6.0, LineJoin::Round, LineCap::Butt)))),
            node(&mut scene, "line", shape(polyline(&[(20.0, 120.0), (180.0, 140.0)], false), None, Some(stroke(Color::rgba(90, 30, 160, 255), 9.0, LineJoin::Miter, LineCap::Round)))),
        ];
        let group = faded(&mut scene, children, 0.4);
        assert_same(gpu, "faded apart", &scene, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &scene, identity()).layers, 0, "no two of them can touch: the opacity goes into each");
        // The circle's fill made see-through: the group still needs none, but the circle (fill and stroke, one of them
        // see-through, faded) is drawn on a layer of its own.
        let NodeKind::Group(children) = &mut scene.find_mut(group).unwrap().kind else { unreachable!() };
        let NodeKind::Shape(circle) = &mut children[1].kind else { unreachable!() };
        circle.fill = solid(40, 160, 60, 200);
        assert_same(gpu, "faded apart, see-through circle", &scene, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &scene, identity()).layers, 1, "only the circle");
    });
}

#[test]
fn see_through_children_that_overlap_inside_a_faded_group_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        let children = vec![
            rect(&mut scene, 20.0, 20.0, 120.0, 100.0, (220, 40, 40, 160)),
            node(&mut scene, "ring", shape(Path::ellipse(Vec2::new(100.0, 80.0), Vec2::new(60.0, 50.0)), solid(40, 200, 90, 140), Some(stroke(Color::rgba(0, 0, 0, 180), 10.0, LineJoin::Round, LineCap::Butt)))),
            node(&mut scene, "bar", shape(polyline(&[(0.0, 140.0), (200.0, 30.0)], false), None, Some(stroke(Color::rgba(250, 250, 250, 200), 14.0, LineJoin::Miter, LineCap::Butt)))),
        ];
        faded(&mut scene, children, 0.6);
        assert_same(gpu, "faded see-through overlap", &scene, identity(), Rule { interior_limit: 4, ..Rule::default() });
    });
}

#[test]
fn a_shape_with_a_see_through_fill_and_stroke_and_opacity_is_faded_as_one_whole() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        // The stroke covers half of the fill's edge: faded separately the stroke would show the fill through it, faded as one
        // whole (a layer) it does not.
        let mut s = shape(Path::ellipse(Vec2::new(70.0, 80.0), Vec2::new(50.0, 50.0)), solid(230, 60, 60, 200), Some(stroke(Color::rgba(20, 20, 160, 200), 24.0, LineJoin::Round, LineCap::Butt)));
        s.fill_rule = FillRule::NonZero;
        let id = add(&mut scene, s);
        scene.find_mut(id).unwrap().set_opacity(0.5);
        assert_same(gpu, "faded see-through shape", &scene, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &scene, identity()).layers, 1);
    });
}

#[test]
fn faded_groups_inside_faded_groups_match_at_every_level() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        // An inner group of two overlapping shapes at 70%, inside an outer group at 50% with a shape that overlaps it, and a
        // third level inside the inner group.
        let deep = vec![rect(&mut scene, 60.0, 60.0, 100.0, 100.0, (250, 250, 60, 255)), rect(&mut scene, 80.0, 70.0, 130.0, 120.0, (60, 250, 250, 255))];
        let mut deep_group = scene.new_node("deep", NodeKind::Group(deep));
        deep_group.opacity = 0.8;
        let inner = vec![rect(&mut scene, 30.0, 30.0, 110.0, 90.0, (220, 40, 40, 255)), deep_group, rect(&mut scene, 90.0, 20.0, 150.0, 80.0, (40, 60, 220, 255))];
        let mut inner_group = scene.new_node("inner", NodeKind::Group(inner));
        inner_group.opacity = 0.7;
        let outer = vec![inner_group, rect(&mut scene, 100.0, 90.0, 190.0, 150.0, (40, 160, 60, 255))];
        faded(&mut scene, outer, 0.5);
        assert_same(gpu, "nested faded groups", &scene, identity(), Rule::default());
        assert!(stats_of(gpu, &scene, identity()).layers >= 3, "outer, inner and deep each need one");
    });
}

#[test]
fn a_faded_group_with_a_transform_a_view_zoom_and_a_part_outside_the_view_matches() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        let children = vec![rect(&mut scene, -60.0, -30.0, 60.0, 30.0, (220, 40, 40, 255)), node(&mut scene, "disc", shape(Path::ellipse(Vec2::new(20.0, 10.0), Vec2::new(45.0, 35.0)), solid(40, 90, 230, 255), Some(stroke(Color::rgb(0, 0, 0), 8.0, LineJoin::Round, LineCap::Butt))))];
        let id = faded(&mut scene, children, 0.5);
        scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(150.0, 100.0)) * Affine2::from_angle(0.6));
        assert_same(gpu, "turned faded group, partly outside", &scene, identity(), Rule::default());
        // Zoomed: the group is much bigger than the view, so the layer is the whole view, and never more than that.
        let zoomed = Affine2::from_scale(Vec2::splat(3.0)) * Affine2::from_translation(Vec2::new(-110.0, -70.0));
        assert_same(gpu, "zoomed faded group", &scene, zoomed, Rule::default());
        let stats = stats_of(gpu, &scene, zoomed);
        let class = crate::layers::size_class((W, H), 16384);
        assert_eq!((stats.layers, stats.layer_pixels), (1, class.0 as u64 * class.1 as u64), "one layer of the size class of the view: {stats:?}");
        // And entirely outside: nothing to draw, nothing made.
        let away = Affine2::from_translation(Vec2::new(-5000.0, 0.0));
        let stats = stats_of(gpu, &scene, away);
        assert_eq!((stats.layers, stats.passes), (0, 1), "{stats:?}");
    });
}

#[test]
fn a_clip_group_with_opacity_a_faded_group_inside_a_clip_and_a_clip_inside_a_faded_group_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        // 1. A clip group with opacity: the clipped content is one layer.
        let content = vec![rect(&mut scene, 0.0, 0.0, 120.0, 90.0, (220, 40, 40, 255)), rect(&mut scene, 40.0, 30.0, 160.0, 120.0, (40, 60, 220, 255))];
        let mask = node(&mut scene, "mask", shape(Path::ellipse(Vec2::new(70.0, 55.0), Vec2::new(55.0, 40.0)), solid(255, 255, 255, 255), None));
        let mut children = content;
        children.push(mask);
        let mut clip = scene.new_node("clip", NodeKind::Group(children));
        clip.clip = true;
        clip.opacity = 0.5;
        scene.add(clip, None);
        // 2. A faded group of overlapping shapes inside the content of a clip group (the composite is clipped).
        let inner = vec![rect(&mut scene, 110.0, 70.0, 190.0, 140.0, (250, 250, 60, 255)), rect(&mut scene, 130.0, 90.0, 200.0, 160.0, (60, 250, 250, 255))];
        let mut inner_group = scene.new_node("inner", NodeKind::Group(inner));
        inner_group.opacity = 0.5;
        let mask = node(&mut scene, "mask", shape(Path::ellipse(Vec2::new(160.0, 115.0), Vec2::new(36.0, 36.0)), solid(255, 255, 255, 255), None));
        let mut outer = scene.new_node("outer", NodeKind::Group(vec![inner_group, mask]));
        outer.clip = true;
        scene.add(outer, None);
        // 3. A clip group inside a faded group of overlapping shapes: the clip is in the layer.
        let mask = node(&mut scene, "mask", shape(Path::rect(Vec2::new(10.5, 110.3), Vec2::new(70.2, 150.9)), solid(255, 255, 255, 255), None));
        let wash = node(&mut scene, "wash", shape(polyline(&[(0.0, 100.0), (100.0, 160.0)], false), None, Some(stroke(Color::rgb(200, 30, 140), 40.0, LineJoin::Miter, LineCap::Butt))));
        let mut clipped = scene.new_node("clipped", NodeKind::Group(vec![wash, mask]));
        clipped.clip = true;
        let beside = rect(&mut scene, 20.0, 120.0, 90.0, 155.0, (30, 150, 70, 255));
        faded(&mut scene, vec![beside, clipped], 0.6);
        assert_same(gpu, "clips and opacity together", &scene, identity(), Rule::default());
    });
}

/// Layers are made smaller than the view when the node is: a small faded group on a big view costs a small texture.
#[test]
fn a_layer_is_as_big_as_its_node_and_not_as_big_as_the_view() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let children = vec![rect(&mut scene, 10.0, 10.0, 50.0, 40.0, (220, 40, 40, 255)), rect(&mut scene, 30.0, 20.0, 70.0, 50.0, (40, 60, 220, 255))];
        faded(&mut scene, children, 0.5);
        let stats = stats_of(gpu, &scene, identity());
        let view = W as u64 * H as u64;
        assert_eq!(stats.layers, 1);
        assert!(stats.layer_pixels * 4 < view, "a 60 x 40 node got {} pixels of a view of {view}", stats.layer_pixels);
        assert!(stats.layer_pixels >= 60 * 40, "and enough for what it draws");
    });
}

/// With too little memory for the layers the frame is refused with an error that says why (the editor then uses the CPU
/// renderer), nothing is left half done, and with the memory back the next frame draws.
#[test]
fn a_frame_whose_layers_do_not_fit_the_budget_is_refused_and_the_next_one_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let children = vec![rect(&mut scene, 10.0, 10.0, 150.0, 120.0, (220, 40, 40, 255)), rect(&mut scene, 30.0, 20.0, 190.0, 150.0, (40, 60, 220, 255))];
        faded(&mut scene, children, 0.5);
        let mut renderer = Renderer::new(gpu).unwrap();
        let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(crate::alloc::Category::Layers, 1000));
        let refused = renderer.render(&scene, identity(), (W, H), &options).expect_err("a 160 x 140 layer is not 1000 bytes");
        assert!(matches!(refused, crate::renderer::RenderError::Alloc(_)), "{refused}");
        gpu.alloc.set_budgets(crate::alloc::Budgets::high());
        renderer.render(&scene, identity(), (W, H), &options).expect("with the memory back it draws");
        let after_failure = gpu.read_texture_part(renderer.picture().unwrap(), 0, 0, W, H);
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, identity(), (W, H), &options).unwrap();
        assert!(after_failure == gpu.read_texture_part(fresh.picture().unwrap(), 0, 0, W, H), "the same picture as a renderer that never failed");
    });
}

/// When the memory for layers is too small for a layer at its own size it is drawn at half the size (then a quarter) and shown
/// stretched, with the frame saying so, instead of the whole view going to the CPU renderer. The picture is then a little soft
/// at the edges of what is in the layer and right everywhere else.
#[test]
fn a_layer_that_does_not_fit_the_memory_is_drawn_smaller_and_still_looks_right() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        let children = vec![rect(&mut scene, 20.0, 15.0, 140.0, 110.0, (220, 40, 40, 255)), rect(&mut scene, 60.0, 40.0, 180.0, 140.0, (40, 60, 220, 255))];
        faded(&mut scene, children, 0.5);
        // The layer is about 160 x 125 pixels: 160 x 128 in its class, 82 KB. 40 KB hold it at half the size, not at its own.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(crate::alloc::Category::Layers, 40_000));
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!((stats.layers, stats.layers_reduced), (1, 1), "{stats:?}");
        let soft = Rule { interior_limit: 4, interior_exceeding: 0.005, edge_limit: 110, edge_exceeding: 0.05, mean_difference: 1.5, ..Rule::default() };
        assert_same(gpu, "a layer at half the size", &scene, identity(), soft);
        // With the memory for it, nothing is reduced and the picture is the exact one.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high());
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!((stats.layers, stats.layers_reduced), (1, 0), "{stats:?}");
        assert_same(gpu, "the same layer at its own size", &scene, identity(), Rule::default());
    });
}

/// Layers inside a layer when only the outer one is reduced (the inner one is made first and gets the memory), and when both are.
#[test]
fn nested_layers_with_a_reduced_one_inside_or_outside_are_placed_right() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        let deep = vec![rect(&mut scene, 60.0, 50.0, 110.0, 100.0, (250, 250, 60, 255)), rect(&mut scene, 80.0, 60.0, 130.0, 120.0, (60, 250, 250, 255))];
        let mut deep_group = scene.new_node("deep", NodeKind::Group(deep));
        deep_group.opacity = 0.8;
        let outer = vec![rect(&mut scene, 20.0, 20.0, 150.0, 90.0, (220, 40, 40, 255)), deep_group, rect(&mut scene, 100.0, 70.0, 190.0, 150.0, (40, 160, 60, 255))];
        faded(&mut scene, outer, 0.5);
        let soft = Rule { interior_limit: 10, interior_exceeding: 0.005, edge_limit: 120, edge_exceeding: 0.05, mean_difference: 2.0, ..Rule::default() };
        // Memory for the small inner layer (about 80 x 72) and the outer one only at half: the outer one is reduced.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(crate::alloc::Category::Layers, 4 * (80 * 80) + 4 * (96 * 80) + 1000));
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!(stats.layers, 2, "{stats:?}");
        assert!(stats.layers_reduced >= 1, "{stats:?}");
        assert_same(gpu, "reduced outer layer", &scene, identity(), soft);
        // Hardly any memory: both are reduced as far as they can be.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(crate::alloc::Category::Layers, 4 * (40 * 40) + 4 * (48 * 40) + 4000));
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!((stats.layers, stats.layers_reduced), (2, 2), "{stats:?}");
        assert_same(gpu, "both layers reduced", &scene, identity(), Rule { interior_limit: 50, interior_exceeding: 0.005, edge_limit: 130, edge_exceeding: 0.1, mean_difference: 3.0, ..Rule::default() });
        gpu.alloc.set_budgets(crate::alloc::Budgets::high());
    });
}

/// Layers inside layers are limited (GPU_SAFETY.md B2): deeper ones are not made, the opacity goes down into what is inside, and
/// the frame says how often that happened.
#[test]
fn layers_nested_deeper_than_the_limit_are_not_made_and_are_counted() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let mut inner: Option<Node> = None;
        for i in 0..40 {
            let mut children = vec![rect(&mut scene, 10.0 + i as f32, 10.0, 90.0 + i as f32, 80.0, (200, 40 + i as u8 * 4, 40, 255))];
            children.extend(inner.take());
            children.push(rect(&mut scene, 40.0, 30.0, 150.0, 120.0, (40, 60, 200, 255)));
            let mut group = scene.new_node("faded", NodeKind::Group(children));
            group.opacity = 0.97;
            inner = Some(group);
        }
        scene.add(inner.unwrap(), None);
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!((stats.layers, stats.layers_flattened), (crate::draw_list::MAX_LAYER_DEPTH, 40 - crate::draw_list::MAX_LAYER_DEPTH), "{stats:?}");
        // Nested within the limit, the same scene matches the CPU renderer.
        let mut scene = Scene::default();
        let mut inner: Option<Node> = None;
        for i in 0..12 {
            let mut children = vec![rect(&mut scene, 10.0 + i as f32 * 3.0, 10.0, 90.0, 80.0, (200, 40 + i as u8 * 10, 40, 255))];
            children.extend(inner.take());
            children.push(rect(&mut scene, 40.0, 30.0, 150.0, 120.0, (40, 60, 200, 255)));
            let mut group = scene.new_node("faded", NodeKind::Group(children));
            group.opacity = 0.9;
            inner = Some(group);
        }
        scene.add(inner.unwrap(), None);
        assert_same(gpu, "twelve nested layers", &scene, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &scene, identity()).layers, 12);
    });
}

/// A frame may have this many layers and no more (each is a render pass of its own): beyond that it is refused, and the editor
/// draws that view with the CPU renderer.
#[test]
fn a_scene_with_more_layers_than_a_frame_may_have_is_refused() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        for i in 0..crate::renderer::MAX_LAYERS + 1 {
            let (x, y) = ((i % 60) as f32 * 3.0, (i / 60) as f32 * 4.0);
            let children = vec![rect(&mut scene, x, y, x + 4.0, y + 3.0, (200, 40, 40, 255)), rect(&mut scene, x + 1.0, y + 1.0, x + 5.0, y + 4.0, (40, 60, 200, 255))];
            faded(&mut scene, children, 0.5);
        }
        let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
        let mut renderer = Renderer::new(gpu).unwrap();
        match renderer.render(&scene, identity(), (W, H), &options) {
            Err(crate::renderer::RenderError::TooManyLayers(n)) => assert_eq!(n, crate::renderer::MAX_LAYERS + 1),
            other => panic!("expected a refusal, got {:?}", other.map(|s| s.layers)),
        }
        assert_eq!(gpu.alloc.used(crate::alloc::Category::Layers), 0, "nothing was made for it");
    });
}

/// Enough layers that the buffer of view uniforms (one slot per pass) has to grow, with the pool reusing textures.
#[test]
fn many_layers_in_one_frame_all_match_and_the_next_frame_reuses_their_textures() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        for i in 0..40 {
            let (x, y) = ((i % 8) as f32 * 24.0 + 4.0, (i / 8) as f32 * 30.0 + 4.0);
            let children = vec![rect(&mut scene, x, y, x + 22.0, y + 18.0, (200, 40 + i as u8 * 5, 40, 255)), rect(&mut scene, x + 6.0, y + 6.0, x + 26.0, y + 24.0, (40, 60, 200 - i as u8 * 3, 255))];
            faded(&mut scene, children, 0.5);
        }
        assert_same(gpu, "forty layers", &scene, identity(), Rule::default());
        let mut renderer = Renderer::new(gpu).unwrap();
        let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
        let first = renderer.render(&scene, identity(), (W, H), &options).unwrap();
        assert_eq!((first.layers, first.passes), (40, 41), "{first:?}");
        let used = gpu.alloc.used(crate::alloc::Category::Layers);
        let second = renderer.render(&scene, identity(), (W, H), &options).unwrap();
        assert_eq!((second.layers, second.table_bytes_uploaded, second.geometry_bytes_uploaded), (40, 0, 0), "an unchanged scene changes no rows: {second:?}");
        assert_eq!(gpu.alloc.used(crate::alloc::Category::Layers), used, "the same textures were used again");
    });
}
