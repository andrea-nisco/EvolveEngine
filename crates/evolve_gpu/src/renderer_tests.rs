//! What the renderer does between frames: what it uploads, what it rebuilds, what it refuses, and what it gives back.

use crate::alloc::Category;
use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::draw_list::{Unsupported, CHUNK_ITEMS};
use crate::renderer::{FrameStats, RenderError, RenderOptions, Renderer};
use crate::targets::TargetPolicy;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, LineCap, LineJoin, Paint, Stroke};
use evolve_engine::Vec2;

const SIZE: (u32, u32) = (320, 240);

fn options() -> RenderOptions {
    RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() }
}

fn rect_shape(x: f32, y: f32) -> Shape {
    Shape {
        path: Path::rect(Vec2::new(x, y), Vec2::new(x + 6.0, y + 4.0)),
        fill: Some(Paint::Solid(Color::rgb(200, 40, 40))),
        stroke: Some(Stroke { paint: Paint::Solid(Color::rgb(0, 0, 0)), width: 1.0, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 }),
        fill_rule: FillRule::NonZero,
        skin: None,
    }
}

/// `count` small shapes in a grid, each a fill and a stroke (two draw items).
fn grid(count: usize) -> (Scene, Vec<NodeId>) {
    let mut scene = Scene::default();
    let ids = (0..count)
        .map(|i| {
            let node = scene.new_node(format!("r{i}"), NodeKind::Shape(rect_shape((i % 40) as f32 * 8.0, (i / 40) as f32 * 6.0)));
            scene.add(node, None)
        })
        .collect();
    (scene, ids)
}

fn render(renderer: &mut Renderer, scene: &Scene) -> FrameStats {
    renderer.render(scene, Affine2::IDENTITY, SIZE, &options()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_first_frame_builds_everything_and_the_second_uploads_nothing() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(100);
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = render(&mut renderer, &scene);
        assert_eq!((first.items, first.shapes_tessellated), (200, 100));
        assert!(first.table_bytes_uploaded > 0 && first.geometry_bytes_uploaded > 0 && first.chunks_rebuilt == 1);
        assert_eq!(first.draw_calls, 1, "200 items fit in one chunk");
        let second = render(&mut renderer, &scene);
        assert_eq!((second.shapes_tessellated, second.chunks_rebuilt, second.vertices_uploaded), (0, 0, 0));
        assert_eq!((second.table_bytes_uploaded, second.geometry_bytes_uploaded), (0, 0), "a frame where nothing changed uploads nothing");
        assert!(second.reused_frame, "nothing was walked: the last frame was used as it was");
    });
}

#[test]
fn moving_a_shape_rewrites_one_line_of_the_table_and_nothing_else() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(100);
        let mut renderer = Renderer::new(gpu).unwrap();
        render(&mut renderer, &scene);
        scene.find_mut(ids[37]).unwrap().set_transform(Affine2::from_translation(Vec2::new(30.0, 20.0)));
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.table_bytes_uploaded, 16 * 1024, "one line of 256 rows");
        assert_eq!((stats.geometry_bytes_uploaded, stats.chunks_rebuilt, stats.shapes_tessellated), (0, 0, 0));
        // Recoloring and the same for a group's transform: rows only.
        scene.find_mut(ids[3]).unwrap().set_solid_fill_color(Color::rgb(1, 2, 3));
        let stats = render(&mut renderer, &scene);
        assert_eq!((stats.geometry_bytes_uploaded, stats.chunks_rebuilt), (0, 0));
        assert!(stats.table_bytes_uploaded <= 16 * 1024);
    });
}

#[test]
fn editing_one_shape_uploads_its_vertices_and_rebuilds_only_its_chunk() {
    Gpu::for_each_test_mode(|gpu| {
        // 1 200 shapes: 2 400 items: five chunks of 512.
        let (mut scene, ids) = grid(1200);
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = render(&mut renderer, &scene);
        assert_eq!(first.items, 2400);
        assert_eq!(first.draw_calls, 2400_usize.div_ceil(CHUNK_ITEMS), "five draw calls for 2 400 items");
        assert_eq!(first.chunks_rebuilt, 5);

        // Shape 700 is items 1400 and 1401: in chunk 2. Its path changes, so its geometry stamp changes.
        scene.find_mut(ids[700]).unwrap().path_mut().unwrap().subpaths[0].anchors[2].pos = Vec2::new(40.0, 40.0);
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.shapes_tessellated, 1);
        assert_eq!(stats.chunks_rebuilt, 1, "only the chunk that holds the shape");
        assert!(stats.vertices_uploaded > 0 && stats.vertices_uploaded < 40, "just this shape's vertices: {}", stats.vertices_uploaded);
        assert!(stats.geometry_bytes_uploaded < 512 * 24 * 4 + 512 * 3 * 4 * 4, "a small part of the geometry: {}", stats.geometry_bytes_uploaded);
        assert_eq!(stats.draw_calls, 5);
    });
}

#[test]
fn adding_and_removing_shapes_changes_the_items_and_gives_rows_and_vertices_back() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(300);
        let mut renderer = Renderer::new(gpu).unwrap();
        render(&mut renderer, &scene);
        let geometry_before = gpu.alloc.used(Category::Geometry);
        let rows_before = renderer.table().rows_in_use();
        assert_eq!(rows_before, 600);

        for &id in &ids[100..200] {
            scene.remove(id);
        }
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.items, 400);
        assert_eq!(renderer.table().rows_in_use(), 400, "the rows of the removed shapes were freed");
        // New shapes reuse those rows and the freed vertex ranges.
        for i in 0..100 {
            let node = scene.new_node("new", NodeKind::Shape(rect_shape(i as f32, 100.0)));
            scene.add(node, None);
        }
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.items, 600);
        assert_eq!(renderer.table().rows_in_use(), 600);
        assert_eq!(renderer.table().capacity_rows(), 16 * crate::table::ROWS_PER_LINE, "the table did not need to grow");
        assert!(gpu.alloc.used(Category::Geometry) <= geometry_before, "freed ranges were reused, not added to");
    });
}

#[test]
fn a_scene_with_things_the_gpu_cannot_draw_yet_is_refused_without_changing_anything() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(20);
        // A clip group whose mask is skinned (gradients, clip groups, opacity, symbol instances and skinned shapes are drawn now, and are
        // not in the list).
        scene.find_mut(ids[0]).unwrap().kind = NodeKind::Shape(Shape { fill: Some(Paint::linear(Vec2::ZERO, Vec2::X, Color::BLACK, Color::WHITE)), ..rect_shape(0.0, 0.0) });
        let first = scene.new_node("first", NodeKind::Shape(rect_shape(0.0, 0.0)));
        let second = scene.new_node("second", NodeKind::Shape(rect_shape(3.0, 3.0)));
        let mut faded = scene.new_node("faded", NodeKind::Group(vec![first, second]));
        faded.opacity = 0.5;
        scene.add(faded, None);
        let _ = ids[1];
        let mut mask = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)));
        mask.stroke = None;
        let content = scene.new_node("content", NodeKind::Shape(rect_shape(0.0, 0.0)));
        let mask = scene.new_node("mask", NodeKind::Shape(mask));
        let mut clip = scene.new_node("clip", NodeKind::Group(vec![content, mask]));
        clip.clip = true;
        scene.add(clip, None);
        let mut skinned = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)));
        skinned.skin = Some(Box::new(evolve_engine::rig::Skin { bones: vec![], inverse_bind: vec![], bind_world: Affine2::IDENTITY, weights: vec![] }));
        let content = scene.new_node("content", NodeKind::Shape(rect_shape(1.0, 1.0)));
        let skinned = scene.new_node("skinned mask", NodeKind::Shape(skinned));
        let mut bent_clip = scene.new_node("bent clip", NodeKind::Group(vec![content, skinned]));
        bent_clip.clip = true;
        scene.add(bent_clip, None);

        let gate = Renderer::unsupported(&scene);
        assert_eq!(gate.keys().copied().collect::<Vec<_>>(), [Unsupported::Skinned]);

        let mut renderer = Renderer::new(gpu).unwrap();
        let before = gpu.alloc.snapshot();
        match renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()) {
            Err(RenderError::Unsupported(what)) => assert_eq!(what, gate, "the error says the same as the gate"),
            other => panic!("expected a refusal, got {:?}", other.map(|s| s.items)),
        }
        assert_eq!(renderer.table().rows_in_use(), 0, "nothing was drawn or set up");
        assert!(renderer.picture().is_none());
        let _ = before;

        // For timing only: draw what can be drawn and say what was left out.
        let lenient = RenderOptions { skip_unsupported: true, ..options() };
        let stats = renderer.render(&scene, Affine2::IDENTITY, SIZE, &lenient).unwrap();
        assert_eq!(stats.skipped, gate);
        assert!(stats.items > 0);
    });
}

#[test]
fn a_clean_scene_passes_the_gate() {
    let (scene, _) = grid(5);
    assert!(Renderer::unsupported(&scene).is_empty());
    assert!(Renderer::unsupported(&Scene::with_camera(Vec2::new(100.0, 100.0))).is_empty());
}

#[test]
fn an_item_count_or_a_scene_that_would_not_fit_is_an_error_not_a_hang() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(2000);
        let mut renderer = Renderer::new(gpu).unwrap();
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Geometry, 1_000_000));
        let error = renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).expect_err("the geometry does not fit in 1 MB");
        assert!(matches!(error, RenderError::Alloc(crate::alloc::AllocError::OverBudget { category: Category::Geometry, .. })), "{error}");
        gpu.alloc.set_budgets(crate::alloc::Budgets::high());
    });
}

#[test]
fn all_memory_is_given_back_when_the_renderer_is_dropped_and_across_many_frames_and_resizes() {
    Gpu::for_each_test_mode(|gpu| {
        let before = gpu.alloc.snapshot();
        {
            let (mut scene, ids) = grid(600);
            let mut renderer = Renderer::new(gpu).unwrap();
            let policy = TargetPolicy::default();
            let mut peak = 0;
            for frame in 0..60u32 {
                // The view changes size, the scene is edited, shapes come and go.
                let size = (200 + (frame * 13) % 150, 150 + (frame * 7) % 100);
                scene.find_mut(ids[(frame as usize * 11) % 600]).unwrap().set_transform(Affine2::from_translation(Vec2::new(frame as f32, 0.0)));
                if frame % 9 == 0 {
                    scene.find_mut(ids[(frame as usize * 5) % 600]).unwrap().path_mut().unwrap().subpaths[0].anchors[1].pos.x += 3.0;
                }
                if frame % 10 == 0 {
                    let node = scene.new_node("n", NodeKind::Shape(rect_shape(frame as f32, 150.0)));
                    scene.add(node, None);
                }
                let view = Affine2::from_scale(Vec2::splat(1.0 + (frame % 5) as f32));
                renderer.render(&scene, view, size, &RenderOptions { targets: policy, ..options() }).unwrap();
                peak = peak.max(gpu.alloc.total_used());
            }
            // After a warm-up the totals stop growing: the same state again gives the same bytes.
            let steady = gpu.alloc.total_used();
            for frame in 0..60u32 {
                renderer.render(&scene, Affine2::IDENTITY, (200 + (frame * 13) % 150, 150 + (frame * 7) % 100), &RenderOptions { targets: policy, ..options() }).unwrap();
            }
            assert!(gpu.alloc.total_used() <= peak.max(steady), "the totals do not creep up over many frames");
        }
        assert_eq!(gpu.alloc.snapshot(), before, "dropping the renderer gave back every byte (rule C4)");
    });
}

#[test]
fn a_lost_device_makes_the_renderer_say_so_and_not_draw() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(5);
        let mut renderer = Renderer::new(gpu).unwrap();
        render(&mut renderer, &scene);
        gpu.errors.record_loss("simulated driver reset".into());
        match renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()) {
            Err(RenderError::DeviceLost(why)) => assert!(why.contains("simulated")),
            other => panic!("expected DeviceLost, got {:?}", other.map(|s| s.items)),
        }
        // (The shared test device is fine; take the pretend loss back so that other tests are not told about it.)
        gpu.errors.clear_loss();
    });
}

#[test]
fn depth_values_keep_painters_order_for_many_items() {
    // 3 000 overlapping rectangles, each a bit different: the one drawn last is on top, however many there are.
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        for i in 0..3000u32 {
            let shade = (i % 250) as u8;
            let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors: [(10.0, 10.0), (60.0, 10.0), (60.0, 60.0), (10.0, 60.0)].map(|(x, y)| Anchor::corner(Vec2::new(x, y))).to_vec(), closed: true }] });
            shape.fill = Some(Paint::Solid(Color::rgb(shade, shade, 255 - shade)));
            shape.stroke = None;
            let node = scene.new_node("layer", NodeKind::Shape(shape));
            scene.add(node, None);
        }
        let mut renderer = Renderer::new(gpu).unwrap();
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.draw_calls, 3000_usize.div_ceil(CHUNK_ITEMS));
        let picture = gpu.read_texture_part(renderer.picture().unwrap(), 0, 0, SIZE.0, SIZE.1);
        let at = ((30 * SIZE.0 + 30) * 4) as usize;
        let last_shade = (2999 % 250) as u8;
        assert_eq!(&picture[at..at + 4], &[last_shade, last_shade, 255 - last_shade, 255], "the last rectangle is on top");
    });
}

#[test]
fn the_watchdog_names_the_view_and_the_scene_and_says_what_was_slow() {
    use crate::renderer::watchdog_message;
    assert_eq!(watchdog_message("Scene view", "Main", 5.0, Some(8.0)), None, "a quick frame says nothing");
    assert_eq!(watchdog_message("Scene view", "Main", 100.0, Some(100.0)), None, "exactly at the limit is still fine");
    assert_eq!(watchdog_message("Scene view", "Main", 5.0, None), None, "no report from the card yet");
    let slow_cpu = watchdog_message("Game view", "Level 3", 250.0, Some(3.0)).expect("slow on the CPU");
    assert!(slow_cpu.contains("Game view") && slow_cpu.contains("Level 3") && slow_cpu.contains("250 ms of CPU") && !slow_cpu.contains("card"), "{slow_cpu}");
    let slow_card = watchdog_message("Scene view", "Main", 4.0, Some(180.0)).expect("slow on the card");
    assert!(slow_card.contains("180 ms until the card was done") && !slow_card.contains("CPU"), "{slow_card}");
    let both = watchdog_message("Scene view", "Main", 120.0, Some(150.0)).unwrap();
    assert!(both.contains("CPU") && both.contains("card") && both.contains("and"), "{both}");
}

#[test]
fn the_card_reports_when_a_frame_is_done_without_anyone_waiting_for_it() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(50);
        let mut renderer = Renderer::new(gpu).unwrap();
        assert_eq!(renderer.last_completion_ms(), None);
        render(&mut renderer, &scene);
        gpu.wait_idle(); // only so that the test does not have to wait for the callback
        let ms = renderer.last_completion_ms().expect("the card reported the frame done");
        assert!((0.0..1000.0).contains(&ms), "{ms}");
    });
}

// --- Culling: what cannot reach the view is neither made nor drawn ------------------------------------------------------------

fn without_culling() -> RenderOptions {
    RenderOptions { cull: false, ..options() }
}

/// Every pixel of the last picture.
fn picture_bytes(gpu: &Gpu, renderer: &Renderer) -> Vec<u8> {
    gpu.read_picture(renderer.picture().expect("a picture"))
}

#[test]
fn culling_leaves_the_picture_as_it_was_and_makes_far_fewer_shapes() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(1200); // 320 x 180 units, 2 400 draw items
        for (zoom, at) in [(6.0, (0.0, 0.0)), (6.0, (130.0, 70.0)), (10.0, (310.0, 170.0)), (3.0, (-20.0, -10.0)), (1.0, (0.0, 0.0))] {
            let view = Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(Vec2::new(-at.0, -at.1));
            let mut culled = Renderer::new(gpu).unwrap();
            let mut full = Renderer::new(gpu).unwrap();
            let with = culled.render(&scene, view, SIZE, &options()).unwrap();
            let without = full.render(&scene, view, SIZE, &without_culling()).unwrap();
            assert_eq!(picture_bytes(gpu, &culled), picture_bytes(gpu, &full), "zoom {zoom} at {at:?}: culling changed the picture");
            assert_eq!(without.shapes_culled, 0);
            assert_eq!(with.shapes_culled + with.shapes_tessellated, 1200, "every shape was either made or culled");
            if zoom >= 6.0 {
                assert!(with.shapes_culled > 900, "zoom {zoom} at {at:?}: only {} of 1200 culled", with.shapes_culled);
                assert!(with.items < without.items / 3, "{} of {} items", with.items, without.items);
                assert!(with.shapes_tessellated < 300);
            } else if zoom >= 3.0 {
                assert!(with.shapes_culled > 500, "zoom {zoom} at {at:?}: only {} of 1200 culled", with.shapes_culled);
            }
        }
    });
}

#[test]
fn a_wide_stroke_from_outside_the_view_is_still_drawn() {
    Gpu::for_each_test_mode(|gpu| {
        // Thick lines with miter joins and square caps, each starting far to the left of where its stroke reaches.
        let mut scene = Scene::default();
        for i in 0..40 {
            let x = -600.0 - i as f32 * 20.0;
            let y = 20.0 + i as f32 * 4.0;
            let mut shape = Shape::new(Path::rect(Vec2::new(x, y), Vec2::new(x + 40.0, y + 10.0)));
            shape.fill = Some(Paint::Solid(Color::rgb(30, 90, 200)));
            shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgba(0, 0, 0, 160)), width: 20.0 + i as f32 * 30.0, join: LineJoin::Miter, cap: LineCap::Square, miter_limit: 4.0 });
            let node = scene.new_node(format!("w{i}"), NodeKind::Shape(shape));
            scene.add(node, None);
        }
        let mut seen_culled = false;
        for shift in [0.0, 150.0, 300.0, 450.0, 600.0, 800.0, 1100.0] {
            // The scene is to the left of the view: look at it from further and further left.
            let view = Affine2::from_translation(Vec2::new(shift, 0.0));
            let mut culled = Renderer::new(gpu).unwrap();
            let mut full = Renderer::new(gpu).unwrap();
            let stats = culled.render(&scene, view, SIZE, &options()).unwrap();
            full.render(&scene, view, SIZE, &without_culling()).unwrap();
            assert_eq!(picture_bytes(gpu, &culled), picture_bytes(gpu, &full), "shift {shift}: culling hid part of a wide stroke");
            seen_culled |= stats.shapes_culled > 0;
        }
        assert!(seen_culled, "the test never culled anything, so it proves nothing");
    });
}

#[test]
fn panning_makes_only_the_shapes_that_come_near_and_forgets_those_that_leave() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(1200);
        let mut renderer = Renderer::new(gpu).unwrap();
        let at = |x: f32| Affine2::from_scale(Vec2::splat(8.0)) * Affine2::from_translation(Vec2::new(-x, 0.0));
        let first = renderer.render(&scene, at(0.0), SIZE, &options()).unwrap();
        let kept = renderer.cache().len();
        assert!(kept < 300 && first.shapes_tessellated == kept);
        // The same place again: nothing to make.
        let again = renderer.render(&scene, at(0.0), SIZE, &options()).unwrap();
        assert_eq!((again.shapes_tessellated, again.geometry_bytes_uploaded), (0, 0));
        // A small pan inside the margin: everything that can show was made, so the last frame is used as it is.
        let inside = renderer.render(&scene, at(8.0), SIZE, &options()).unwrap();
        assert!(inside.reused_frame && inside.shapes_tessellated == 0, "{inside:?}");
        // A pan past the margin: a few new shapes, not a rebuild.
        let small = renderer.render(&scene, at(20.0), SIZE, &options()).unwrap();
        assert!(small.shapes_tessellated > 0 && small.shapes_tessellated < kept / 2, "{} new of {kept}", small.shapes_tessellated);
        // Far to the right: all different, and the old ones are forgotten.
        let far = renderer.render(&scene, at(280.0), SIZE, &options()).unwrap();
        assert!(far.shapes_tessellated > 0);
        assert!(renderer.cache().len() < 300, "the cache holds {} shapes: what left was not forgotten", renderer.cache().len());
        // Back where it started: made again, and the picture is the same as a first look from there.
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, at(0.0), SIZE, &options()).unwrap();
        renderer.render(&scene, at(0.0), SIZE, &options()).unwrap();
        assert_eq!(picture_bytes(gpu, &renderer), picture_bytes(gpu, &fresh));
    });
}

#[test]
fn zooming_far_into_a_big_scene_stays_inside_the_geometry_budget_because_only_what_is_near_is_made() {
    Gpu::for_each_test_mode(|gpu| {
        // 1 600 circles with round joins: their triangles depend on the zoom, and they get many more at a fine bucket.
        let mut scene = Scene::default();
        for i in 0..1600 {
            let mut shape = Shape::new(Path::ellipse(Vec2::new((i % 40) as f32 * 10.0 + 5.0, (i / 40) as f32 * 10.0 + 5.0), Vec2::splat(4.0)));
            shape.fill = Some(Paint::Solid(Color::rgb(200, 90, 40)));
            shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgb(0, 0, 0)), width: 1.0, join: LineJoin::Round, cap: LineCap::Round, miter_limit: 4.0 });
            let node = scene.new_node(format!("c{i}"), NodeKind::Shape(shape));
            scene.add(node, None);
        }
        let zoomed = Affine2::from_scale(Vec2::splat(16.0));

        // What everything at this zoom costs, with no culling: that is what the editor tried to do.
        let everything = {
            let mut renderer = Renderer::new(gpu).unwrap();
            renderer.render(&scene, zoomed, SIZE, &without_culling()).unwrap();
            gpu.alloc.used(Category::Geometry)
        };
        let near = {
            let mut renderer = Renderer::new(gpu).unwrap();
            renderer.render(&scene, zoomed, SIZE, &options()).unwrap();
            gpu.alloc.used(Category::Geometry)
        };
        assert!(near * 8 < everything, "culling should make the geometry several times smaller: {near} against {everything} bytes");

        // A budget that holds what is near but not everything: with culling the view still draws, without it it is refused.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Geometry, everything / 2));
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, zoomed, SIZE, &options()).expect("the near shapes fit the budget");
        drop(renderer);
        let mut renderer = Renderer::new(gpu).unwrap();
        let refused = renderer.render(&scene, zoomed, SIZE, &without_culling()).expect_err("all of it does not");
        assert!(matches!(refused, RenderError::Alloc(_)), "{refused}");
    });
}

/// 1 600 circles with round joins: their triangles depend on the zoom, so every change of zoom bucket makes all of them
/// again. Larger circles get many more vertices at a fine zoom.
fn circles(radius: f32) -> Scene {
    let mut scene = Scene::default();
    let spacing = radius * 2.5;
    for i in 0..1600 {
        let mut shape = Shape::new(Path::ellipse(Vec2::new((i % 40) as f32 * spacing + spacing / 2.0, (i / 40) as f32 * spacing + spacing / 2.0), Vec2::splat(radius)));
        shape.fill = Some(Paint::Solid(Color::rgb(200, 90, 40)));
        shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgb(0, 0, 0)), width: 1.0, join: LineJoin::Round, cap: LineCap::Round, miter_limit: 4.0 });
        let node = scene.new_node(format!("c{i}"), NodeKind::Shape(shape));
        scene.add(node, None);
    }
    scene
}

/// The most geometry memory (counting a buffer and its larger replacement together) that drawing `scene` at `zoom` takes.
fn geometry_needed(gpu: &Gpu, scene: &Scene, zoom: f32) -> u64 {
    gpu.alloc.reset_peaks();
    let mut renderer = Renderer::new(gpu).unwrap();
    renderer.render(scene, Affine2::from_scale(Vec2::splat(zoom)), SIZE, &without_culling()).unwrap();
    gpu.alloc.peak(Category::Geometry)
}

#[test]
fn a_change_of_zoom_replaces_the_geometry_without_needing_the_old_and_the_new_at_once() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(4.0);
        // A budget of one and a half times what the finest zoom needs: not enough to hold two sets at once.
        let budget = geometry_needed(gpu, &scene, 16.0) * 3 / 2;
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Geometry, budget));
        let mut renderer = Renderer::new(gpu).unwrap();
        for zoom in [1.0, 16.0, 1.0, 16.0, 4.0, 0.5, 16.0] {
            renderer.render(&scene, Affine2::from_scale(Vec2::splat(zoom)), SIZE, &without_culling()).unwrap_or_else(|e| panic!("zoom {zoom}: {e}"));
        }
        let after_changes = picture_bytes(gpu, &renderer);
        drop(renderer);
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, Affine2::from_scale(Vec2::splat(16.0)), SIZE, &without_culling()).unwrap();
        assert_eq!(after_changes, picture_bytes(gpu, &fresh), "after all those changes of zoom the picture is the one a new renderer draws");
    });
}

#[test]
fn a_request_that_fails_half_way_leaves_nothing_wrong_behind_for_the_next_frame() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(40.0);
        let coarse = geometry_needed(gpu, &scene, 1.0);
        let fine = geometry_needed(gpu, &scene, 16.0);
        assert!(fine > coarse + coarse / 4, "the finest zoom must need clearly more than the coarsest for this test: {fine} against {coarse}");
        // Room for the coarse zoom (and what its buffers need while they grow), but not for the fine one.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Geometry, coarse + coarse / 40));
        let mut renderer = Renderer::new(gpu).unwrap();
        let at = |zoom: f32| Affine2::from_scale(Vec2::splat(zoom));
        renderer.render(&scene, at(1.0), SIZE, &without_culling()).unwrap();
        let vertex_capacity = renderer.store().vertices.capacity();
        let refused = renderer.render(&scene, at(16.0), SIZE, &without_culling()).expect_err("the fine zoom does not fit");
        assert!(matches!(refused, RenderError::Alloc(_)), "{refused}");
        // Back at the coarse zoom: the renderer must not use ranges it gave back while failing.
        renderer.render(&scene, at(1.0), SIZE, &without_culling()).unwrap();
        let after_failure = picture_bytes(gpu, &renderer);
        drop(renderer);
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, at(1.0), SIZE, &without_culling()).unwrap();
        assert_eq!(fresh.store().vertices.capacity(), vertex_capacity, "(the same scene at the same zoom needs the same room)");
        assert_eq!(after_failure, picture_bytes(gpu, &fresh), "the picture after a failed frame is a correct one");
    });
}

#[test]
fn shapes_that_stay_and_shapes_that_are_made_again_leave_no_holes_behind() {
    Gpu::for_each_test_mode(|gpu| {
        // Circles (made again at every zoom bucket) in the middle of the drawing order, between straight-sided boxes (never):
        // after each change of zoom the boxes sit on both sides of the ranges that were given back.
        let mut scene = Scene::default();
        for i in 0..1200 {
            let at = Vec2::new((i % 40) as f32 * 30.0, (i / 40) as f32 * 30.0);
            let circle = (300..1000).contains(&i);
            // (circles of different sizes, so that what they need changes by different amounts with the zoom)
            let path = if circle { Path::ellipse(at + Vec2::splat(12.0), Vec2::splat(3.0 + (i % 9) as f32 * 1.2)) } else { Path::rect(at, at + Vec2::splat(20.0)) };
            let mut shape = Shape::new(path);
            shape.fill = Some(Paint::Solid(Color::rgb(40, 120, 200)));
            let join = if circle { LineJoin::Round } else { LineJoin::Miter }; // a miter join does not depend on the zoom
            shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgb(0, 0, 0)), width: 1.0, join, cap: LineCap::Butt, miter_limit: 4.0 });
            let node = scene.new_node(format!("m{i}"), NodeKind::Shape(shape));
            scene.add(node, None);
        }
        let mut renderer = Renderer::new(gpu).unwrap();
        let (mut vertices_packed, mut indices_packed) = (false, false);
        for zoom in [1.0f32, 16.0, 1.0, 16.0, 4.0, 1.0] {
            let stats = renderer.render(&scene, Affine2::from_scale(Vec2::splat(zoom)), SIZE, &without_culling()).unwrap_or_else(|e| panic!("zoom {zoom}: {e}"));
            vertices_packed |= stats.vertices_compacted;
            indices_packed |= stats.indices_compacted;
            let (vertices, indices) = (renderer.store().vertices.ranges(), renderer.store().indices.ranges());
            assert!(vertices.fragmentation() <= 0.3, "zoom {zoom}: {:.0}% of the vertex buffer is holes", vertices.fragmentation() * 100.0);
            assert!(indices.fragmentation() <= 0.3, "zoom {zoom}: {:.0}% of the index buffer is holes", indices.fragmentation() * 100.0);
        }
        assert!(vertices_packed, "the vertex buffer was never packed, so the test proves nothing about it");
        assert!(indices_packed, "the index buffer was never packed, so the test proves nothing about it");
        let after_changes = picture_bytes(gpu, &renderer);
        drop(renderer);
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, Affine2::from_scale(Vec2::splat(1.0)), SIZE, &without_culling()).unwrap();
        assert_eq!(after_changes, picture_bytes(gpu, &fresh), "the picture after all those changes is the one a new renderer draws");
    });
}

/// Zooming and panning with one renderer draws, at every step, exactly what a new renderer draws there: nothing that
/// happens between frames (shapes culled and made again, circles made again at every zoom bucket next to boxes that never
/// are, packing) may change the picture. The zooms are in the middle of their buckets, so the 10% the renderer keeps a
/// bucket past its edge plays no part. (The vanishing provinces of 2026-10-16 are pinned down by the next test.)
#[test]
fn every_frame_of_a_zoom_and_pan_is_the_picture_a_new_renderer_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        for i in 0..1200 {
            let at = Vec2::new((i % 40) as f32 * 10.0, (i / 40) as f32 * 10.0);
            let circle = i % 2 == 0;
            let path = if circle { Path::ellipse(at + Vec2::splat(4.0), Vec2::splat(2.0 + (i % 7) as f32 * 0.3)) } else { Path::rect(at, at + Vec2::splat(7.0)) };
            let mut shape = Shape::new(path);
            shape.fill = Some(Paint::Solid(Color::rgb(40 + (i % 5) as u8 * 40, 120, 200)));
            let join = if circle { LineJoin::Round } else { LineJoin::Miter };
            shape.stroke = Some(Stroke { paint: Paint::Solid(Color::rgb(0, 0, 0)), width: 0.8, join, cap: LineCap::Butt, miter_limit: 4.0 });
            let node = scene.new_node(format!("z{i}"), NodeKind::Shape(shape));
            scene.add(node, None);
        }
        let view = |zoom: f32, x: f32, y: f32| {
            Affine2::from_translation(Vec2::new(SIZE.0 as f32 * 0.5, SIZE.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(Vec2::new(-x, -y))
        };
        let steps = [(0.75, 200.0, 150.0), (1.5, 200.0, 150.0), (3.0, 180.0, 140.0), (6.0, 170.0, 130.0), (3.0, 200.0, 150.0), (1.5, 120.0, 100.0), (3.0, 250.0, 170.0), (0.75, 200.0, 150.0), (6.0, 60.0, 40.0), (1.5, 200.0, 150.0)];
        let mut renderer = Renderer::new(gpu).unwrap();
        let mut reused = false;
        for (zoom, x, y) in steps {
            let stats = renderer.render(&scene, view(zoom, x, y), SIZE, &options()).unwrap_or_else(|e| panic!("zoom {zoom}: {e}"));
            reused |= stats.shapes_reused > 0 && stats.shapes_tessellated > 0;
            let mut fresh = Renderer::new(gpu).unwrap();
            fresh.render(&scene, view(zoom, x, y), SIZE, &options()).unwrap();
            assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "zoom {zoom} at ({x}, {y}): the renderer that has been zooming draws something else than a new one");
        }
        assert!(reused, "no frame both kept shapes and made new ones, so the test proves nothing");
    });
}

/// A quarter of the shapes leave the view and come back (here by being hidden; culling does the same while zooming and
/// panning): the shapes that come back take their old places in the buffer, with one shape that stayed between two of them
/// (shapes 1 and 3 of every 8, fills only: one block of 64 vertices apart, which the old joining of writes filled with
/// zeros). Those must be left as they were.
#[test]
fn shapes_that_come_back_between_shapes_that_stayed_leave_those_alone() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(1200);
        for &id in &ids {
            if let NodeKind::Shape(shape) = &mut scene.find_mut(id).unwrap().kind {
                shape.stroke = None;
            }
        }
        let leaving: Vec<NodeId> = ids.iter().enumerate().filter(|(n, _)| n % 8 == 1 || n % 8 == 3).map(|(_, id)| *id).collect();
        let mut renderer = Renderer::new(gpu).unwrap();
        render(&mut renderer, &scene);
        for &id in &leaving {
            scene.find_mut(id).unwrap().set_visible(false);
        }
        let hidden = render(&mut renderer, &scene);
        assert!(!hidden.vertices_compacted, "a quarter of the buffer is holes: below the packing threshold, so the holes stay");
        for &id in &leaving {
            scene.find_mut(id).unwrap().set_visible(true);
        }
        let back = render(&mut renderer, &scene);
        assert!(!back.vertices_compacted && back.shapes_tessellated == 300, "{back:?}");
        let mut fresh = Renderer::new(gpu).unwrap();
        render(&mut fresh, &scene);
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "shapes that stayed were damaged by the ones that came back");
    });
}

// --- Clip groups ---------------------------------------------------------------------------------------------------------------

/// A clip group of `count` small shapes (a grid, 2 items each) and a round mask over the middle of it.
fn clipped_grid(count: usize) -> (Scene, NodeId, NodeId) {
    let mut scene = Scene::default();
    let mut children: Vec<_> = (0..count).map(|i| scene.new_node(format!("r{i}"), NodeKind::Shape(rect_shape((i % 40) as f32 * 8.0, (i / 40) as f32 * 6.0)))).collect();
    let mut mask = Shape::new(Path::ellipse(Vec2::new(100.0, 40.0), Vec2::new(60.0, 30.0)));
    mask.fill = None; // a mask needs no fill: only its silhouette counts
    let mask = scene.new_node("mask", NodeKind::Shape(mask));
    let mask_id = mask.id;
    children.push(mask);
    let mut group = scene.new_node("clip", NodeKind::Group(children));
    group.clip = true;
    let group_id = scene.add(group, None);
    (scene, group_id, mask_id)
}

#[test]
fn a_clip_group_whose_mask_is_far_from_the_view_makes_nothing_of_its_content() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, _, mask) = clipped_grid(400);
        scene.find_mut(mask).unwrap().set_transform(Affine2::from_translation(Vec2::new(100_000.0, 0.0)));
        let mut renderer = Renderer::new(gpu).unwrap();
        let stats = render(&mut renderer, &scene);
        assert_eq!((stats.items, stats.shapes_tessellated), (0, 0), "the content of a clip that cannot show is not made: {stats:?}");
        assert!(stats.shapes_culled >= 1);
        // Back in view: the whole clip is made.
        scene.find_mut(mask).unwrap().set_transform(Affine2::IDENTITY);
        let stats = render(&mut renderer, &scene);
        assert!(stats.items > 700 && stats.shapes_tessellated > 350, "{stats:?}");
    });
}

#[test]
fn a_clip_costs_nothing_on_an_idle_frame_and_moving_the_mask_only_rewrites_table_lines() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, group, mask) = clipped_grid(400);
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = render(&mut renderer, &scene);
        assert!(first.draw_calls >= 3, "a push, the content and a pop: {first:?}");
        let idle = render(&mut renderer, &scene);
        assert_eq!((idle.shapes_tessellated, idle.chunks_rebuilt, idle.geometry_bytes_uploaded, idle.table_bytes_uploaded), (0, 0, 0, 0), "{idle:?}");
        // Moving the mask: its row changes, nothing is made again and no chunk is rebuilt.
        scene.find_mut(mask).unwrap().set_transform(Affine2::from_translation(Vec2::new(30.0, 10.0)));
        let moved = render(&mut renderer, &scene);
        assert_eq!((moved.shapes_tessellated, moved.chunks_rebuilt, moved.geometry_bytes_uploaded), (0, 0, 0), "{moved:?}");
        assert!(moved.table_bytes_uploaded > 0);
        let moved_picture = picture_bytes(gpu, &renderer);
        // Turning the clip off draws all the content whole; turning it on again brings the earlier picture back.
        scene.find_mut(group).unwrap().set_clip(false);
        let off = render(&mut renderer, &scene);
        assert!(off.items > 700, "all 800 items, without the mask: {off:?}");
        let off_picture = picture_bytes(gpu, &renderer);
        assert!(off_picture != moved_picture, "with the clip off more is visible");
        scene.find_mut(group).unwrap().set_clip(true);
        render(&mut renderer, &scene);
        assert!(picture_bytes(gpu, &renderer) == moved_picture, "the same clip, the same picture");
        // And a renderer that never saw the other states draws the same.
        let mut fresh = Renderer::new(gpu).unwrap();
        render(&mut fresh, &scene);
        assert!(picture_bytes(gpu, &fresh) == moved_picture);
    });
}

// --- Frames made from the last one (plan 5.3) ------------------------------------------------------------------------------------

/// A scene of 1200 shapes seen at 8x, where only part of it is in view: the margin around the view is what a pan can use.
fn at(zoom: f32, x: f32, y: f32) -> Affine2 {
    Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(Vec2::new(-x, -y))
}

#[test]
fn a_camera_move_inside_the_margin_is_drawn_from_the_last_frame_and_is_the_picture_a_new_renderer_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(1200);
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = renderer.render(&scene, at(8.0, 0.0, 0.0), SIZE, &options()).unwrap();
        assert!(!first.reused_frame);
        // Sixteen small moves in every direction: each is a frame with nothing to walk, make or send.
        for (i, (x, y)) in [(1.5, 0.0), (3.25, 1.0), (-2.0, 4.5), (6.0, -3.0), (0.5, 0.5), (-5.0, -5.0), (7.0, 7.0), (2.0, -1.0)].into_iter().enumerate() {
            let stats = renderer.render(&scene, at(8.0, x, y), SIZE, &options()).unwrap();
            assert!(stats.reused_frame, "move {i} to ({x}, {y}): {stats:?}");
            assert_eq!((stats.shapes_tessellated, stats.chunks_rebuilt, stats.table_bytes_uploaded, stats.geometry_bytes_uploaded), (0, 0, 0, 0));
            assert_eq!((stats.items, stats.draw_calls), (first.items, first.draw_calls), "it says what it drew");
            let mut fresh = Renderer::new(gpu).unwrap();
            fresh.render(&scene, at(8.0, x, y), SIZE, &options()).unwrap();
            assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "move {i} to ({x}, {y}): not the picture a new renderer draws");
        }
        // And the same view again, which is the same thing with no move.
        assert!(renderer.render(&scene, at(8.0, 2.0, -1.0), SIZE, &options()).unwrap().reused_frame);
    });
}

/// A view of `zoom` pixels per unit with the point (x, y) of the scene in the middle of the picture.
fn centered(zoom: f32, x: f32, y: f32) -> Affine2 {
    Affine2::from_translation(Vec2::new(SIZE.0 as f32 / 2.0, SIZE.1 as f32 / 2.0)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(Vec2::new(-x, -y))
}

/// Two renderers that see the same views of the same scene, one allowed to make a frame from the last one and one not. After every
/// frame the pictures must be the same.
struct Pair<'g> {
    gpu: &'g Gpu,
    reusing: Renderer,
    whole: Renderer,
}

impl<'g> Pair<'g> {
    fn new(gpu: &'g Gpu) -> Pair<'g> {
        let mut whole = Renderer::new(gpu).unwrap();
        whole.set_frame_reuse(false);
        Pair { gpu, reusing: Renderer::new(gpu).unwrap(), whole }
    }

    /// Draws `scene` through `view` with both, checks the pictures, and returns what the reusing renderer says.
    fn frame(&mut self, scene: &Scene, view: Affine2, what: &str) -> FrameStats {
        let stats = self.reusing.render(scene, view, SIZE, &options()).unwrap();
        let reference = self.whole.render(scene, view, SIZE, &options()).unwrap();
        assert!(!reference.reused_frame && reference.patched_items == 0, "{what}: the reference reuses nothing");
        assert!(picture_bytes(self.gpu, &self.reusing) == picture_bytes(self.gpu, &self.whole), "{what}: not the picture a frame made whole draws ({stats:?})");
        stats
    }

    /// The same, but the pictures only have to agree as two correct drawings of the scene do. For scenes with instances of one symbol at
    /// several sizes: which zoom bucket's triangles an instance gets (the shared entry of the symbol's art, or one of its own) depends
    /// on what the cache did before, so a renderer that walks the scene in every frame and one that patched may draw curves with
    /// slightly different triangles, and a few edge pixels differ by a level or two. A wrong row or a wrong index list is far more.
    fn frame_close(&mut self, scene: &Scene, view: Affine2, what: &str) -> FrameStats {
        let stats = self.reusing.render(scene, view, SIZE, &options()).unwrap();
        let reference = self.whole.render(scene, view, SIZE, &options()).unwrap();
        assert!(!reference.reused_frame && reference.patched_items == 0, "{what}: the reference reuses nothing");
        let (a, b) = (picture_bytes(self.gpu, &self.reusing), picture_bytes(self.gpu, &self.whole));
        let result = compare_pictures(&a, &b, SIZE.0, Rule { interior_limit: 2, edge_limit: 40, ..Rule::default() });
        assert!(result.passes(), "{what}: not a picture of the same scene as the frame made whole ({result}) ({stats:?})");
        stats
    }
}

/// Circles with round joins: their triangles depend on the zoom bucket. The windows of the buckets are what decide how far the zoom may
/// change: a circle drawn at 4 pixels per unit is in bucket 2, which holds for scales from 1.8 to 4.4.
#[test]
fn a_change_of_zoom_that_no_shape_minds_is_drawn_from_the_last_frame_and_is_the_picture_a_frame_made_whole_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(4.0);
        let mut pair = Pair::new(gpu);
        // (zoom, whether the frame is made from the last one that was made whole); a window is counted from that last whole frame
        let steps = [
            (4.0, false),
            (4.2, true),
            (3.4, true),
            (3.9, true),
            (4.6, false), // 1.15 times 4: past 4.4, circles are made for bucket 3 now, which holds from 3.6 to 8.8
            (4.8, true),
            (5.2, true),
            (4.5, true),
            (9.5, false), // 2.07 times 4.6: past the range, and past 8.8
            (9.0, true),
        ];
        for (zoom, reused) in steps {
            let stats = pair.frame(&scene, centered(zoom, 205.0, 205.0), &format!("zoom {zoom}"));
            assert_eq!(stats.reused_frame, reused, "zoom {zoom}: {stats:?}");
            if reused {
                assert_eq!((stats.shapes_tessellated, stats.chunks_rebuilt, stats.table_bytes_uploaded, stats.geometry_bytes_uploaded), (0, 0, 0, 0), "zoom {zoom}");
            }
        }
    });
}

/// Straight-sided shapes have no bucket at all: only the area that was kept and the largest change of 2 decide.
#[test]
fn a_zoom_out_is_drawn_from_the_last_frame_only_while_the_view_stays_inside_the_area_that_was_kept() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(1200);
        // Inside the area that was kept, but not by the few pixels of inset: 160 / 0.53 = 302 is within 304 of the middle.
        let mut pair = Pair::new(gpu);
        assert!(!pair.frame(&scene, centered(8.0, 160.0, 90.0), "first").reused_frame);
        assert!(!pair.frame(&scene, centered(8.0 * 0.53, 160.0, 90.0), "0.53").reused_frame, "a shape that touches the picture only by its soft edge may not have been made");
        let mut pair = Pair::new(gpu);
        assert!(!pair.frame(&scene, centered(8.0, 160.0, 90.0), "first").reused_frame);
        // The kept area reaches 144 pixels past the picture; at 0.55 times the zoom the picture reaches 160 / 0.55 = 291 from the
        // middle, which is inside (300 less the 4 pixels of inset), at 0.5 times 320, which is not.
        assert!(pair.frame(&scene, centered(4.4, 160.0, 90.0), "0.55").reused_frame);
        assert!(pair.frame(&scene, centered(12.0, 160.0, 90.0), "1.5").reused_frame);
        assert!(!pair.frame(&scene, centered(4.0, 160.0, 90.0), "0.5").reused_frame, "the picture would show shapes that were never made");
        // The frame made whole at 4: the zoom may now go to 8 (a factor of 2) and not past it.
        assert!(pair.frame(&scene, centered(8.0, 160.0, 90.0), "2").reused_frame);
        assert!(!pair.frame(&scene, centered(8.5, 160.0, 90.0), "2.1").reused_frame, "past the largest change that is drawn from the last frame");
    });
}

/// A frame brought up to date by an edit is made for the scale it was drawn at, and what it made again changes what the zoom may do.
#[test]
fn an_edit_is_not_brought_up_to_date_at_another_zoom_and_a_shape_that_was_made_again_narrows_the_window() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = circles(4.0);
        let id = scene.all_ids()[820];
        let mut pair = Pair::new(gpu);
        pair.frame(&scene, centered(4.0, 205.0, 205.0), "first");
        // A circle drawn at scale 2.2 (half its size, in the same place) with an outline that changed: its triangles are made again,
        // for bucket 2, which holds from 1.8, so the zoom may go down by 1.8 / 2.2 = 0.82 and not by the 0.45 that the others allow.
        scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::splat(205.0 * 0.45)) * Affine2::from_scale(Vec2::splat(0.55)));
        scene.find_mut(id).unwrap().path_mut().unwrap().subpaths[0].anchors[1].pos.x += 0.2;
        let patched = pair.frame(&scene, centered(4.0, 205.0, 205.0), "edited");
        assert!(patched.patched_items > 0 && patched.shapes_tessellated == 1, "{patched:?}");
        let small = pair.frame(&scene, centered(4.0 * 0.85, 205.0, 205.0), "a small zoom out");
        assert!(small.reused_frame, "{small:?}");
        let big = pair.frame(&scene, centered(4.0 * 0.8, 205.0, 205.0), "a zoom the made-again circle minds");
        assert!(!big.reused_frame, "the circle that was made again is below its bucket: {big:?}");
        // An edit at another zoom than the last frame's is a frame made whole (the shape would be made for the wrong scale).
        let mut scene = circles(4.0);
        let id = scene.all_ids()[820]; // in the middle of the picture
        let mut pair = Pair::new(gpu);
        pair.frame(&scene, centered(4.0, 205.0, 205.0), "first");
        scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(1.0, 1.0)));
        let moved = pair.frame(&scene, centered(4.2, 205.0, 205.0), "moved and zoomed");
        assert!(!moved.reused_frame && moved.patched_items == 0, "{moved:?}");
        // At the zoom of the last frame the same edit is patched.
        scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(2.0, 2.0)));
        let again = pair.frame(&scene, centered(4.2, 205.0, 205.0), "moved again");
        assert!(again.patched_items > 0, "{again:?}");
    });
}

/// A frame that is not made of plain shapes (here a group with opacity whose children are apart, so its opacity is in their rows, and
/// that stays apart only at some zooms) is drawn again for another zoom: whether things touch depends on it.
#[test]
fn a_zoom_is_not_drawn_from_a_frame_that_is_not_made_of_plain_shapes() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // Two rectangles 6 units apart horizontally: at 8 pixels per unit they are far apart, at 0.2 they touch.
        let children = vec![scene.new_node("a", NodeKind::Shape(rect_shape(0.0, 10.0))), scene.new_node("b", NodeKind::Shape(rect_shape(12.0, 10.0)))];
        let mut group = scene.new_node("faded", NodeKind::Group(children));
        group.opacity = 0.5;
        scene.add(group, None);
        let mut pair = Pair::new(gpu);
        let first = pair.frame(&scene, centered(8.0, 10.0, 12.0), "first");
        assert_eq!(first.layers, 0, "apart: no layer");
        let zoomed = pair.frame(&scene, centered(8.4, 10.0, 12.0), "zoomed");
        assert!(!zoomed.reused_frame, "{zoomed:?}");
        let moved = pair.frame(&scene, centered(8.4, 11.0, 12.0), "moved");
        assert!(moved.reused_frame, "a move of the same frame is still drawn from it: {moved:?}");
    });
}

/// Everything that makes the last frame wrong for the next one: each of these must walk the scene again, and each is checked
/// against a new renderer.
#[test]
fn a_frame_is_not_reused_when_the_scene_the_zoom_the_turn_the_margin_or_the_options_changed() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(1200);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, at(8.0, 0.0, 0.0), SIZE, &options()).unwrap();
        let check = |renderer: &mut Renderer, scene: &Scene, what: &str, view: Affine2, options: &RenderOptions, size: (u32, u32)| {
            let stats = renderer.render(scene, view, size, options).unwrap();
            assert!(!stats.reused_frame, "{what}: a frame that cannot be reused was: {stats:?}");
            let mut fresh = Renderer::new(gpu).unwrap();
            fresh.render(scene, view, size, options).unwrap();
            let part = |r: &Renderer| gpu.read_texture_part(r.picture().unwrap(), 0, 0, size.0, size.1);
            assert!(part(renderer) == part(&fresh), "{what}: not the picture a new renderer draws");
        };
        // A shape moved in the scene.
        scene.find_mut(ids[3]).unwrap().set_transform(Affine2::from_translation(Vec2::new(3.0, 2.0)));
        check(&mut renderer, &scene, "a shape moved", at(8.0, 0.0, 0.0), &options(), SIZE);
        // A zoom by more than the factor 2 that is drawn from the last frame (a small one is, see below), a turn, and a move past
        // the margin (shapes that were not made).
        check(&mut renderer, &scene, "zoomed far", at(20.0, 0.0, 0.0), &options(), SIZE);
        check(&mut renderer, &scene, "zoomed back", at(8.5, 0.0, 0.0), &options(), SIZE);
        check(&mut renderer, &scene, "turned", Affine2::from_angle(0.01) * at(8.5, 0.0, 0.0), &options(), SIZE);
        check(&mut renderer, &scene, "moved past the margin", at(8.5, 120.0, 0.0), &options(), SIZE);
        // Options that change the picture.
        check(&mut renderer, &scene, "with the camera frame", at(8.5, 120.0, 0.0), &RenderOptions { camera_frame: true, ..options() }, SIZE);
        check(&mut renderer, &scene, "without culling", at(8.5, 120.0, 0.0), &RenderOptions { camera_frame: true, cull: false, ..options() }, SIZE);
        // (With no culling everything was made, so any view can use that frame: a bigger one is reused, and is right.)
        let everything = RenderOptions { camera_frame: true, cull: false, ..options() };
        let bigger = renderer.render(&scene, at(8.5, 120.0, 0.0), (SIZE.0 * 2, SIZE.1 * 2), &everything).unwrap();
        assert!(bigger.reused_frame);
        // A view much larger than the one the shapes were made for, with culling on.
        check(&mut renderer, &scene, "culling back on", at(8.5, 120.0, 0.0), &options(), SIZE);
        check(&mut renderer, &scene, "a larger view", at(8.5, 120.0, 0.0), &options(), (SIZE.0 * 2, SIZE.1 * 2));
        // After a frame that was not reused, a repeat is.
        let again = renderer.render(&scene, at(8.5, 120.0, 0.0), (SIZE.0 * 2, SIZE.1 * 2), &options()).unwrap();
        assert!(again.reused_frame);
    });
}

/// Groups with opacity whose two children overlap, so that each needs a layer: `columns` by `rows`, `spacing` units apart.
fn faded_pairs(columns: usize, rows: usize, spacing: f32) -> Scene {
    let mut scene = Scene::default();
    for i in 0..columns * rows {
        let (x, y) = ((i % columns) as f32 * spacing, (i / columns) as f32 * spacing);
        let children = vec![scene.new_node("a", NodeKind::Shape(rect_shape(x, y))), scene.new_node("b", NodeKind::Shape(rect_shape(x + 3.0, y + 2.0)))];
        let mut group = scene.new_node("faded", NodeKind::Group(children));
        group.opacity = 0.5;
        scene.add(group, None);
    }
    scene
}

/// A frame with layers is drawn from the last one when the view only moved: each layer's box moves with the view, its picture is
/// drawn again into the same texture, and the line of the table that says where it is shown is written again. Layers that touch the
/// edge of the view change their size as they go in and out of it, and a layer that was out of the view, or comes into it, has nothing
/// collected for what is inside it: those frames are made whole. Every picture is the one a frame made whole draws.
#[test]
fn a_frame_with_layers_moves_them_with_the_view_and_is_the_picture_a_frame_made_whole_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = faded_pairs(6, 6, 14.0); // 36 layers of about 110 x 75 pixels at 12 pixels per unit, 168 pixels apart
        let mut pair = Pair::new(gpu);
        let first = pair.frame(&scene, centered(12.0, 14.0, 7.0), "first");
        assert!(!first.reused_frame && first.layers >= 4, "{first:?}");
        let (mut reused, mut made) = (0, 0);
        for step in 1..60 {
            let x = 14.0 + step as f32 * 0.45; // 5.4 pixels to the side in every frame
            let y = 7.0 + step as f32 * 0.2;
            let stats = pair.frame(&scene, centered(12.0, x, y), &format!("step {step}"));
            if stats.reused_frame {
                reused += 1;
                assert_eq!((stats.shapes_tessellated, stats.chunks_rebuilt, stats.geometry_bytes_uploaded), (0, 0, 0), "step {step}");
                assert!(stats.layers >= 1 && stats.passes == stats.layers + 1, "it still says what it drew: step {step}: {stats:?}");
            } else {
                made += 1;
            }
        }
        assert!(reused >= 30, "most of the moves keep the same layers: {reused} frames were reused and {made} made whole");
        assert!(made >= 1, "but a layer that comes into the view is another frame: {reused} reused, {made} made whole");
    });
}

/// Layers inside layers: the place of the inner composite is in the pixels of the outer layer, which moves with the view too.
#[test]
fn layers_inside_a_layer_are_moved_with_the_view_and_placed_in_the_picture_of_the_layer_around_them() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        for i in 0..4 {
            let x = i as f32 * 16.0;
            let deep = vec![scene.new_node("d1", NodeKind::Shape(rect_shape(x + 4.0, 5.0))), scene.new_node("d2", NodeKind::Shape(rect_shape(x + 7.0, 7.0)))];
            let mut deep_group = scene.new_node("deep", NodeKind::Group(deep));
            deep_group.opacity = 0.8;
            let outer = vec![scene.new_node("o1", NodeKind::Shape(rect_shape(x, 3.0))), deep_group, scene.new_node("o2", NodeKind::Shape(rect_shape(x + 6.0, 9.0)))];
            let mut outer_group = scene.new_node("outer", NodeKind::Group(outer));
            outer_group.opacity = 0.5;
            scene.add(outer_group, None);
        }
        let mut pair = Pair::new(gpu);
        let first = pair.frame(&scene, centered(10.0, 20.0, 8.0), "first");
        assert!(first.layers >= 4 && first.layers.is_multiple_of(2), "some outer layers and one inside each: {first:?}");
        let mut reused = 0;
        for step in 1..30 {
            let stats = pair.frame(&scene, centered(10.0, 20.0 + step as f32 * 0.4, 8.0 + step as f32 * 0.1), &format!("step {step}"));
            reused += stats.reused_frame as usize;
        }
        assert!(reused >= 15, "{reused} of 29 frames were reused");
    });
}

/// A layer that was drawn smaller than its place has a texture of another size and a row that says so; such a frame is made whole.
#[test]
fn a_frame_with_a_layer_drawn_smaller_is_never_drawn_from_the_last_one() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = faded_pairs(1, 1, 14.0);
        // The layer is about 162 x 126 pixels (192 x 128 in its class, 98 KB with its twin in the other renderer of the pair, which
        // draws it the same way): memory for it at half the size and not at its own.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Layers, 70_000));
        let mut pair = Pair::new(gpu);
        let first = pair.frame(&scene, centered(12.0, 6.0, 4.0), "first");
        assert_eq!((first.layers, first.layers_reduced), (1, 1), "{first:?}");
        // With the memory back, the layer would fit at its own size: the picture of the moved view is drawn at that size, not at the
        // half that the last frame drew it at.
        gpu.alloc.set_budgets(crate::alloc::Budgets::high());
        let moved = pair.frame(&scene, centered(12.0, 6.2, 4.0), "moved");
        assert!(!moved.reused_frame && moved.layers_reduced == 0, "{moved:?}");
    });
}

/// A layer that is out of the view but near it is not drawn (nothing inside it is collected): moving it into the view makes the frame
/// whole. A layer that leaves the view takes its pass with it: the frame is made whole too.
#[test]
fn a_layer_that_comes_into_the_view_or_leaves_it_makes_the_frame_whole() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        // One group of two rectangles at x = 100 to 109 units, 108 x 72 pixels at 12 pixels per unit. Its box on the screen is 27 pixels
        // wider on each side (a miter join can reach 24 pixels at this width and zoom, and 3 are the margin of anti-aliasing).
        let children = vec![scene.new_node("a", NodeKind::Shape(rect_shape(100.0, 10.0))), scene.new_node("b", NodeKind::Shape(rect_shape(103.0, 12.0)))];
        let mut group = scene.new_node("faded", NodeKind::Group(children));
        group.opacity = 0.5;
        scene.add(group, None);
        let mut pair = Pair::new(gpu);
        // With the middle of the picture at x = 83.7 the box starts at pixel 328.6, which is out of the view (320 wide) and inside the
        // area kept (144 pixels more).
        let out = pair.frame(&scene, centered(12.0, 83.7, 12.0), "the layer is out of the view");
        assert_eq!(out.layers, 0, "{out:?}");
        let near = pair.frame(&scene, centered(12.0, 83.9, 12.0), "still out");
        assert!(near.reused_frame && near.layers == 0, "{near:?}");
        let coming = pair.frame(&scene, centered(12.0, 90.0, 12.0), "the layer is in the view");
        assert!(!coming.reused_frame && coming.layers == 1, "a layer that came in has no picture to show: {coming:?}");
        let moving = pair.frame(&scene, centered(12.0, 90.5, 12.0), "moved with it");
        assert!(moving.reused_frame && moving.layers == 1, "{moving:?}");
        // The box ends 20 pixels inside the left edge of the view at x = 122.9; 2.3 units later (28 pixels) it is out.
        pair.frame(&scene, centered(12.0, 122.9, 12.0), "near the left edge");
        let gone = pair.frame(&scene, centered(12.0, 125.2, 12.0), "the layer left");
        assert!(!gone.reused_frame && gone.layers == 0, "a layer that left has no pass any more: {gone:?}");
    });
}

/// The picture of a layer is a texture of a size class; a layer that grows past the class of its texture as it moves into the view
/// gets another one, and the frame is still drawn from the last one.
#[test]
fn a_layer_that_changes_its_size_class_as_it_enters_the_view_gets_a_new_texture_and_the_frame_is_still_reused() {
    Gpu::for_each_test_mode(|gpu| {
        // A group 120 x 60 units, 480 x 240 pixels at 4 pixels per unit: wider than the 320 pixel view, so it is clipped at the
        // right edge and the clipped part grows as it moves in from there.
        let mut scene = Scene::default();
        let children = vec![scene.new_node("a", NodeKind::Shape(Shape { path: Path::rect(Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)), ..rect_shape(0.0, 0.0) })), scene.new_node("b", NodeKind::Shape(Shape { path: Path::rect(Vec2::new(20.0, 10.0), Vec2::new(120.0, 60.0)), ..rect_shape(0.0, 0.0) }))];
        let mut group = scene.new_node("faded", NodeKind::Group(children));
        group.opacity = 0.5;
        scene.add(group, None);
        let mut pair = Pair::new(gpu);
        // With the middle of the picture at x = -35 the group's left edge is at pixel 160 + 35 * 4 = 300, just inside the view.
        pair.frame(&scene, centered(4.0, -35.0, 30.0), "first");
        let (mut reused, mut made) = (0, 0);
        let mut layer_bytes = std::collections::BTreeSet::new();
        for step in 1..60 {
            let x = -35.0 + step as f32 * 0.5; // the view moves right, so the group moves left, 2 pixels a frame
            let stats = pair.frame(&scene, centered(4.0, x, 30.0), &format!("step {step}"));
            if stats.reused_frame {
                reused += 1;
                layer_bytes.insert(pair.reusing.memory().into_iter().find(|(category, _)| *category == Category::Layers).map(|(_, bytes)| bytes));
            } else {
                made += 1;
            }
        }
        assert!(reused >= 55, "the view stays inside the area that was kept: {reused} reused, {made} made whole");
        assert!(layer_bytes.len() >= 2, "the clipped width went through at least one class edge, and the layer got another texture: {layer_bytes:?}");
    });
}

/// Instances are culled by the area that was kept for the view, as every other shape: a frame with instances is drawn from the last one
/// when the view only moved.
#[test]
fn a_frame_with_instances_is_drawn_from_the_last_one_when_the_view_moved_and_not_when_the_scale_changed() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(5);
        let (symbol, _) = scene.make_symbol(&ids[..2], "Art").expect("a symbol");
        for i in 0..12 {
            let id = scene.place_instance(symbol, Vec2::ZERO).expect("an instance");
            scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new((i % 4) as f32 * 30.0, (i / 4) as f32 * 20.0)) * Affine2::from_scale(Vec2::splat(1.0 + i as f32 * 0.1)));
        }
        let mut pair = Pair::new(gpu);
        let first = pair.frame(&scene, centered(8.0, 40.0, 25.0), "first");
        assert!(!first.reused_frame && first.items >= 24, "{first:?}");
        for (i, (x, y)) in [(41.0, 25.0), (43.0, 24.0), (38.0, 27.0), (40.0, 22.0)].into_iter().enumerate() {
            let stats = pair.frame(&scene, centered(8.0, x, y), &format!("move {i}"));
            assert!(stats.reused_frame, "move {i}: {stats:?}");
            assert_eq!((stats.items, stats.draw_calls), (first.items, first.draw_calls));
        }
        let zoomed = pair.frame(&scene, centered(8.4, 40.0, 25.0), "zoomed");
        assert!(!zoomed.reused_frame, "a frame with instances is made whole for another scale: {zoomed:?}");
    });
}

/// The rectangles of layers are clipped to the view, so a frame with layers is not drawn for a view of another size.
#[test]
fn a_frame_with_layers_is_not_drawn_from_the_last_one_for_a_view_of_another_size() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = faded_pairs(3, 3, 14.0);
        let mut renderer = Renderer::new(gpu).unwrap();
        let view = centered(12.0, 14.0, 7.0);
        let first = renderer.render(&scene, view, SIZE, &options()).unwrap();
        assert!(first.layers >= 1);
        assert!(renderer.render(&scene, view, SIZE, &options()).unwrap().reused_frame);
        let other = (SIZE.0 + 40, SIZE.1);
        let bigger = renderer.render(&scene, view, other, &options()).unwrap();
        assert!(!bigger.reused_frame, "{bigger:?}");
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, view, other, &options()).unwrap();
        let part = |r: &Renderer| gpu.read_texture_part(r.picture().unwrap(), 0, 0, other.0, other.1);
        assert!(part(&renderer) == part(&fresh));
    });
}

// --- Frames brought up to date from the scene's journal (plan 5.3) -------------------------------------------------------------------

/// A scene for the edit tests: groups of shapes, curved and straight, with and without strokes, a few with gradients.
fn editable_scene() -> (Scene, Vec<NodeId>, Vec<NodeId>) {
    let mut scene = Scene::default();
    let (mut shapes, mut groups) = (Vec::new(), Vec::new());
    for g in 0..12 {
        let mut children = Vec::new();
        for i in 0..12 {
            let n = g * 12 + i;
            let at = Vec2::new((n % 16) as f32 * 18.0 + 8.0, (n / 16) as f32 * 18.0 + 8.0);
            let path = match n % 3 {
                0 => Path::ellipse(at, Vec2::new(6.0 + (n % 4) as f32, 5.0)),
                1 => Path::rect(at - Vec2::splat(6.0), at + Vec2::splat(6.0)),
                _ => Path { subpaths: vec![SubPath { anchors: [(0.0, 0.0), (11.0, 2.0), (4.0, 12.0)].map(|(x, y)| Anchor::corner(at + Vec2::new(x - 6.0, y - 6.0))).to_vec(), closed: true }] },
            };
            let mut shape = Shape::new(path);
            shape.fill = Some(if n % 11 == 0 { Paint::linear(at, at + Vec2::X * 10.0, Color::rgb(250, 200, 30), Color::rgb(30, 60, 220)) } else { Paint::Solid(Color::rgb(40 + (n % 7) as u8 * 30, 120, 200 - (n % 5) as u8 * 30)) });
            shape.stroke = (n % 4 != 0).then(|| Stroke { paint: Paint::Solid(Color::rgb(20, 20, 20)), width: 1.0 + (n % 3) as f32, join: if n % 2 == 0 { LineJoin::Round } else { LineJoin::Miter }, cap: LineCap::Butt, miter_limit: 4.0 });
            let node = scene.new_node(format!("s{n}"), NodeKind::Shape(shape));
            shapes.push(node.id);
            children.push(node);
        }
        let group = scene.new_node(format!("g{g}"), NodeKind::Group(children));
        groups.push(group.id);
        scene.add(group, None);
    }
    (scene, shapes, groups)
}

#[test]
fn moving_one_shape_brings_up_only_its_items_and_the_view_is_still_right() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, shapes, _) = editable_scene();
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        assert!(!first.reused_frame && first.patched_items == 0);
        scene.find_mut(shapes[5]).unwrap().set_transform(Affine2::from_translation(Vec2::new(7.0, 3.0)));
        let moved = renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        assert!(moved.patched_items >= 1 && moved.patched_items <= 2, "a fill and a stroke at most: {moved:?}");
        assert_eq!((moved.shapes_tessellated, moved.chunks_rebuilt, moved.geometry_bytes_uploaded), (0, 0, 0), "{moved:?}");
        assert!(moved.table_bytes_uploaded > 0 && moved.table_bytes_uploaded <= 16 * 1024, "one line of the table at most: {moved:?}");
        assert!(moved.update_ms < first.update_ms, "and it is quick: {} against {}", moved.update_ms, first.update_ms);
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh));
    });
}

/// Undo puts back an older copy of the scene, and the next edit is made to that copy. What the journal of the copy says (one edit)
/// is not what changed since the frame that was drawn (everything that was done and then undone): the copy must be drawn whole.
#[test]
fn an_older_copy_of_the_scene_with_one_new_edit_is_drawn_whole_not_patched() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, shapes, _) = editable_scene();
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        let snapshot = scene.clone(); // what undo will go back to
        for (i, id) in [shapes[3], shapes[40], shapes[76]].into_iter().enumerate() { // (every eleventh shape has a gradient: not patched)
            scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(9.0 + i as f32, 4.0)));
            let stats = renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
            assert!(stats.patched_items > 0, "{stats:?}");
        }
        // Undo, then one new edit of another shape.
        let mut undone = snapshot;
        undone.find_mut(shapes[100]).unwrap().set_transform(Affine2::from_translation(Vec2::new(-5.0, 2.0)));
        let stats = renderer.render(&undone, Affine2::IDENTITY, SIZE, &options()).unwrap();
        assert_eq!(stats.patched_items, 0, "a copy is not the scene that was drawn: {stats:?}");
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&undone, Affine2::IDENTITY, SIZE, &options()).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "the three undone moves must be gone");
    });
}

/// A group with opacity whose children do not touch gets no layer: its opacity is in the rows of the children. A frame like that is
/// drawn whole when one of the children changes, because a row written alone would not know about the group's opacity.
#[test]
fn a_child_of_a_faded_group_is_not_patched_because_its_row_carries_the_groups_opacity() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let children: Vec<_> = (0..4).map(|i| scene.new_node("child", NodeKind::Shape(rect_shape(i as f32 * 30.0, 10.0)))).collect();
        let ids: Vec<NodeId> = children.iter().map(|c| c.id).collect();
        let mut group = scene.new_node("faded", NodeKind::Group(children));
        group.opacity = 0.5;
        scene.add(group, None);
        let view = Affine2::from_scale(Vec2::splat(2.0));
        let mut renderer = Renderer::new(gpu).unwrap();
        let first = renderer.render(&scene, view, SIZE, &options()).unwrap();
        assert_eq!(first.layers, 0, "the children are apart: the opacity went into their rows");
        scene.find_mut(ids[1]).unwrap().set_transform(Affine2::from_translation(Vec2::new(3.0, 4.0)));
        let moved = renderer.render(&scene, view, SIZE, &options()).unwrap();
        assert_eq!(moved.patched_items, 0, "{moved:?}");
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, view, SIZE, &options()).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "the moved child must still be at half opacity");
    });
}

/// A random number generator that needs nothing (the tests must give the same frames every time).
struct Dice(u64);

impl Dice {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % below
    }
}

#[test]
fn edits_brought_up_to_date_one_node_at_a_time_give_the_picture_a_new_renderer_draws_every_frame() {
    Gpu::for_each_test_mode(|gpu| {
        for seed in 1..=3u64 {
            let (mut scene, shapes, groups) = editable_scene();
            let mut dice = Dice(seed * 7919);
            let mut renderer = Renderer::new(gpu).unwrap();
            let (mut patched, mut reused, mut full) = (0, 0, 0);
            let (mut zoom_steps, mut pan) = (vec![1.0f32; shapes.len()], 0.0f32);
            let mut restore: Vec<NodeId> = Vec::new();
            for frame in 0..120 {
                // What was hidden or faded for one frame comes back, so that the scene is mostly plain shapes again.
                for id in restore.drain(..) {
                    if let Some(mut node) = scene.find_mut(id) {
                        node.set_visible(true);
                        node.set_opacity(1.0);
                    }
                }
                for _ in 0..1 + dice.next(3) {
                    let id = shapes[dice.next(shapes.len())];
                    match dice.next(16) {
                        0..=3 => {
                            let t = Affine2::from_translation(Vec2::new(dice.next(9) as f32 - 4.0, dice.next(9) as f32 - 4.0));
                            if let Some(mut node) = scene.find_mut(id) {
                                let now = node.transform;
                                node.set_transform(t * now);
                            }
                        }
                        4 => {
                            scene.find_mut(id).map(|mut n| n.set_solid_fill_color(Color::rgb(dice.next(255) as u8, dice.next(255) as u8, 90)));
                        }
                        5 => {
                            scene.find_mut(id).map(|mut n| n.set_solid_stroke_color(Color::rgb(30, dice.next(255) as u8, dice.next(255) as u8)));
                        }
                        6 | 7 => {
                            let shift = Vec2::new(dice.next(5) as f32 - 2.0, dice.next(5) as f32 - 2.0);
                            if let Some(mut node) = scene.find_mut(id) {
                                if let Some(anchor) = node.path_mut().and_then(|p| p.subpaths.first_mut()).and_then(|s| s.anchors.first_mut()) {
                                    anchor.pos += shift;
                                }
                            }
                        }
                        8 => {
                            // Three times bigger, or back: always far enough to leave the zoom bucket.
                            let index = shapes.iter().position(|s| *s == id).unwrap();
                            zoom_steps[index] = if zoom_steps[index] > 1.5 { 1.0 } else { 3.0 };
                            let center = Vec2::new(60.0, 60.0);
                            let scale = Affine2::from_translation(center) * Affine2::from_scale(Vec2::splat(zoom_steps[index])) * Affine2::from_translation(-center);
                            if let Some(mut node) = scene.find_mut(id) {
                                node.set_transform(scale);
                            }
                        }
                        9 => {
                            if let Some(mut node) = scene.find_mut(id) {
                                let now = node.transform;
                                node.set_transform(now * Affine2::from_angle(0.3));
                            }
                        }
                        10 => {
                            if let Some(mut node) = scene.find_mut(id) {
                                node.set_visible(false);
                            }
                            restore.push(id);
                        }
                        11 if dice.next(3) == 0 => {
                            if let Some(mut node) = scene.find_mut(id) {
                                node.set_opacity(0.6);
                            }
                            restore.push(id);
                        }
                        12 => {
                            let g = groups[dice.next(groups.len())];
                            let shift = Vec2::new(dice.next(7) as f32 - 3.0, 1.0);
                            if let Some(mut node) = scene.find_mut(g) {
                                let now = node.transform;
                                node.set_transform(Affine2::from_translation(shift) * now);
                            }
                        }
                        13 => {
                            let node = scene.new_node("new", NodeKind::Shape(rect_shape(dice.next(300) as f32, dice.next(200) as f32)));
                            scene.add(node, None);
                        }
                        14 => pan = (pan + dice.next(9) as f32 - 4.0).clamp(-30.0, 30.0),
                        15 if dice.next(2) == 0 => {
                            // A stroke appears or goes (the shape then has other parts than the frame that was drawn), or the shape is deleted.
                            if dice.next(4) == 0 {
                                scene.remove(id);
                            } else if let Some(shape) = scene.shape_mut(id) {
                                shape.stroke = if shape.stroke.is_some() { None } else { Some(Stroke { paint: Paint::Solid(Color::rgb(200, 30, 30)), width: 2.0, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 }) };
                            }
                        }
                        _ => {} // nothing at all this time (or a fade that was not drawn)
                    }
                }
                let view = Affine2::from_translation(Vec2::new(pan, 0.0)) * Affine2::from_scale(Vec2::splat(1.1));
                let stats = renderer.render(&scene, view, SIZE, &options()).unwrap_or_else(|e| panic!("seed {seed}, frame {frame}: {e}"));
                if stats.patched_items > 0 {
                    patched += 1;
                } else if stats.reused_frame {
                    reused += 1;
                } else {
                    full += 1;
                }
                let mut fresh = Renderer::new(gpu).unwrap();
                fresh.render(&scene, view, SIZE, &options()).unwrap();
                let part = |r: &Renderer| gpu.read_texture_part(r.picture().unwrap(), 0, 0, SIZE.0, SIZE.1);
                assert!(part(&renderer) == part(&fresh), "seed {seed}, frame {frame}: not the picture a new renderer draws ({stats:?})");
            }
            println!("seed {seed}: {patched} frames brought up to date node by node, {reused} reused, {full} made whole");
            assert!(patched >= 15, "the path under test was hardly used: {patched} of 120 frames");
        }
    });
}

// --- A budget for making shapes (plan 5.4) ------------------------------------------------------------------------------------------

/// How much of a picture is not transparent: a way to see holes without comparing curves that are cut a little differently.
fn covered(picture: &[u8]) -> usize {
    picture.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count()
}

#[test]
fn a_zoom_across_a_bucket_is_finished_over_several_frames_with_no_holes_and_ends_as_a_new_renderer_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(4.0);
        let zoom_in = Affine2::from_scale(Vec2::splat(4.0));
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        // A fresh renderer's picture of the zoomed view, and the same view from the one that made the shapes for zoom 1.
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, zoom_in, SIZE, &options()).unwrap();
        let wanted = picture_bytes(gpu, &fresh);
        renderer.set_tessellation_budget(Some(0.2));
        let mut frames = 0;
        let mut first = None;
        loop {
            let stats = renderer.render(&scene, zoom_in, SIZE, &options()).unwrap();
            frames += 1;
            first.get_or_insert_with(|| stats.clone());
            if frames == 1 {
                // The first frame already shows every shape that is in the view (with the triangles it has): no holes.
                let shown = covered(&picture_bytes(gpu, &renderer));
                assert!(shown * 100 >= covered(&wanted) * 99, "{shown} covered pixels of {}", covered(&wanted));
            }
            if stats.deferred == 0 {
                break;
            }
            assert!(!stats.reused_frame, "a frame with work waiting is never reused");
            assert!(frames < 2000, "it never finishes: {stats:?}");
        }
        let first = first.unwrap();
        assert!(first.deferred > 0 && frames > 2, "the work was spread over {frames} frames, the first deferred {}", first.deferred);
        assert!(first.shapes_tessellated < renderer.cache().len() / 2, "{first:?}");
        assert!(picture_bytes(gpu, &renderer) == wanted, "after the last frame it is the picture a new renderer draws");
        // Nothing waits any more: the next frame is the last one again.
        assert!(renderer.render(&scene, zoom_in, SIZE, &options()).unwrap().reused_frame);
    });
}

#[test]
fn a_shape_that_is_in_the_view_is_made_at_once_whatever_the_budget_and_only_what_is_kept_ready_waits() {
    Gpu::for_each_test_mode(|gpu| {
        let (scene, _) = grid(1200);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.set_tessellation_budget(Some(0.0));
        let view = |x: f32| Affine2::from_scale(Vec2::splat(8.0)) * Affine2::from_translation(Vec2::new(-x, 0.0));
        let first = renderer.render(&scene, view(0.0), SIZE, &options()).unwrap();
        // Nothing may be missing from the view although there was no time at all: what is in it was made.
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, view(0.0), SIZE, &options()).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "{first:?}");
        assert!(first.deferred > 0, "the margin around the view waited: {first:?}");
        assert!(first.shapes_tessellated > 0 && first.shapes_tessellated < renderer.cache().len() + first.deferred);
        // A jump to somewhere else: again everything in the view is there at once.
        renderer.render(&scene, view(150.0), SIZE, &options()).unwrap();
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, view(150.0), SIZE, &options()).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh));
    });
}

#[test]
fn a_view_without_a_budget_makes_everything_at_once_as_before() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(4.0);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, Affine2::IDENTITY, SIZE, &options()).unwrap();
        let zoomed = renderer.render(&scene, Affine2::from_scale(Vec2::splat(4.0)), SIZE, &options()).unwrap();
        assert_eq!(zoomed.deferred, 0);
        assert!(zoomed.shapes_tessellated > 100, "{zoomed:?}");
    });
}

#[test]
fn a_first_frame_makes_the_buffers_just_large_enough_and_not_up_to_twice_that() {
    Gpu::for_each_test_mode(|gpu| {
        let scene = circles(15.0);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, Affine2::from_scale(Vec2::splat(1.0)), SIZE, &without_culling()).unwrap();
        for (name, buffer) in [("vertex", &renderer.store().vertices), ("index", &renderer.store().indices)] {
            let (top, capacity) = (buffer.ranges().top() as u64, buffer.capacity() as u64);
            assert!(top > 100_000, "{name} buffer: the scene should be a big one ({top})");
            assert!(capacity >= top && capacity <= top + top / 4, "{name} buffer: {capacity} elements of room for {top} in use");
        }
    });
}

/// The layers a scene needs (a layer is a pass of its own). The opacity of a single shape goes into its row where that is
/// exactly what the CPU renderer's layer gives; anything else with opacity is drawn on a layer.
fn layers_of(gpu: &Gpu, scene: &Scene) -> usize {
    let mut renderer = Renderer::new(gpu).unwrap();
    render(&mut renderer, scene).layers
}

#[test]
fn a_shape_with_opacity_needs_a_layer_only_where_its_row_cannot_do_it() {
    Gpu::for_each_test_mode(|gpu| {
        let opacity_of = |fill: Option<Paint>, stroke: Option<Paint>, opacity: f32| {
            let mut scene = Scene::default();
            let mut shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)));
            shape.fill = fill;
            shape.stroke = stroke.map(|paint| Stroke { paint, width: 2.0, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 });
            let node = scene.new_node("shape", NodeKind::Shape(shape));
            let id = scene.add(node, None);
            scene.find_mut(id).unwrap().set_opacity(opacity);
            layers_of(gpu, &scene) > 0
        };
        let solid = |a: u8| Paint::Solid(Color::rgba(10, 20, 30, a));
        let ramp = |a: u8| Paint::linear(Vec2::ZERO, Vec2::X, Color::rgba(0, 0, 0, a), Color::rgba(255, 255, 255, 255));
        // One part, whatever its paint: the row's opacity does it.
        assert!(!opacity_of(Some(solid(255)), None, 0.5));
        assert!(!opacity_of(Some(solid(100)), None, 0.5));
        assert!(!opacity_of(None, Some(ramp(0)), 0.5));
        // Both parts and both opaque: the stroke first and the same depth. A see-through paint on either needs a layer.
        assert!(!opacity_of(Some(solid(255)), Some(solid(255)), 0.5));
        assert!(!opacity_of(Some(ramp(255)), Some(solid(255)), 0.5));
        assert!(opacity_of(Some(solid(255)), Some(solid(200)), 0.5), "a see-through stroke over a fill");
        assert!(opacity_of(Some(solid(200)), Some(solid(255)), 0.5), "a see-through fill under a stroke");
        assert!(opacity_of(Some(ramp(255)), Some(ramp(0)), 0.5), "a gradient that fades out");
        // No opacity: nothing special, and a shape that draws no part at all has nothing to fade.
        assert!(!opacity_of(Some(solid(100)), Some(solid(100)), 1.0));
        assert!(!opacity_of(None, None, 0.5));
    });
}

/// Two fill-only rectangles with a gap: a soft edge can reach a pixel and a half past a shape, so shapes closer than three
/// pixels can both paint one pixel and the group needs a layer; farther apart they cannot, and it does not.
#[test]
fn the_margin_for_anti_aliasing_decides_how_close_two_shapes_may_be_without_a_layer() {
    Gpu::for_each_test_mode(|gpu| {
        let with_gap = |gap: f32| {
            let mut scene = Scene::default();
            let boxes: Vec<_> = [0.0, 10.0 + gap]
                .iter()
                .map(|&x| {
                    let mut shape = Shape::new(Path::rect(Vec2::new(x, 0.0), Vec2::new(x + 10.0, 10.0)));
                    shape.fill = Some(Paint::Solid(Color::rgb(200, 30, 30)));
                    shape.stroke = None;
                    scene.new_node("box", NodeKind::Shape(shape))
                })
                .collect();
            let mut group = scene.new_node("group", NodeKind::Group(boxes));
            group.opacity = 0.5;
            scene.add(group, None);
            scene
        };
        assert_eq!(layers_of(gpu, &with_gap(0.0)), 1, "touching");
        assert_eq!(layers_of(gpu, &with_gap(2.5)), 1, "a soft edge can bridge it");
        assert_eq!(layers_of(gpu, &with_gap(3.5)), 0, "too far for that");
    });
}

/// A group with opacity: the opacity goes into the children when there is one child or when their boxes are apart, and the
/// group gets a layer when they touch (here by a margin of one pixel, which anti-aliasing can bridge).
#[test]
fn a_group_with_opacity_needs_a_layer_only_where_its_children_can_touch() {
    Gpu::for_each_test_mode(|gpu| {
        let faded_group = |positions: &[(f32, f32)]| {
            let mut scene = Scene::default();
            let children: Vec<_> = positions.iter().enumerate().map(|(i, &(x, y))| scene.new_node(format!("c{i}"), NodeKind::Shape(rect_shape(x, y)))).collect();
            let mut group = scene.new_node("group", NodeKind::Group(children));
            group.opacity = 0.5;
            scene.add(group, None);
            scene
        };
        // Each rect is 6 x 4 with a 1-wide stroke (a box of about 7 x 5 plus the anti-aliasing margin).
        assert_eq!(layers_of(gpu, &faded_group(&[(0.0, 0.0)])), 0, "one child");
        assert_eq!(layers_of(gpu, &faded_group(&[(0.0, 0.0), (30.0, 0.0), (0.0, 30.0)])), 0, "children far apart");
        assert_eq!(layers_of(gpu, &faded_group(&[(0.0, 0.0), (4.0, 2.0)])), 1, "children that overlap");
        assert_eq!(layers_of(gpu, &faded_group(&[(0.0, 0.0), (7.5, 0.0)])), 1, "children one pixel apart can share an anti-aliased pixel");
        // Nested: the outer group has two groups that are far apart, each with two overlapping shapes: one layer each.
        let mut scene = Scene::default();
        let inner = |scene: &mut Scene, x: f32| {
            let a = scene.new_node("a", NodeKind::Shape(rect_shape(x, 0.0)));
            let b = scene.new_node("b", NodeKind::Shape(rect_shape(x + 3.0, 1.0)));
            scene.new_node("inner", NodeKind::Group(vec![a, b]))
        };
        let (left, right) = (inner(&mut scene, 0.0), inner(&mut scene, 100.0));
        let mut outer = scene.new_node("outer", NodeKind::Group(vec![left, right]));
        outer.opacity = 0.5;
        let mut renderer = Renderer::new(gpu).unwrap();
        scene.add(outer, None);
        // The outer group's opacity goes down into its two inner groups (they are far apart), and each of those draws two
        // overlapping shapes, so each needs a layer for the opacity it received.
        let stats = render(&mut renderer, &scene);
        assert_eq!(stats.layers, 2, "{stats:?}");
    });
}

#[test]
fn a_shape_with_opacity_and_both_parts_shares_one_depth_value_with_the_stroke_first() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let plain = scene.new_node("before", NodeKind::Shape(rect_shape(0.0, 0.0)));
        scene.add(plain, None);
        let both = scene.new_node("both", NodeKind::Shape(rect_shape(20.0, 0.0)));
        let both = scene.add(both, None);
        scene.find_mut(both).unwrap().set_opacity(0.5);
        let after = scene.new_node("after", NodeKind::Shape(rect_shape(40.0, 0.0)));
        scene.add(after, None);
        let mut renderer = Renderer::new(gpu).unwrap();
        render(&mut renderer, &scene);
        // Rows are handed out in the order the items are first seen: the plain shape's fill (0) and stroke (1), then the
        // stroke (2) and the fill (3) of the faded shape, then the last shape's.
        let depths: Vec<f32> = (0..6).map(|row| renderer.table().get(row).depth()).collect();
        let step = depths[0];
        let levels: Vec<f32> = depths.iter().map(|d| d / step).collect();
        assert_eq!(levels, [1.0, 2.0, 3.0, 3.0, 4.0, 5.0], "the faded shape's two items share the level 3");
        assert_eq!(renderer.table().get(2).0[14], 0.5, "the stroke is faded");
        assert_eq!(renderer.table().get(3).0[14], 0.5, "and so is the fill");
        assert_eq!(renderer.table().get(0).0[14], 1.0, "the others are not");
    });
}

// --- Groups and instances that move (plan 6.4) -----------------------------------------------------------------------------------

/// A scene for moving things about: a background, six groups of twenty small rectangles (each with a transform of its own, one with a
/// group inside it), and thirty instances of a symbol of three parts (two ellipses and a rectangle) at several sizes. Every paint is
/// solid and everything is at full opacity, so that a frame of it is flat. Returns the scene, the groups, the instances, and the
/// shapes of the first group.
fn moving_scene() -> (Scene, Vec<NodeId>, Vec<NodeId>, Vec<NodeId>) {
    let mut scene = Scene::default();
    let ground = scene.new_node("ground", NodeKind::Shape(Shape { path: Path::rect(Vec2::new(-20.0, -20.0), Vec2::new(200.0, 140.0)), stroke: None, fill: Some(Paint::Solid(Color::rgb(30, 50, 40))), ..rect_shape(0.0, 0.0) }));
    scene.add(ground, None);
    let (mut groups, mut first_shapes) = (Vec::new(), Vec::new());
    for g in 0..6 {
        let mut children = Vec::new();
        for i in 0..20 {
            let mut node = scene.new_node(format!("r{g}-{i}"), NodeKind::Shape(rect_shape((i % 5) as f32 * 8.0, (i / 5) as f32 * 6.0)));
            if g == 0 {
                first_shapes.push(node.id);
            }
            if let NodeKind::Shape(shape) = &mut node.kind {
                shape.fill = Some(Paint::Solid(Color::rgb(40 + g as u8 * 30, 120, 200 - i as u8 * 5)));
            }
            children.push(node);
        }
        if g == 1 {
            // A group inside a group.
            let inner: Vec<_> = (0..4).map(|i| scene.new_node("inner", NodeKind::Shape(rect_shape(i as f32 * 9.0, 30.0)))).collect();
            let mut inner = scene.new_node("inner group", NodeKind::Group(inner));
            inner.transform = Affine2::from_translation(Vec2::new(2.0, 1.0));
            children.push(inner);
        }
        let mut group = scene.new_node(format!("g{g}"), NodeKind::Group(children));
        group.transform = Affine2::from_translation(Vec2::new((g % 3) as f32 * 52.0 + 4.0, (g / 3) as f32 * 40.0 + 6.0)) * Affine2::from_angle(g as f32 * 0.03);
        groups.push(scene.add(group, None));
    }
    let parts = vec![
        scene.new_node("body", NodeKind::Shape(Shape { path: Path::ellipse(Vec2::ZERO, Vec2::new(6.0, 4.0)), fill: Some(Paint::Solid(Color::rgb(230, 190, 70))), ..rect_shape(0.0, 0.0) })),
        scene.new_node("eye", NodeKind::Shape(Shape { path: Path::ellipse(Vec2::new(3.0, -1.0), Vec2::splat(1.2)), fill: Some(Paint::Solid(Color::rgb(20, 20, 30))), stroke: None, ..rect_shape(0.0, 0.0) })),
        scene.new_node("tail", NodeKind::Shape(Shape { path: Path::rect(Vec2::new(-9.0, -1.0), Vec2::new(-6.0, 1.0)), fill: Some(Paint::Solid(Color::rgb(200, 150, 60))), ..rect_shape(0.0, 0.0) })),
    ];
    let ids: Vec<NodeId> = parts.into_iter().map(|node| scene.add(node, None)).collect();
    let (symbol, first) = scene.make_symbol(&ids, "Sprite").expect("a symbol");
    scene.remove(first);
    let mut instances = Vec::new();
    for k in 0..30 {
        let id = scene.place_instance(symbol, Vec2::ZERO).expect("an instance");
        let scale = 0.8 + (k % 5) as f32 * 0.5;
        scene.find_mut(id).unwrap().set_transform(Affine2::from_scale_angle_translation(Vec2::splat(scale), k as f32 * 0.4, Vec2::new((k % 10) as f32 * 14.0 + 12.0, 90.0 + (k / 10) as f32 * 9.0)));
        instances.push(id);
    }
    (scene, groups, instances, first_shapes)
}

fn shift(scene: &mut Scene, id: NodeId, by: Vec2) {
    let mut node = scene.find_mut(id).unwrap();
    let now = node.transform;
    node.set_transform(Affine2::from_translation(by) * now);
}

/// A group that moves, an instance that moves, a group inside a group: only the items below the node that moved get new rows. Nothing
/// is walked, nothing is made again, no triangles are sent, and the picture is the one a frame made whole draws.
#[test]
fn a_group_or_an_instance_that_moves_brings_up_only_its_own_items_and_is_the_picture_a_frame_made_whole_draws() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, groups, instances, first) = moving_scene();
        let mut pair = Pair::new(gpu);
        let view = at(2.0, 0.0, 0.0);
        let start = pair.frame(&scene, view, "first");
        assert!(!start.reused_frame && start.items > 200, "{start:?}");

        // A group of twenty rectangles with a fill and a stroke each: forty items.
        shift(&mut scene, groups[2], Vec2::new(3.0, 2.0));
        let moved = pair.frame(&scene, view, "a group moved");
        assert_eq!(moved.patched_items, 40, "{moved:?}");
        assert_eq!((moved.shapes_tessellated, moved.chunks_rebuilt, moved.geometry_bytes_uploaded, moved.vertices_uploaded), (0, 0, 0, 0), "{moved:?}");
        assert!(moved.table_bytes_uploaded > 0 && moved.table_bytes_uploaded <= 32 * 1024, "{moved:?}");

        // An instance of a symbol of three parts: a body, an eye and a tail, with a stroke on two of them: five items.
        shift(&mut scene, instances[4], Vec2::new(-2.0, 1.0));
        let moved = pair.frame(&scene, view, "an instance moved");
        assert_eq!(moved.patched_items, 5, "{moved:?}");
        assert_eq!((moved.shapes_tessellated, moved.chunks_rebuilt, moved.geometry_bytes_uploaded), (0, 0, 0), "{moved:?}");

        // A group inside a group moved: the inner one's items, and a shape of the outer one changed at the same time.
        let inner = match &scene.find(groups[1]).unwrap().kind {
            NodeKind::Group(children) => children.last().unwrap().id,
            _ => unreachable!(),
        };
        shift(&mut scene, inner, Vec2::new(1.0, -1.0));
        scene.find_mut(first[3]).unwrap().set_solid_fill_color(Color::rgb(250, 10, 10));
        let both = pair.frame(&scene, view, "an inner group moved and a shape of another recolored");
        assert_eq!(both.patched_items, 8 + 2, "{both:?}");

        // Many at once: ten instances and a group.
        for &id in &instances[10..20] {
            shift(&mut scene, id, Vec2::new(0.5, -0.5));
        }
        shift(&mut scene, groups[4], Vec2::new(-1.0, 0.0));
        let many = pair.frame(&scene, view, "ten instances and a group");
        assert_eq!(many.patched_items, 10 * 5 + 40, "{many:?}");

        // Moving a group, and then a shape inside it that is not moved with it: the shape is where the group is now.
        shift(&mut scene, groups[0], Vec2::new(4.0, 4.0));
        pair.frame(&scene, view, "a group moved again");
        scene.find_mut(first[7]).unwrap().set_solid_fill_color(Color::rgb(10, 250, 10));
        let inside = pair.frame(&scene, view, "a shape inside the group that moved");
        assert_eq!(inside.patched_items, 2, "{inside:?}");
    });
}

/// An instance at another size draws the art of its symbol with other triangles: they are sent, the instances that share the chunk keep
/// theirs, and the picture is right.
#[test]
fn an_instance_that_grows_into_another_zoom_bucket_gets_its_triangles_and_leaves_its_neighbours_alone() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, _, instances, _) = moving_scene();
        let mut pair = Pair::new(gpu);
        let view = at(2.0, 0.0, 0.0);
        pair.frame(&scene, view, "first");
        for (step, scale) in [4.0f32, 0.3, 1.0, 6.0].into_iter().enumerate() {
            let id = instances[7 + step];
            let mut node = scene.find_mut(id).unwrap();
            let at_now = node.transform.translation;
            node.set_transform(Affine2::from_scale_angle_translation(Vec2::splat(scale), 0.2, at_now));
            let stats = pair.frame(&scene, view, &format!("instance {step} at scale {scale}"));
            assert!(stats.patched_items > 0, "step {step}: {stats:?}");
        }
    });
}

/// What cannot be patched is made whole, and drawn right: an instance that comes into the area that was kept (it has no items), a
/// group that is hidden or faded, a bone, a group with a gradient inside, an instance whose overrides changed.
#[test]
fn a_node_that_did_more_than_move_or_has_no_items_makes_the_frame_whole() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, groups, instances, _) = moving_scene();
        // An instance far outside the area kept for the view: it has no items.
        let far = scene.place_instance(match &scene.find(instances[0]).unwrap().kind {
            NodeKind::Instance(i) => i.symbol,
            _ => unreachable!(),
        }, Vec2::new(5000.0, 5000.0)).unwrap();
        let mut pair = Pair::new(gpu);
        let view = at(2.0, 0.0, 0.0);
        pair.frame(&scene, view, "first");

        // It moves a little and is still far: nothing to draw, and its (missing) items are no reason not to patch.
        shift(&mut scene, far, Vec2::new(10.0, 0.0));
        let still_far = pair.frame(&scene, view, "the far instance moved a little");
        assert!(still_far.patched_items == 0 && !still_far.reused_frame || still_far.patched_items == 0, "{still_far:?}");

        // It comes into view: its items are made by the whole frame.
        scene.find_mut(far).unwrap().set_transform(Affine2::from_translation(Vec2::new(40.0, 60.0)));
        let arrived = pair.frame(&scene, view, "the far instance arrives");
        assert_eq!(arrived.patched_items, 0, "an instance that has no items cannot be patched: {arrived:?}");
        assert!(arrived.items > 0 && !arrived.reused_frame);
        // ... and now it can.
        shift(&mut scene, far, Vec2::new(1.0, 1.0));
        assert_eq!(pair.frame(&scene, view, "and moves on").patched_items, 5);

        // A group that is hidden, one that is faded, one that is hidden and comes back.
        scene.find_mut(groups[3]).unwrap().set_visible(false);
        assert_eq!(pair.frame(&scene, view, "a group hidden").patched_items, 0);
        scene.find_mut(groups[3]).unwrap().set_visible(true);
        assert_eq!(pair.frame(&scene, view, "and back").patched_items, 0);
        scene.find_mut(groups[5]).unwrap().set_opacity(0.5);
        assert_eq!(pair.frame(&scene, view, "a group faded").patched_items, 0);
        scene.find_mut(groups[5]).unwrap().set_opacity(1.0);
        pair.frame(&scene, view, "and back to full opacity");

        // A group with a gradient inside it moves: gradients are not patched.
        if let Some(mut node) = scene.find_mut(groups[2]) {
            if let NodeKind::Group(children) = &mut node.kind {
                if let NodeKind::Shape(shape) = &mut children[0].kind {
                    shape.fill = Some(Paint::linear(Vec2::ZERO, Vec2::X, Color::BLACK, Color::WHITE));
                }
            }
        }
        pair.frame(&scene, view, "a gradient in a group");
        shift(&mut scene, groups[2], Vec2::new(1.0, 1.0));
        assert_eq!(pair.frame(&scene, view, "that group moved").patched_items, 0);

        // The overrides of an instance change.
        if let Some(mut node) = scene.find_mut(instances[2]) {
            if let NodeKind::Instance(instance) = &mut node.kind {
                instance.fill = Some(Color::rgb(250, 30, 30));
            }
        }
        assert_eq!(pair.frame(&scene, view, "an instance recolored").patched_items, 0);
        shift(&mut scene, instances[2], Vec2::new(1.0, 0.0));
        assert_eq!(pair.frame(&scene, view, "and moved").patched_items, 5, "its overrides are in the rows it gets");
    });
}

/// Moving things at random, groups, instances and shapes together: every frame is the picture a frame made whole draws.
#[test]
fn moving_groups_instances_and_shapes_at_random_gives_the_picture_a_frame_made_whole_draws_every_frame() {
    Gpu::for_each_test_mode(|gpu| {
        for seed in 1..=3u64 {
            let (mut scene, groups, instances, first) = moving_scene();
            let mut dice = Dice(seed * 104729);
            let mut pair = Pair::new(gpu);
            let mut patched_frames = 0;
            let mut pan = 0.0f32;
            for frame in 0..100 {
                let mut log = String::new();
                for _ in 0..1 + dice.next(4) {
                    match dice.next(9) {
                        0 | 1 => {
                            let (g, by) = (dice.next(groups.len()), Vec2::new(dice.next(7) as f32 - 3.0, dice.next(7) as f32 - 3.0));
                            log += &format!("[group {g} by {by}] ");
                            shift(&mut scene, groups[g], by)
                        }
                        2..=4 => {
                            let (k, by) = (dice.next(instances.len()), Vec2::new(dice.next(9) as f32 - 4.0, dice.next(9) as f32 - 4.0));
                            log += &format!("[instance {k} by {by}] ");
                            shift(&mut scene, instances[k], by)
                        }
                        5 => {
                            // An instance to another size (another bucket) and back.
                            let (k, scale, turn) = (dice.next(instances.len()), [0.4, 1.0, 3.5][dice.next(3)], dice.next(6) as f32);
                            log += &format!("[instance {k} to scale {scale}] ");
                            let mut node = scene.find_mut(instances[k]).unwrap();
                            let at_now = node.transform.translation;
                            node.set_transform(Affine2::from_scale_angle_translation(Vec2::splat(scale), turn, at_now));
                        }
                        6 => {
                            let (k, color) = (dice.next(first.len()), Color::rgb(dice.next(255) as u8, 100, dice.next(255) as u8));
                            log += &format!("[shape {k} recolored] ");
                            scene.find_mut(first[k]).map(|mut n| n.set_solid_fill_color(color));
                        }
                        7 => {
                            // An instance far away and back.
                            let k = dice.next(instances.len());
                            let to = if dice.next(2) == 0 { Vec2::new(9000.0, 9000.0) } else { Vec2::new(dice.next(150) as f32, 60.0 + dice.next(50) as f32) };
                            log += &format!("[instance {k} to {to}] ");
                            scene.find_mut(instances[k]).unwrap().set_transform(Affine2::from_translation(to));
                        }
                        _ => {
                            pan = (pan + dice.next(7) as f32 - 3.0).clamp(-20.0, 20.0);
                            log += &format!("[pan {pan}] ");
                        }
                    }
                }
                let stats = pair.frame_close(&scene, at(2.0, pan, 0.0), &format!("seed {seed}, frame {frame}: {log}"));
                patched_frames += (stats.patched_items > 0) as usize;
            }
            assert!(patched_frames >= 20, "seed {seed}: only {patched_frames} of 100 frames were patched");
        }
    });
}

/// A bone is a group, and a bone that moves moves the shapes bound to it, wherever they are in the tree: it is not a node that only moved.
#[test]
fn a_bone_that_moves_makes_the_frame_whole_because_it_bends_shapes_elsewhere() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, _, _, _) = moving_scene();
        let base = scene.add_bone(None, Vec2::new(20.0, 100.0), 0.0, 40.0);
        let tip = scene.add_bone(Some(base), Vec2::new(60.0, 100.0), 0.0, 40.0);
        let mut points = Vec::new();
        for i in 0..9 {
            points.push(Vec2::new(20.0 + i as f32 * 10.0, 96.0));
        }
        for i in (0..9).rev() {
            points.push(Vec2::new(20.0 + i as f32 * 10.0, 104.0));
        }
        let path = Path { subpaths: vec![SubPath { anchors: points.into_iter().map(Anchor::corner).collect(), closed: true }] };
        let arm = scene.new_node("arm", NodeKind::Shape(Shape { path, stroke: None, ..rect_shape(0.0, 0.0) }));
        let arm = scene.add(arm, None);
        assert!(scene.bind_skin(arm, &[base, tip]));
        let mut pair = Pair::new(gpu);
        let view = at(2.0, 0.0, 0.0);
        pair.frame(&scene, view, "first");
        for (step, angle) in [0.4f32, 0.9, -0.5].into_iter().enumerate() {
            scene.set_bone_world(tip, Vec2::new(60.0, 100.0), angle, None);
            let stats = pair.frame(&scene, view, &format!("the tip bone at {angle}"));
            assert_eq!(stats.patched_items, 0, "step {step}: {stats:?}");
            assert_eq!(stats.shapes_tessellated, 1, "step {step}: only the skinned shape is made again: {stats:?}");
        }
    });
}

/// With no time left, a shape that only needs another zoom bucket keeps its old triangles for now and the frame has work waiting. A group
/// that moved and grew into another bucket is then made by the full walk (which says so), not patched; once the triangles are made it can
/// be patched again.
#[test]
fn a_group_that_needs_triangles_the_frame_had_no_time_for_is_not_patched() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let circles: Vec<_> = (0..10)
            .map(|i| {
                let mut shape = Shape::new(Path::ellipse(Vec2::new(10.0 + (i % 5) as f32 * 12.0, 10.0 + (i / 5) as f32 * 12.0), Vec2::splat(4.0)));
                shape.fill = Some(Paint::Solid(Color::rgb(200, 90, 40)));
                shape.stroke = Some(Stroke { paint: Paint::Solid(Color::BLACK), width: 1.0, join: LineJoin::Round, cap: LineCap::Round, miter_limit: 4.0 });
                scene.new_node(format!("c{i}"), NodeKind::Shape(shape))
            })
            .collect();
        let group = scene.new_node("circles", NodeKind::Group(circles));
        let group = scene.add(group, None);
        let more = scene.new_node("more", NodeKind::Shape(rect_shape(100.0, 100.0)));
        scene.add(more, None);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.set_tessellation_budget(Some(0.0));
        let view = at(1.0, 0.0, 0.0);
        renderer.render(&scene, view, SIZE, &options()).unwrap();
        // The group grows by 4 about its corner: its circles are two zoom buckets finer now.
        let grown = Affine2::from_scale(Vec2::splat(4.0)) * Affine2::from_translation(Vec2::new(-4.0, -4.0));
        scene.find_mut(group).unwrap().set_transform(grown);
        let stats = renderer.render(&scene, view, SIZE, &options()).unwrap();
        assert_eq!(stats.patched_items, 0, "it needs triangles that were put off: {stats:?}");
        assert!(stats.deferred > 0 && !stats.reused_frame, "{stats:?}");
        // The time comes back: the triangles are made, and the group can be patched again.
        renderer.set_tessellation_budget(None);
        let mut last = stats;
        for _ in 0..30 {
            if last.deferred == 0 {
                break;
            }
            last = renderer.render(&scene, view, SIZE, &options()).unwrap();
        }
        assert_eq!(last.deferred, 0);
        scene.find_mut(group).unwrap().set_transform(Affine2::from_translation(Vec2::new(1.0, 1.0)) * grown);
        let moved = renderer.render(&scene, view, SIZE, &options()).unwrap();
        assert_eq!(moved.patched_items, 20, "{moved:?}");
    });
}

/// The shapes of a group that moved and was made again for another zoom bucket narrow what the frame allows of a change of zoom, as
/// a shape that was edited does.
#[test]
fn a_group_that_moved_into_another_bucket_narrows_the_zoom_the_frame_can_be_drawn_for() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = circles(4.0);
        let ids = scene.all_ids();
        // The circles around the middle of the picture in a group of their own, so that the group is what moves.
        // (The circles are in a grid of 40 columns, 10 units apart: columns 16 to 24 of rows 19 and 20 are around (205, 205).)
        let middle: Vec<NodeId> = (16..25).flat_map(|column| [19 * 40 + column, 20 * 40 + column]).map(|i| ids[i]).collect();
        let inside: Vec<_> = middle.iter().map(|id| scene.remove(*id).expect("a circle")).collect();
        let group = scene.new_node("some circles", NodeKind::Group(inside));
        let group = scene.add(group, None);
        let mut pair = Pair::new(gpu);
        pair.frame(&scene, centered(4.0, 205.0, 205.0), "first");
        // Half the size about the group's own corner: the circles are drawn at scale 2.2 now (bucket 1, which holds from 1.8), so the zoom
        // may go down by 1.8 / 2.2 = 0.82 and not by the 0.45 that the other circles allow. (They stay where they are: the group's
        // move is only a scale, about the middle of the picture.)
        let about_middle = Affine2::from_translation(Vec2::splat(205.0)) * Affine2::from_scale(Vec2::splat(0.55)) * Affine2::from_translation(Vec2::splat(-205.0));
        scene.find_mut(group).unwrap().set_transform(about_middle);
        let patched = pair.frame(&scene, centered(4.0, 205.0, 205.0), "the group grew smaller");
        assert!(patched.patched_items > 0, "{patched:?}");
        let small = pair.frame(&scene, centered(4.0 * 0.85, 205.0, 205.0), "a small zoom out");
        assert!(small.reused_frame, "{small:?}");
        let big = pair.frame(&scene, centered(4.0 * 0.8, 205.0, 205.0), "a zoom the made-again circles mind");
        assert!(!big.reused_frame, "the circles of the group are below their bucket: {big:?}");
    });
}

/// A group that reaches out of the area kept for the view has items only for the part that is in it; moving it brings those up to date
/// (and makes no triangles for the part that is out).
#[test]
fn a_group_that_reaches_out_of_the_area_kept_is_patched_for_the_part_that_is_in_it() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = Scene::default();
        let row: Vec<_> = (0..60).map(|i| scene.new_node(format!("r{i}"), NodeKind::Shape(rect_shape(i as f32 * 8.0, 20.0)))).collect();
        let group = scene.new_node("row", NodeKind::Group(row));
        let group = scene.add(group, None);
        let mut pair = Pair::new(gpu);
        // The picture shows x from 0 to 160 and the area kept reaches 72 units past it (144 pixels at 2 pixels a unit).
        let view = at(2.0, 0.0, 0.0);
        let first = pair.frame(&scene, view, "first");
        assert!(first.items > 40 && first.items < 120, "only part of the row has items: {first:?}");
        shift(&mut scene, group, Vec2::new(1.0, 1.0));
        let moved = pair.frame(&scene, view, "the row moved");
        assert_eq!(moved.patched_items, first.items, "every item there is got its row, and nothing else was made: {moved:?}");
        assert_eq!(moved.shapes_tessellated, 0, "{moved:?}");
    });
}

/// `scene` with a camera node at the end of its list.
fn with_camera(mut scene: Scene) -> (Scene, NodeId) {
    let camera = scene.new_camera(Vec2::new(300.0, 200.0), Vec2::new(150.0, 100.0));
    let id = scene.add(camera, None);
    (scene, id)
}

/// A game moves and zooms its camera in every frame. The camera draws nothing, and when its frame is not part of the picture it must
/// leave the frame to be drawn from the last one, for zooms and moves and for an edit made in the same frame.
/// A name or a script is nothing that is drawn: a frame where one shape moved and another was renamed patches the moved one's items, and is
/// not made whole for the rename (a game that keeps a count in a node's name, or the editor renaming while something plays).
#[test]
fn a_rename_next_to_a_move_changes_nothing_that_is_drawn() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, ids) = grid(100);
        let mut pair = Pair::new(gpu);
        assert!(!pair.frame(&scene, Affine2::IDENTITY, "first").reused_frame);
        scene.find_mut(ids[3]).unwrap().set_transform(Affine2::from_translation(Vec2::new(1.0, 0.0)));
        *scene.find_mut(ids[7]).unwrap().name_mut() = "renamed".into();
        scene.find_mut(ids[9]).unwrap().set_script(Some("fn on_update(me, dt) {}".into()));
        let stats = pair.frame(&scene, Affine2::IDENTITY, "a move, a rename and a script");
        assert!(!stats.reused_frame, "{stats:?}");
        assert_eq!((stats.patched_items, stats.shapes_tessellated), (2, 0), "the two items of the shape that moved, and nothing else: {stats:?}");
        // Only a name: nothing is made, patched or sent.
        *scene.find_mut(ids[8]).unwrap().name_mut() = "again".into();
        let stats = pair.frame(&scene, Affine2::IDENTITY, "a rename");
        assert_eq!((stats.patched_items, stats.shapes_tessellated, stats.chunks_rebuilt, stats.table_bytes_uploaded, stats.geometry_bytes_uploaded), (0, 0, 0, 0, 0), "{stats:?}");
    });
}

#[test]
fn a_camera_that_moves_or_zooms_does_not_make_the_frame_whole_unless_its_frame_is_in_the_picture() {
    Gpu::for_each_test_mode(|gpu| {
        let (mut scene, camera) = with_camera(circles(4.0));
        let mut pair = Pair::new(gpu);
        assert!(!pair.frame(&scene, centered(4.0, 100.0, 100.0), "first").reused_frame);
        for (step, zoom) in [4.2, 3.8, 4.1, 3.9, 4.3].into_iter().enumerate() {
            scene.find_mut(camera).unwrap().set_transform(Affine2::from_translation(Vec2::new(150.0 + step as f32 * 3.0, 100.0)));
            scene.find_mut(camera).unwrap().set_camera_size(Vec2::new(300.0 - step as f32 * 5.0, 200.0));
            let stats = pair.frame(&scene, centered(zoom, 100.0 + step as f32, 100.0), &format!("step {step}"));
            assert!(stats.reused_frame, "step {step}: the camera moved and was resized: {stats:?}");
        }
        // An edit made while the camera moves is brought up to date in the frame, as without the camera.
        let shape = scene.all_ids()[410]; // at (105, 105), in the view
        scene.find_mut(shape).unwrap().set_solid_fill_color(Color::rgb(10, 200, 30));
        scene.find_mut(camera).unwrap().set_transform(Affine2::from_translation(Vec2::new(170.0, 100.0)));
        // (An edit is brought up to date at the scale the frame was made for.)
        let stats = pair.frame(&scene, centered(4.0, 105.0, 100.0), "edit");
        assert!(!stats.reused_frame && stats.patched_items >= 1 && stats.patched_items <= 2, "the shape's items only: {stats:?}");
        assert_eq!(stats.shapes_tessellated, 0, "{stats:?}");
        // Anything else that changes while the camera does makes the frame whole.
        scene.find_mut(shape).unwrap().set_visible(false);
        let stats = pair.frame(&scene, centered(4.0, 105.0, 100.0), "hidden");
        assert!(!stats.reused_frame && stats.patched_items == 0, "{stats:?}");

        // With the frame in the picture the camera is drawn, and a move of it is a new picture.
        let with_frame = RenderOptions { camera_frame: true, ..options() };
        let view = centered(4.0, 100.0, 100.0);
        let mut renderer = Renderer::new(gpu).unwrap();
        renderer.render(&scene, view, SIZE, &with_frame).unwrap();
        scene.find_mut(camera).unwrap().set_transform(Affine2::from_translation(Vec2::new(190.0, 100.0)));
        let stats = renderer.render(&scene, view, SIZE, &with_frame).unwrap();
        assert!(!stats.reused_frame && stats.patched_items == 0, "{stats:?}");
        let mut fresh = Renderer::new(gpu).unwrap();
        fresh.render(&scene, view, SIZE, &with_frame).unwrap();
        assert!(picture_bytes(gpu, &renderer) == picture_bytes(gpu, &fresh), "and it is the picture of the camera where it is now");
    });
}
