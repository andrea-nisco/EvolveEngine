//! Symbol instances (plan 4.5): the GPU renderer against the CPU renderer, which draws the master's root under the instance's
//! transform with the instance's colour overrides (outer instances win over inner ones) and, when the instance has opacity, as
//! a whole on a layer. The art of a symbol is made once and shared by every instance (even at different sizes); each instance
//! is its own set of items, in document order, so overlapping instances of a symbol with several see-through parts are blended
//! as the CPU does.

use crate::compare::Rule;
use crate::device::Gpu;
use crate::picture_tests::{add, assert_same, identity, polyline, shape, solid, stroke, H, W};
use crate::renderer::{FrameStats, RenderOptions, Renderer};
use crate::targets::TargetPolicy;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, LineCap, LineJoin, Paint};
use evolve_engine::symbol::SymbolId;
use evolve_engine::Vec2;

/// A symbol made of `parts` (the scene's own shapes become it), and the instance that took their place.
fn symbol_of(scene: &mut Scene, parts: Vec<Shape>) -> (SymbolId, NodeId) {
    let ids: Vec<NodeId> = parts.into_iter().map(|part| add(scene, part)).collect();
    scene.make_symbol(&ids, "Art").expect("a symbol")
}

fn place(scene: &mut Scene, symbol: SymbolId, at: Vec2, angle: f32, scale: f32) -> NodeId {
    let id = scene.place_instance(symbol, Vec2::ZERO).expect("an instance");
    scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(at) * Affine2::from_angle(angle) * Affine2::from_scale(Vec2::splat(scale)));
    id
}

fn set_overrides(scene: &mut Scene, instance: NodeId, fill: Option<Color>, stroke: Option<Color>) {
    let mut node = scene.find_mut(instance).unwrap();
    if let NodeKind::Instance(i) = &mut node.kind {
        i.fill = fill;
        i.stroke = stroke;
    }
}

fn stats_of(gpu: &Gpu, scene: &Scene, view: Affine2) -> FrameStats {
    let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
    Renderer::new(gpu).unwrap().render(scene, view, (W, H), &options).unwrap()
}

/// A two-part symbol: a rectangle with a stroke, and a circle over it (both a little see-through).
fn two_parts() -> Vec<Shape> {
    vec![
        shape(Path::rect(Vec2::new(-20.0, -14.0), Vec2::new(20.0, 14.0)), solid(220, 50, 50, 255), Some(stroke(Color::rgb(30, 10, 10), 4.0, LineJoin::Miter, LineCap::Butt))),
        shape(Path::ellipse(Vec2::new(10.0, 6.0), Vec2::new(16.0, 16.0)), solid(40, 90, 230, 170), Some(stroke(Color::rgba(0, 0, 0, 200), 3.0, LineJoin::Round, LineCap::Round))),
    ]
}

#[test]
fn instances_with_their_own_transform_scale_and_turn_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let (symbol, first) = symbol_of(&mut scene, two_parts());
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(35.0, 30.0)));
        place(&mut scene, symbol, Vec2::new(110.0, 35.0), 0.5, 1.0);
        place(&mut scene, symbol, Vec2::new(60.0, 100.0), -0.3, 2.0);
        place(&mut scene, symbol, Vec2::new(160.0, 110.0), 1.2, 0.6);
        assert_same(gpu, "instances", &scene, identity(), Rule::default());
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!(stats.items, 4 * 4, "four instances of four items each");
        // The art is made once for all of them (four nodes made, not sixteen) as long as the scales share a bucket; more than one
        // bucket can be needed here (the sizes differ by 3x), so only an upper bound.
        assert!(stats.shapes_tessellated <= 8, "the art of one symbol, in at most two buckets: {stats:?}");
    });
}

#[test]
fn colour_overrides_replace_every_fill_and_stroke_gradients_included_and_leave_missing_parts_missing() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let parts = vec![
            shape(Path::rect(Vec2::new(-20.0, -14.0), Vec2::new(20.0, 14.0)), Some(Paint::linear(Vec2::new(-20.0, 0.0), Vec2::new(20.0, 0.0), Color::rgb(250, 230, 40), Color::rgb(40, 60, 220))), Some(stroke(Color::rgb(0, 0, 0), 5.0, LineJoin::Miter, LineCap::Butt))),
            shape(polyline(&[(-25.0, 20.0), (25.0, 20.0)], false), None, Some(stroke(Color::rgb(20, 120, 60), 6.0, LineJoin::Miter, LineCap::Round))),
            shape(Path::ellipse(Vec2::new(0.0, -2.0), Vec2::new(8.0, 8.0)), solid(255, 255, 255, 255), None),
        ];
        let (symbol, first) = symbol_of(&mut scene, parts);
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(35.0, 40.0)));
        let both = place(&mut scene, symbol, Vec2::new(100.0, 40.0), 0.0, 1.0);
        set_overrides(&mut scene, both, Some(Color::rgb(200, 30, 120)), Some(Color::rgb(250, 250, 250)));
        let fill_only = place(&mut scene, symbol, Vec2::new(165.0, 40.0), 0.0, 1.0);
        set_overrides(&mut scene, fill_only, Some(Color::rgba(30, 160, 200, 140)), None);
        let stroke_only = place(&mut scene, symbol, Vec2::new(35.0, 110.0), 0.0, 1.0);
        set_overrides(&mut scene, stroke_only, None, Some(Color::rgb(250, 120, 0)));
        place(&mut scene, symbol, Vec2::new(100.0, 110.0), 0.0, 1.0);
        assert_same(gpu, "overrides", &scene, identity(), Rule { interior_limit: 4, ..Rule::default() });
    });
}

#[test]
fn nested_instances_and_the_outer_override_winning_match() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // An inner symbol (a small disc with a ring), and an outer one made of a plate with two instances of the inner one.
        let (inner, inner_instance) = symbol_of(
            &mut scene,
            vec![
                shape(Path::ellipse(Vec2::ZERO, Vec2::new(9.0, 9.0)), solid(230, 200, 40, 255), Some(stroke(Color::rgb(60, 30, 0), 3.0, LineJoin::Round, LineCap::Butt))),
                shape(Path::ellipse(Vec2::ZERO, Vec2::new(4.0, 4.0)), solid(40, 40, 160, 255), None),
            ],
        );
        scene.remove(inner_instance);
        let plate = add(&mut scene, shape(Path::rect(Vec2::new(-30.0, -14.0), Vec2::new(30.0, 14.0)), solid(60, 120, 90, 255), Some(stroke(Color::rgb(0, 0, 0), 3.0, LineJoin::Miter, LineCap::Butt))));
        let left = place(&mut scene, inner, Vec2::new(-14.0, 0.0), 0.0, 1.0);
        let right = place(&mut scene, inner, Vec2::new(14.0, 0.0), 0.0, 1.0);
        set_overrides(&mut scene, right, Some(Color::rgb(220, 60, 60)), None); // the inner instance's own override
        let (outer, outer_instance) = scene.make_symbol(&[plate, left, right], "Outer").expect("the outer symbol");
        scene.find_mut(outer_instance).unwrap().set_transform(Affine2::from_translation(Vec2::new(45.0, 35.0)));
        place(&mut scene, outer, Vec2::new(125.0, 40.0), 0.3, 1.0);
        let overridden = place(&mut scene, outer, Vec2::new(60.0, 105.0), -0.2, 1.5);
        set_overrides(&mut scene, overridden, Some(Color::rgb(30, 30, 30)), None); // the outer one wins over the inner one's
        place(&mut scene, outer, Vec2::new(150.0, 110.0), 0.0, 0.8);
        assert_same(gpu, "nested instances", &scene, identity(), Rule::default());
    });
}

/// The plan's regression for instancing: a symbol of two see-through parts, instances that overlap each other. Drawing part A
/// of every instance before part B would blend them in the wrong order; each instance is drawn whole, in document order.
#[test]
fn overlapping_instances_of_a_two_part_see_through_symbol_blend_in_document_order() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), Some(Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0), Color::rgb(250, 240, 200), Color::rgb(60, 80, 160))), None));
        let (symbol, first) = symbol_of(
            &mut scene,
            vec![
                shape(Path::rect(Vec2::new(-25.0, -20.0), Vec2::new(25.0, 20.0)), solid(220, 40, 40, 150), None),
                shape(Path::ellipse(Vec2::new(8.0, 4.0), Vec2::new(24.0, 24.0)), solid(40, 200, 80, 150), Some(stroke(Color::rgba(0, 0, 100, 160), 6.0, LineJoin::Round, LineCap::Butt))),
            ],
        );
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(50.0, 60.0)));
        for (x, y, angle) in [(75.0, 70.0, 0.2), (100.0, 80.0, -0.2), (125.0, 90.0, 0.4), (150.0, 100.0, 0.0)] {
            place(&mut scene, symbol, Vec2::new(x, y), angle, 1.0);
        }
        assert_same(gpu, "overlapping instances", &scene, identity(), Rule::default());
    });
}

#[test]
fn an_instance_with_opacity_is_faded_as_a_whole_and_so_is_an_art_with_its_own_opacity_and_clip() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        add(&mut scene, shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), solid(230, 230, 120, 255), None));
        let (symbol, first) = symbol_of(&mut scene, two_parts());
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(40.0, 40.0)));
        scene.find_mut(first).unwrap().set_opacity(0.5);
        let second = place(&mut scene, symbol, Vec2::new(70.0, 55.0), 0.3, 1.0);
        scene.find_mut(second).unwrap().set_opacity(0.5);
        // An instance without opacity of a symbol whose art is clipped: a mask inside the master.
        let content = add(&mut scene, shape(Path::rect(Vec2::new(-30.0, -30.0), Vec2::new(30.0, 30.0)), solid(250, 120, 30, 255), Some(stroke(Color::rgb(60, 20, 0), 6.0, LineJoin::Miter, LineCap::Butt))));
        let mask = add(&mut scene, shape(Path::ellipse(Vec2::ZERO, Vec2::new(22.0, 18.0)), solid(255, 255, 255, 255), None));
        let mut group = scene.new_node("clipped", NodeKind::Group(vec![]));
        group.clip = true;
        let group = scene.add(group, None);
        for id in [content, mask] {
            let node = scene.remove(id).unwrap();
            scene.add(node, Some(group));
        }
        let (clipped, clipped_instance) = scene.make_symbol(&[group], "Clipped").expect("a symbol of the clip group");
        scene.find_mut(clipped_instance).unwrap().set_transform(Affine2::from_translation(Vec2::new(60.0, 115.0)));
        place(&mut scene, clipped, Vec2::new(140.0, 115.0), 0.5, 1.4);
        let faded_clip = place(&mut scene, clipped, Vec2::new(150.0, 50.0), 0.0, 1.0);
        scene.find_mut(faded_clip).unwrap().set_opacity(0.6);
        assert_same(gpu, "instances with opacity and clips", &scene, identity(), Rule::default());
    });
}

/// A group with opacity whose children are instances: the layer must be as big as the instances are, and a group of instances
/// that do not touch needs none.
#[test]
fn a_faded_group_of_instances_gets_a_layer_where_they_overlap_and_none_where_they_do_not() {
    Gpu::for_each_test_mode(|gpu| {
        let build = |positions: &[(f32, f32)]| {
            let mut scene = Scene::default();
            add(&mut scene, shape(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(200.0, 160.0)), Some(Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Color::rgb(250, 230, 60), Color::rgb(30, 60, 200))), None));
            let (symbol, first) = symbol_of(&mut scene, two_parts());
            let mut ids = vec![first];
            ids.extend(positions.iter().skip(1).map(|&(x, y)| place(&mut scene, symbol, Vec2::new(x, y), 0.0, 1.0)));
            scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(positions[0].0, positions[0].1)));
            let group = scene.group(&ids).expect("a group");
            scene.find_mut(group).unwrap().set_opacity(0.5);
            scene
        };
        let overlapping = build(&[(60.0, 70.0), (85.0, 80.0), (150.0, 40.0)]);
        assert_same(gpu, "faded group of overlapping instances", &overlapping, identity(), Rule::default());
        assert_eq!(stats_of(gpu, &overlapping, identity()).layers, 1);
        let apart = build(&[(30.0, 30.0), (100.0, 100.0), (165.0, 40.0)]);
        assert_same(gpu, "faded group of instances far apart", &apart, identity(), Rule::default());
        // (the symbol's second part is see-through with a stroke, so each instance is a whole of its own: its own layer, but the
        // group needs none)
        let stats = stats_of(gpu, &apart, identity());
        assert!(stats.layers <= 3, "{stats:?}");
    });
}

/// The same art at sizes that differ by a factor of 16, with the view zoomed: the circles of the big one and the small one are
/// both smooth, and the art is not made once per instance.
#[test]
fn instances_at_very_different_sizes_are_each_smooth_and_share_their_art() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let (symbol, first) = symbol_of(
            &mut scene,
            vec![shape(Path::ellipse(Vec2::ZERO, Vec2::new(2.0, 1.5)), solid(40, 120, 220, 255), Some(stroke(Color::rgb(0, 0, 0), 0.3, LineJoin::Round, LineCap::Round)))],
        );
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(4.0, 4.0)) * Affine2::from_scale(Vec2::splat(16.0)));
        place(&mut scene, symbol, Vec2::new(8.0, 3.0), 0.0, 1.0);
        place(&mut scene, symbol, Vec2::new(8.0, 5.0), 0.7, 1.0);
        place(&mut scene, symbol, Vec2::new(10.0, 4.0), 0.0, 2.0);
        let view = Affine2::from_scale(Vec2::splat(6.0));
        assert_same(gpu, "instances at different sizes", &scene, view, Rule::default());
        let stats = stats_of(gpu, &scene, view);
        assert_eq!(stats.items, 4 * 2);
        assert!(stats.shapes_tessellated <= 3, "the art in the two buckets the sizes need, not once per instance: {stats:?}");
    });
}

/// A symbol that contains an instance of itself (the engine never makes one, but a damaged file can hold one) must not make
/// the renderer go on for ever; the CPU renderer is not asked to draw it.
#[test]
fn a_symbol_that_contains_itself_does_not_hang_the_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let (symbol, first) = symbol_of(&mut scene, two_parts());
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(100.0, 80.0)));
        let again = scene.new_node("again", NodeKind::Instance(evolve_engine::symbol::Instance { symbol, fill: None, stroke: None }));
        if let NodeKind::Group(children) = &mut scene.symbols[0].root.kind {
            children.push(again);
        }
        assert!(scene.has_symbol_cycle());
        assert!(crate::renderer::Renderer::unsupported(&scene).is_empty());
        let stats = stats_of(gpu, &scene, identity());
        assert!(stats.items >= 4, "what is before the loop is drawn: {stats:?}");
    });
}

/// Instances out of the view cost nothing, and a symbol that was deleted leaves its instances drawing nothing.
#[test]
fn instances_outside_the_view_are_not_drawn_and_an_instance_of_a_deleted_symbol_draws_nothing() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let (symbol, first) = symbol_of(&mut scene, two_parts());
        scene.find_mut(first).unwrap().set_transform(Affine2::from_translation(Vec2::new(50.0, 50.0)));
        for i in 0..200 {
            place(&mut scene, symbol, Vec2::new(5000.0 + i as f32 * 60.0, 50.0), 0.0, 1.0);
        }
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!(stats.items, 4, "only the instance in the view: {stats:?}");
        assert!(stats.shapes_culled >= 2 * 100, "the others were culled: {stats:?}");
        assert_same(gpu, "one in view, many out of it", &scene, identity(), Rule::default());
        scene.symbols.clear(); // (what a damaged file looks like: the engine never deletes a symbol that is still placed)
        let stats = stats_of(gpu, &scene, identity());
        assert_eq!(stats.items, 0, "{stats:?}");
    });
}
