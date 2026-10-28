//! Where the time of an editor frame goes with the real map (`local/ironkrieg/ironkrieg.evolve`): each pane timed by itself, then
//! panning, zooming, dragging a shape and playing. A tool, not a test: it is ignored by default and needs the map.
//!
//!     PROFILE_GPU=1 cargo test --release -p evolve_editor profile_tests -- --ignored --nocapture
//!
//! Without `PROFILE_GPU` the CPU renderer is used. With it, the GPU renderer draws for real on this machine's card for a few
//! seconds (GPU_SAFETY.md E3: it is small, but say so before running it on someone else's machine). The numbers are the
//! editor's own work, without the card's presenting and the window system. It found the quadratic overlays and the budget
//! problem of 2026-10-16 (BENCHMARKS.md).

use crate::app::EditorApp;
use crate::renderers::{RendererChoice, RendererSettings};
use eframe::egui;
use evolve_engine::geometry::Vec2;
use evolve_gpu::egui_glue::GpuContext;
use std::sync::Arc;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn run(ctx: &egui::Context, f: impl FnMut(&mut egui::Ui)) -> f64 {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
    let t = Instant::now();
    let mut output = ctx.run_ui(input, f);
    output.textures_delta.clear();
    ms(t)
}

/// How far from the origin the scenes are (plan 5.5): the box of the drawing in scene units, for each scene we have.
#[test]
#[ignore]
fn how_big_the_coordinates_of_the_scenes_are() {
    let mut scenes: Vec<(String, evolve_engine::scene::Scene)> = crate::bench_scene::KINDS.iter().map(|kind| (kind.to_string(), crate::bench_scene::generate(kind).expect("a scene").scenes.remove(0))).collect();
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    if map.exists() {
        scenes.push(("map".into(), evolve_engine::io::load_project(&map).unwrap().scenes.remove(0)));
    }
    for (name, scene) in scenes {
        let b = scene.selection_bounds(&scene.all_ids()).expect("something");
        println!("{name}: from ({:.1}, {:.1}) to ({:.1}, {:.1}), farthest corner {:.0} units from the origin", b.min.x, b.min.y, b.max.x, b.max.y, b.min.abs().max(b.max.abs()).max_element());
    }
}

/// What each kind of frame costs the GPU renderer's CPU side (plan 5.1), on the stress scene, the illustration scene and the map
/// (when it exists): a frame where nothing changed, a pan, a zoom in and out, one shape moved, one shape's outline edited. For each:
/// the average over a run of frames of the whole preparation and its stages (making triangles, finding the items, rows and
/// chunks), the worst frame, and what was made or sent. A few seconds on the card.
///
///     cargo test --release -p evolve_editor the_cost_of_each_kind_of_frame -- --ignored --nocapture
#[test]
#[ignore]
fn the_cost_of_each_kind_of_frame() {
    use evolve_engine::geometry::Affine2;
    use evolve_engine::scene::Scene;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    println!("GPU: {}", gpu.info.name);
    let size = (1920u32, 1080u32);
    let mut scenes: Vec<(String, Scene)> = crate::bench_scene::KINDS.iter().map(|kind| (kind.to_string(), crate::bench_scene::generate(kind).expect("a scene").scenes.remove(0))).collect();
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    if map.exists() {
        scenes.push(("map".into(), evolve_engine::io::load_project(&map).unwrap().scenes.remove(0)));
    }
    for (name, mut scene) in scenes {
        let bounds = scene.selection_bounds(&scene.all_ids()).expect("something to draw");
        let fit = (size.0 as f32 / bounds.size().x).min(size.1 as f32 / bounds.size().y);
        let center = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
        let middle = bounds.center();
        let view = move |zoom: f32, shift: f32| Affine2::from_translation(center + Vec2::new(shift, 0.0)) * Affine2::from_scale(Vec2::splat(fit * zoom)) * Affine2::from_translation(-middle);
        let options = RenderOptions { camera_frame: false, skip_unsupported: true, cull: true, targets: TargetPolicy::default() };
        let mut renderer = Renderer::new(&gpu).unwrap();
        for _ in 0..4 {
            renderer.render(&scene, view(1.0, 0.0), size, &options).unwrap();
        }
        gpu.wait_idle();
        // A shape near the middle of the drawing order that has a path, to move and to edit.
        let target = scene.all_ids().into_iter().filter(|id| matches!(scene.find(*id).map(|n| &n.kind), Some(evolve_engine::scene::NodeKind::Shape(_)))).nth(1000).expect("a shape");
        println!("--- {name}");
        println!("{:<22} {:>9} {:>8} {:>8} {:>8} {:>8} {:>9} {:>7} {:>8} {:>10}", "frame", "prepare", "cache", "collect", "list", "worst", "made", "chunks", "table kB", "geometry kB");
        type Step = Box<dyn Fn(&mut Scene, usize) -> Affine2>;
        let steps: Vec<(&str, usize, Step)> = vec![
            ("same view again", 30, Box::new(move |_, _| view(1.0, 0.0))),
            ("pan 3 px", 30, Box::new(move |_, i| view(1.0, 3.0 * (i + 1) as f32))),
            ("zoom in 2%", 30, Box::new(move |_, i| view(1.02f32.powi(i as i32 + 1), 0.0))),
            ("zoom out 2%", 30, Box::new(move |_, i| view(1.02f32.powi(30 - i as i32 - 1), 0.0))),
            (
                "move one shape",
                30,
                Box::new(move |scene, i| {
                    scene.find_mut(target).unwrap().set_transform(Affine2::from_translation(Vec2::new(2.0 * (i + 1) as f32, 0.0)));
                    view(1.0, 0.0)
                }),
            ),
            (
                "edit one outline",
                30,
                Box::new(move |scene, i| {
                    if let Some(path) = scene.find_mut(target).unwrap().path_mut() {
                        if let Some(anchor) = path.subpaths.first_mut().and_then(|s| s.anchors.first_mut()) {
                            anchor.pos.x += 0.5 + i as f32 * 0.01;
                        }
                    }
                    view(1.0, 0.0)
                }),
            ),
        ];
        for (label, frames, step) in steps {
            let mut total = [0.0f64; 4];
            let (mut worst, mut made, mut chunks, mut table, mut geometry) = (0.0f64, 0usize, 0usize, 0u64, 0u64);
            for i in 0..frames {
                let v = step(&mut scene, i);
                let stats = renderer.render(&scene, v, size, &options).unwrap();
                for (k, x) in [stats.update_ms, stats.tessellate_ms, stats.collect_ms, stats.list_ms].into_iter().enumerate() {
                    total[k] += x;
                }
                worst = worst.max(stats.update_ms);
                (made, chunks, table, geometry) = (made + stats.shapes_tessellated, chunks + stats.chunks_rebuilt, table + stats.table_bytes_uploaded, geometry + stats.geometry_bytes_uploaded);
                if i % 6 == 5 {
                    gpu.wait_idle();
                }
            }
            gpu.wait_idle();
            let n = frames as f64;
            println!(
                "{label:<22} {:>8.2}m {:>7.2}m {:>7.2}m {:>7.2}m {:>7.1}m {:>9.1} {:>7.1} {:>8.1} {:>10.1}",
                total[0] / n, total[1] / n, total[2] / n, total[3] / n, worst, made as f64 / n, chunks as f64 / n, table as f64 / n / 1024.0, geometry as f64 / n / 1024.0
            );
        }
    }
}

/// Where the CPU time of a GPU frame goes with the whole map in view, stage by stage, while the view is panned a little each
/// frame (so a few shapes come and go, as in the editor). A short run on the card (about 10 seconds); needs the map.
///
///     cargo test --release -p evolve_editor where_the_time_of_a_gpu_frame -- --ignored --nocapture
#[test]
#[ignore]
fn where_the_time_of_a_gpu_frame_goes_with_the_map() {
    use evolve_engine::geometry::Affine2;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    if !map.exists() {
        println!("no map");
        return;
    }
    let project = evolve_engine::io::load_project(&map).unwrap();
    let scene = &project.scenes[0];
    let size = (1600u32, 900u32);
    let bounds = scene.selection_bounds(&scene.all_ids()).unwrap();
    let fit = (size.0 as f32 / bounds.size().x).min(size.1 as f32 / bounds.size().y) * 1.2;
    let options = RenderOptions { camera_frame: false, skip_unsupported: true, cull: true, targets: TargetPolicy::default() };
    let mut renderer = Renderer::new(&gpu).unwrap();
    let view_at = |shift: f32| Affine2::from_translation(Vec2::new(size.0 as f32 * 0.5 + shift, size.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(fit)) * Affine2::from_translation(-bounds.center());
    for _ in 0..4 {
        renderer.render(scene, view_at(0.0), size, &options).unwrap();
    }
    gpu.wait_idle();
    let mut total = [0.0f64; 5];
    let frames = 40;
    for i in 0..frames {
        let stats = renderer.render(scene, view_at(i as f32 * 3.0), size, &options).unwrap();
        for (k, v) in [stats.update_ms, stats.tessellate_ms, stats.collect_ms, stats.list_ms, stats.record_ms].into_iter().enumerate() {
            total[k] += v;
        }
        if i % 8 == 7 {
            gpu.wait_idle();
        }
    }
    gpu.wait_idle();
    let n = frames as f64;
    println!(
        "pan over the map, {frames} frames, averages in ms: prepare {:.2} = cache {:.2} + collect {:.2} + list {:.2} + the rest {:.2}; record and submit {:.2}",
        total[0] / n, total[1] / n, total[2] / n, total[3] / n, (total[0] - total[1] - total[2] - total[3]) / n, total[4] / n
    );
}

/// The two generated benchmark scenes (the stress scene and the illustration scene, which has layers, clips and instances) drawn
/// completely by the GPU renderer at the three benchmark zooms: what a frame is made of and how long it takes (CPU time to get
/// it ready, and the card's own time from the submit until it reported the work done). A short run (a few seconds).
///
///     cargo test --release -p evolve_editor where_the_time_of_a_gpu_frame_goes_with_the_generated -- --ignored --nocapture
#[test]
#[ignore]
fn where_the_time_of_a_gpu_frame_goes_with_the_generated_scenes() {
    use evolve_engine::geometry::Affine2;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    let size = (1920u32, 1080u32);
    println!("GPU: {}", gpu.info.name);
    for kind in crate::bench_scene::KINDS {
        let scene = crate::bench_scene::generate(kind).expect("a scene").scenes.remove(0);
        let bounds = scene.selection_bounds(&scene.all_ids()).expect("something to draw");
        let fit = (size.0 as f32 / bounds.size().x).min(size.1 as f32 / bounds.size().y);
        for (name, factor) in [("whole", 1.0f32), ("region", 3.16), ("close-up", 14.0)] {
            let view = Affine2::from_translation(Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(fit * factor)) * Affine2::from_translation(-bounds.center());
            let options = RenderOptions { camera_frame: false, skip_unsupported: true, cull: true, targets: TargetPolicy::default() };
            let mut renderer = Renderer::new(&gpu).unwrap();
            let first = renderer.render(&scene, view, size, &options).unwrap();
            gpu.wait_idle();
            let (mut prepare, mut card, mut stats) = (0.0, 0.0, first.clone());
            let frames = 12;
            for _ in 0..frames {
                stats = renderer.render(&scene, view, size, &options).unwrap();
                gpu.wait_idle();
                prepare += stats.update_ms + stats.record_ms;
                card += renderer.last_completion_ms().unwrap_or(0.0);
            }
            println!(
                "{kind}, {name}: first frame {:.0} ms; steady frame: CPU {:.1} ms, card {:.1} ms; {} items, {} draw calls, {} passes ({} layers, {} reduced, {:.1} Mpixel of layers), skipped {:?}",
                first.update_ms + first.record_ms, prepare / frames as f64, card / frames as f64, stats.items, stats.draw_calls, stats.passes, stats.layers,
                stats.layers_reduced, stats.layer_pixels as f64 / 1e6, stats.skipped
            );
        }
    }
}

/// Zooms into the map step by step with one long-lived renderer, and at each step draws the same view with a fresh renderer:
/// any large difference is state that went wrong between frames (a province whose fill vanished while zooming, 2026-10-16:
/// the geometry store wrote zeros over live vertices). Draws on the card for about 20 seconds; needs the map.
///
///     cargo test --release -p evolve_editor zooming_into_the_map -- --ignored --nocapture
///
/// With `EVOLVE_DUMP_PICTURES=<folder>` the first bad step is saved as three PNG files (long-lived, fresh, CPU renderer).
#[test]
#[ignore]
fn zooming_into_the_map_draws_what_a_fresh_renderer_draws() {
    use evolve_engine::geometry::Affine2;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    if !map.exists() {
        println!("no map");
        return;
    }
    let project = evolve_engine::io::load_project(&map).unwrap();
    let scene = &project.scenes[0];
    let size = (800u32, 600u32);
    let bounds = scene.selection_bounds(&scene.all_ids()).unwrap();
    let fit = (size.0 as f32 / bounds.size().x).min(size.1 as f32 / bounds.size().y);
    let curved = scene.all_ids().iter().filter(|&&id| matches!(&scene.find(id).unwrap().kind, evolve_engine::scene::NodeKind::Shape(s) if evolve_gpu::tessellate::depends_on_scale(s))).count();
    println!("{} nodes, {curved} depend on scale", scene.all_ids().len());
    let options = RenderOptions { camera_frame: false, skip_unsupported: true, cull: true, targets: TargetPolicy::exact() };
    let mut long_lived = Renderer::new(&gpu).unwrap();
    let mut bad_steps = 0;
    let mut dumped = false;
    let mut worst = 0;
    for (spot, focus) in [bounds.center(), bounds.min + bounds.size() * 0.3, bounds.min + bounds.size() * Vec2::new(0.7, 0.4)].into_iter().enumerate() {
        let mut zoom = fit;
        for step in 0..60 {
            let view = Affine2::from_translation(Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(-focus);
            let stats = long_lived.render(scene, view, size, &options).unwrap();
            let a = gpu.read_texture_part(long_lived.picture().unwrap(), 0, 0, size.0, size.1);
            let mut fresh = Renderer::new(&gpu).unwrap();
            let fresh_stats = fresh.render(scene, view, size, &options).unwrap();
            let b = gpu.read_texture_part(fresh.picture().unwrap(), 0, 0, size.0, size.1);
            let far = a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 40)).count();
            if !stats.warnings.is_empty() {
                println!("spot {spot} step {step} zoom {zoom:.3}: warnings {:?}", &stats.warnings[..stats.warnings.len().min(5)]);
            }
            if far > 1000 && std::env::var("EVOLVE_DUMP_PICTURES").is_ok() && !dumped {
                dumped = true;
                let folder = std::path::PathBuf::from(std::env::var("EVOLVE_DUMP_PICTURES").unwrap());
                let mut cpu = evolve_engine::tiny_skia::Pixmap::new(size.0, size.1).unwrap();
                evolve_engine::render::render(scene, view, &mut cpu);
                let _ = cpu.save_png(folder.join(format!("zoom-{spot}-{step}-cpu.png")));
                for (name, data) in [("long", &a), ("fresh", &b)] {
                    let pixmap = evolve_engine::tiny_skia::Pixmap::from_vec(data.clone(), evolve_engine::tiny_skia::IntSize::from_wh(size.0, size.1).unwrap()).unwrap();
                    let _ = pixmap.save_png(folder.join(format!("zoom-{spot}-{step}-{name}.png")));
                }
            }
            worst = worst.max(far);
            if far > 0 {
                bad_steps += 1;
                println!("spot {spot} step {step} zoom {zoom:.3}: {far} pixels differ from a fresh renderer (items {} vs fresh {}, triangles {} vs {}, culled {} vs {}, compacted {}/{})", stats.items, fresh_stats.items, stats.triangles, fresh_stats.triangles, stats.shapes_culled, fresh_stats.shapes_culled, stats.vertices_compacted, stats.indices_compacted);
            }
            drop(fresh);
            gpu.wait_idle();
            zoom *= if step < 40 { 1.15 } else { 1.0 / 1.4 };
        }
    }
    println!("{bad_steps} steps differ, at most {worst} pixels");
    gpu.wait_idle();
    // A few hundred pixels along borders come from the 10% a bucket is kept past its edge (a renderer that has been zooming
    // can be one bucket behind a new one for a moment); a lost shape is tens of thousands.
    assert!(worst < 1000, "a step differs by {worst} pixels from a fresh renderer: something was lost between frames");
}

#[test]
#[ignore]
fn where_the_time_of_a_frame_goes_with_the_map() {
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    if !map.exists() {
        println!("no map at {}: see BENCHMARKS.md for how to make it", map.display());
        return;
    }
    let t = Instant::now();
    let project = evolve_engine::io::load_project(&map).unwrap();
    println!("load: {:.0} ms", ms(t));
    let t = Instant::now();
    let mut app = EditorApp::from_project(project, 0);
    println!("from_project: {:.0} ms", ms(t));
    let ctx = egui::Context::default();

    let gpu = if std::env::var("PROFILE_GPU").is_ok() { evolve_gpu::Gpu::headless() } else { None };
    if let Some(gpu) = &gpu {
        let format = eframe::wgpu::TextureFormat::Rgba8Unorm;
        let renderer = eframe::egui_wgpu::Renderer::new(&gpu.device, format, eframe::egui_wgpu::RendererOptions::default());
        app.renderers = RendererSettings::with_context(RendererChoice::Gpu, GpuContext::new(gpu.clone(), Arc::new(egui::mutex::RwLock::new(renderer)), format));
        println!("GPU: {}", gpu.info.name);
    } else {
        println!("CPU renderer only");
    }

    for round in 0..4 {
        let objects = run(&ctx, |ui| app.objects_pane(ui));
        let inspector = run(&ctx, |ui| app.inspector_pane(ui));
        let scene_view = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        let game_view = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(600.0, 500.0), |ui| app.game_pane(ui));
        });
        let t = Instant::now();
        app.after_frame(&ctx);
        let after = ms(t);
        // a pan: the camera moves, nothing else
        app.camera.center += Vec2::new(30.0, 0.0);
        let pan = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        println!("round {round}: objects {objects:.1}  inspector {inspector:.1}  scene view {scene_view:.1}  game view {game_view:.1}  after_frame {after:.1}  scene view after a pan {pan:.1}");
        if let Some(gpu) = &gpu {
            gpu.wait_idle();
        }
    }
    if let Some(gpu) = &gpu {
        gpu.wait_idle();
    }

    // zooming: a new zoom every frame
    let mut worst: f64 = 0.0;
    let mut total = 0.0;
    for _ in 0..20 {
        let center = app.camera.viewport * 0.5;
        app.camera.zoom_at(center, 1.15);
        let t = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        worst = worst.max(t);
        total += t;
    }
    println!("notice: {:?}", app.canvas.scene_target.notice());
    println!("zoom in, 20 frames: average {:.1}  worst {worst:.1}  (zoom now {:.2})", total / 20.0, app.camera.zoom);
    worst = 0.0;
    total = 0.0;
    for _ in 0..20 {
        let center = app.camera.viewport * 0.5;
        app.camera.zoom_at(center, 1.0 / 1.15);
        let t = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        worst = worst.max(t);
        total += t;
    }
    println!("notice: {:?}", app.canvas.scene_target.notice());
    println!("zoom out, 20 frames: average {:.1}  worst {worst:.1}", total / 20.0);

    // select a shape and drag it: its transform changes every frame, and the editor's whole frame runs
    let id = app.scene.all_ids()[1000];
    app.selection = vec![id];
    let mut total = [0.0f64; 6];
    let mut worst_frame: f64 = 0.0;
    for i in 0..20 {
        let before = app.scene.find(id).unwrap().transform;
        app.scene.find_mut(id).unwrap().set_transform(before * evolve_engine::geometry::Affine2::from_translation(Vec2::new(3.0, 1.0)));
        let a = run(&ctx, |ui| app.objects_pane(ui));
        let b = run(&ctx, |ui| app.inspector_pane(ui));
        let c = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        let d = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(600.0, 500.0), |ui| app.game_pane(ui));
        });
        let t = Instant::now();
        app.after_frame(&ctx); // while the mouse is held the undo check does not run; this is the quiet case
        let e = ms(t);
        for (k, v) in [a, b, c, d, e, a + b + c + d + e].into_iter().enumerate() {
            total[k] += v;
        }
        worst_frame = worst_frame.max(a + b + c + d + e);
        if i == 0 {
            println!("first dragged frame: objects {a:.1} inspector {b:.1} scene view {c:.1} game view {d:.1} after_frame {e:.1}");
        }
    }
    println!(
        "drag, 20 frames, averages: objects {:.1} inspector {:.1} scene view {:.1} game view {:.1} after_frame {:.1} = {:.1} (worst frame {worst_frame:.1})",
        total[0] / 20.0, total[1] / 20.0, total[2] / 20.0, total[3] / 20.0, total[4] / 20.0, total[5] / 20.0
    );

    // play mode
    app.selection.clear();
    let t = Instant::now();
    app.start_game();
    println!("start game: {:.0} ms", ms(t));
    let mut total = [0.0f64; 4];
    let mut worst_frame: f64 = 0.0;
    let before = (app.canvas.scene_target.gpu_counts(), app.canvas.game_target.gpu_counts());
    for _ in 0..30 {
        let t = Instant::now();
        app.update_game(&ctx);
        let u = ms(t);
        let c = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
        });
        let d = run(&ctx, |ui| {
            ui.allocate_ui(egui::vec2(600.0, 500.0), |ui| app.game_pane(ui));
        });
        let t = Instant::now();
        app.after_frame(&ctx);
        let e = ms(t);
        for (k, v) in [u, c, d, e].into_iter().enumerate() {
            total[k] += v;
        }
        worst_frame = worst_frame.max(u + c + d + e);
    }
    println!("notice: {:?}", app.canvas.scene_target.notice());
    println!("play: (draws, idle) scene view {:?} -> {:?}, game view {:?} -> {:?}", before.0, app.canvas.scene_target.gpu_counts(), before.1, app.canvas.game_target.gpu_counts());
    println!(
        "play, 30 frames, averages: game update {:.1} scene view {:.1} game view {:.1} after_frame {:.1} = {:.1} (worst frame {worst_frame:.1})",
        total[0] / 30.0, total[1] / 30.0, total[2] / 30.0, total[3] / 30.0, total.iter().sum::<f64>() / 30.0
    );
    if let Some(gpu) = &gpu {
        gpu.wait_idle();
    }
}

/// What the layers of the illustration scene cost the card (plan 5.13): the same view drawn again and again at three zooms, with how
/// many passes and layers it has, how many pixels the layers take, and how long the card needs for a frame. A few seconds.
///
///     cargo test --release -p evolve_editor what_the_layers_of_the_illustration_scene_cost -- --ignored --nocapture
#[test]
#[ignore]
fn what_the_layers_of_the_illustration_scene_cost() {
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    println!("GPU: {}", gpu.info.name);
    let size = (1920u32, 1080u32);
    let scene = crate::bench_scene::generate("illustration").expect("a scene").scenes.remove(0);
    let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
    for (label, view) in crate::bench::benchmark_views(&scene) {
        let mut renderer = Renderer::new(&gpu).unwrap();
        renderer.set_frame_reuse(false);
        let mut stats = renderer.render(&scene, view, size, &options).unwrap();
        gpu.wait_idle();
        let started = Instant::now();
        for _ in 0..20 {
            stats = renderer.render(&scene, view, size, &options).unwrap();
        }
        gpu.wait_idle();
        let per_frame = ms(started) / 20.0;
        println!(
            "{label:<6} {:>7.2} ms a frame; items {:>6}, draw calls {:>4}, passes {:>4}, layers {:>4} ({} reduced, {} flattened), layer pixels {:>10} (the view is {})",
            per_frame, stats.items, stats.draw_calls, stats.passes, stats.layers, stats.layers_reduced, stats.layers_flattened, stats.layer_pixels, size.0 as u64 * size.1 as u64
        );
    }
}

/// What an animated frame costs (plan 6.4 and 6.6): every instance of the `sprites` scene is moved and turned a little in every frame,
/// as physics or a script would, and the frame is prepared and drawn. Reported: the time the animation itself takes (finding each
/// node by its id and writing its transform), the renderer's preparation, and the card's time per frame when the frames follow each
/// other. A few seconds on the card.
///
///     cargo test --release -p evolve_editor what_an_animated_frame_costs -- --ignored --nocapture
#[test]
#[ignore]
fn what_an_animated_frame_costs() {
    use evolve_engine::geometry::Affine2;
    use evolve_engine::scene::NodeKind;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    use evolve_gpu::targets::TargetPolicy;
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    println!("GPU: {}", gpu.info.name);
    let size = (1920u32, 1080u32);
    let mut scene = crate::bench_scene::generate("sprites").expect("a scene").scenes.remove(0);
    let bounds = scene.selection_bounds(&scene.all_ids()).expect("something to draw");
    let fit = (size.0 as f32 / bounds.size().x).min(size.1 as f32 / bounds.size().y);
    let view = Affine2::from_translation(Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(fit)) * Affine2::from_translation(-bounds.center());
    let options = RenderOptions { camera_frame: false, skip_unsupported: false, cull: true, targets: TargetPolicy::default() };
    let sprites: Vec<_> = scene.all_ids().into_iter().filter(|id| matches!(scene.find(*id).map(|n| &n.kind), Some(NodeKind::Instance(_)))).collect();
    println!("{} sprites, {} draw items", sprites.len(), 0);
    let mut renderer = Renderer::new(&gpu).unwrap();
    renderer.render(&scene, view, size, &options).unwrap();
    gpu.wait_idle();
    for moved in [sprites.len(), sprites.len() / 10, 100, 1] {
        let (mut driver, mut update, mut list, mut collect, mut cache) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let (mut patched, mut reused, mut table, mut geometry) = (0, 0, 0u64, 0u64);
        let frames = 40;
        let started = Instant::now();
        for frame in 0..frames {
            let t = Instant::now();
            for id in &sprites[..moved] {
                if let Some(mut node) = scene.find_mut(*id) {
                    let m = node.transform;
                    node.set_transform(Affine2::from_translation(Vec2::new(0.7, 0.3 * (frame as f32).sin())) * m);
                }
            }
            driver += ms(t);
            let stats = renderer.render(&scene, view, size, &options).unwrap();
            update += stats.update_ms;
            list += stats.list_ms;
            collect += stats.collect_ms;
            cache += stats.tessellate_ms;
            patched += stats.patched_items;
            reused += stats.reused_frame as usize;
            table += stats.table_bytes_uploaded;
            geometry += stats.geometry_bytes_uploaded;
            if frame % 8 == 7 {
                gpu.wait_idle();
            }
        }
        gpu.wait_idle();
        let n = frames as f64;
        println!(
            "{moved:>5} moved: animation {:>6.2} ms, prepare {:>6.2} ms (cache {:>5.2}, collect {:>5.2}, list {:>5.2}), whole frame with the card {:>6.2} ms; patched items {:.0}, reused {}, table {:.0} kB, geometry {:.0} kB a frame",
            driver / n, update / n, cache / n, collect / n, list / n, ms(started) / n, patched as f64 / n, reused, table as f64 / n / 1024.0, geometry as f64 / n / 1024.0
        );
    }
}

/// What a frame of the running IronKrieg game costs while the camera zooms and moves (a script that zooms and moves the camera): the script
/// and the runtime step, then the Game view's frame prepared and drawn. Needs `local/ironkrieg/ironkrieg.evolve`. A few seconds
/// on the card.
///
///     cargo test --release -p evolve_editor what_a_frame_of_the_running_map_costs -- --ignored --nocapture
#[test]
#[ignore]
fn what_a_frame_of_the_running_map_costs() {
    use evolve_engine::runtime::{Input, Runtime};
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    // PROBE_MAP names another project of local/ironkrieg (a made-up map for a measurement, say).
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg").join(std::env::var("PROBE_MAP").unwrap_or_else(|_| "ironkrieg.evolve".into()));
    let Ok(project) = evolve_engine::io::load_project(&path) else { return };
    println!("GPU: {}", gpu.info.name);
    let scene = project.scenes[0].clone();
    let options = RenderOptions { camera_frame: false, ..RenderOptions::default() };
    let mut renderer = Renderer::new(&gpu).unwrap();
    renderer.set_tessellation_budget(Some(evolve_gpu::profile::QualityProfile::High.tessellation_ms()));
    let mut runtime = Runtime::new(&scene);
    let pixels = Vec2::new(1280.0, 1041.0);
    let frame = |runtime: &mut Runtime, renderer: &mut Renderer, input: Input| {
        let t = Instant::now();
        runtime.update(1.0 / 60.0, input);
        let script = ms(t);
        let camera = runtime.scene().camera_view();
        let view = camera.view_matrix(pixels);
        let stats = renderer.render(&runtime.scene(), view, (pixels.x as u32, pixels.y as u32), &options).unwrap();
        (script, stats)
    };
    // Settle on the default zoom first.
    let mut settled = None;
    for _ in 0..180 {
        settled = Some(frame(&mut runtime, &mut renderer, Input::default()).1);
    }
    gpu.wait_idle();
    let (mut shapes, mut anchors) = (0usize, 0usize);
    runtime.scene().for_each_node(|node| {
        if let evolve_engine::scene::NodeKind::Shape(shape) = &node.kind {
            shapes += 1;
            anchors += shape.path.subpaths.iter().map(|sp| sp.anchors.len()).sum::<usize>();
        }
    });
    let settled = settled.expect("frames");
    println!("the map: {shapes} shapes, {anchors} anchors; at the default zoom {} items, {} triangles, {} draw calls", settled.items, settled.triangles, settled.draw_calls);
    let world = |runtime: &Runtime, p: Vec2| {
        let v = runtime.scene().camera_view();
        v.center + Vec2::new(p.x * v.size.x, p.y * v.size.y)
    };
    for scenario in ["idle", "zooming in and out", "panning in one direction"] {
        let (mut script, mut update, mut list, mut collect, mut cache) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let (mut reused, mut patched, mut worst, mut deferred) = (0, 0usize, 0.0f64, 0usize);
        let mut worst_stats = None;
        let (mut table, mut geometry) = (0u64, 0u64);
        let frames = 120;
        let mut pointer = Vec2::new(0.4, 0.0);
        if scenario.starts_with("pan") {
            let input = Input { mouse: world(&runtime, pointer), mouse_right_down: true, mouse_right_pressed: true, ..Default::default() };
            frame(&mut runtime, &mut renderer, input);
        }
        let started = Instant::now();
        for i in 0..frames {
            let mut input = Input::default();
            if scenario.starts_with("zoom") && i % 24 == 0 {
                input.wheel = if (i / 24) % 2 == 0 { 1.0 } else { -1.0 };
            }
            if scenario.starts_with("pan") {
                pointer.x -= 0.004;
                input.mouse = world(&runtime, pointer);
                input.mouse_right_down = true;
            }
            let (s, stats) = frame(&mut runtime, &mut renderer, input);
            script += s;
            if stats.update_ms > worst {
                worst = stats.update_ms;
                worst_stats = Some((i, stats.clone()));
            }
            deferred += (stats.deferred > 0) as usize;
            update += stats.update_ms;
            list += stats.list_ms;
            collect += stats.collect_ms;
            cache += stats.tessellate_ms;
            reused += stats.reused_frame as usize;
            patched += stats.patched_items;
            table += stats.table_bytes_uploaded;
            geometry += stats.geometry_bytes_uploaded;
            if i % 8 == 7 {
                gpu.wait_idle();
            }
        }
        gpu.wait_idle();
        if let Some((i, w)) = &worst_stats {
            println!(
                "   worst frame {i}: prepare {:.1} (cache {:.1}, collect {:.1}, list {:.1}), shapes made {}, reused {}, deferred {}, culled {}, items {}, draw calls {}, table {} kB, geometry {} kB, compacted {}/{}",
                w.update_ms, w.tessellate_ms, w.collect_ms, w.list_ms, w.shapes_tessellated, w.shapes_reused, w.deferred, w.shapes_culled, w.items, w.draw_calls, w.table_bytes_uploaded / 1024, w.geometry_bytes_uploaded / 1024, w.vertices_compacted, w.indices_compacted
            );
        }
        let n = frames as f64;
        println!(
            "{scenario:<26} runtime {:>6.2} ms, prepare {:>6.2} ms (cache {:>5.2}, collect {:>5.2}, list {:>5.2}), whole frame {:>6.2} ms (worst prepare {worst:.1}, deferred frames {deferred}); reused {reused}/{frames}, patched {:.0}, table {:.0} kB, geometry {:.0} kB a frame",
            script / n, update / n, cache / n, collect / n, list / n, ms(started) / n, patched as f64 / n, table as f64 / n / 1024.0, geometry as f64 / n / 1024.0
        );
    }
}

/// What a whole frame of the editor costs while the playable map runs and its camera zooms or moves (Game view with or without the Scene
/// view beside it): the game's update, the Game view, the Scene view and the bookkeeping, as the editor does them in a frame. Needs
/// `local/ironkrieg/ironkrieg.evolve`. A few seconds on the card.
///
///     cargo test --release -p evolve_editor what_a_frame_of_the_editor_costs_while_the_map_plays -- --ignored --nocapture
#[test]
#[ignore]
fn what_a_frame_of_the_editor_costs_while_the_map_plays() {
    // PROBE_MAP names another project of local/ironkrieg, as in what_a_frame_of_the_running_map_costs.
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg").join(std::env::var("PROBE_MAP").unwrap_or_else(|_| "ironkrieg.evolve".into()));
    let Ok(project) = evolve_engine::io::load_project(&map) else { return };
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    println!("GPU: {}", gpu.info.name);
    let format = eframe::wgpu::TextureFormat::Rgba8Unorm;
    for with_scene_view in [false, true] {
        let mut app = EditorApp::from_project(project.clone(), 0);
        app.renderers = RendererSettings::with_context(RendererChoice::Gpu, GpuContext::new(gpu.clone(), Arc::new(egui::mutex::RwLock::new(renderer_clone(&gpu, format))), format));
        let ctx = egui::Context::default();
        app.start_game();
        let pointer = egui::pos2(300.0, 250.0);
        let frame = |app: &mut EditorApp, events: Vec<egui::Event>| -> [f64; 4] {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1300.0))), events, ..Default::default() };
            let mut parts = [0.0f64; 4];
            let mut output = ctx.run_ui(raw, |ui| {
                let t = Instant::now();
                app.update_game(ui.ctx());
                parts[0] = ms(t);
                let t = Instant::now();
                ui.allocate_ui(egui::vec2(600.0, 500.0), |ui| app.game_pane(ui));
                parts[1] = ms(t);
                if with_scene_view {
                    let t = Instant::now();
                    ui.allocate_ui(egui::vec2(800.0, 700.0), |ui| app.canvas_ui(ui));
                    parts[2] = ms(t);
                }
            });
            output.textures_delta.clear();
            let t = Instant::now();
            app.after_frame(&ctx);
            parts[3] = ms(t);
            parts
        };
        for _ in 0..4 {
            frame(&mut app, vec![egui::Event::PointerMoved(pointer)]);
        }
        for _ in 0..180 {
            frame(&mut app, vec![]);
        }
        gpu.wait_idle();
        for scenario in ["idle", "zooming in and out", "panning in one direction"] {
            let mut total = [0.0f64; 4];
            let mut worst = 0.0f64;
            let frames = 120;
            let mut at = pointer;
            let started = Instant::now();
            for i in 0..frames {
                let mut events = Vec::new();
                if scenario.starts_with("zoom") && i % 24 == 0 {
                    events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, if (i / 24) % 2 == 0 { 1.0 } else { -1.0 }), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move });
                }
                if scenario.starts_with("pan") {
                    if i == 0 {
                        events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
                    }
                    at.x -= 2.0;
                    events.push(egui::Event::PointerMoved(at));
                }
                let parts = frame(&mut app, events);
                for (t, p) in total.iter_mut().zip(parts) {
                    *t += p;
                }
                worst = worst.max(parts.iter().sum::<f64>());
                if i % 8 == 7 {
                    gpu.wait_idle();
                }
            }
            gpu.wait_idle();
            if scenario.starts_with("pan") {
                frame(&mut app, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE }]);
            }
            let n = frames as f64;
            println!(
                "{} {scenario:<26} game update {:>5.2}  game view {:>5.2}  scene view {:>5.2}  after_frame {:>5.2} = {:>6.2} ms (worst {worst:.1}); wall {:.2} ms a frame",
                if with_scene_view { "game+scene" } else { "game only " },
                total[0] / n, total[1] / n, total[2] / n, total[3] / n, total.iter().sum::<f64>() / n, ms(started) / n
            );
        }
    }
}

fn renderer_clone(gpu: &evolve_gpu::Gpu, format: eframe::wgpu::TextureFormat) -> eframe::egui_wgpu::Renderer {
    eframe::egui_wgpu::Renderer::new(&gpu.device, format, eframe::egui_wgpu::RendererOptions::default())
}

/// How far the pin of the Objects panel zooms on the objects of the maps, right after the click and after the Scene view has been drawn.
///
///     cargo test --release -p evolve_editor how_far_the_pin_zooms_on_the_map -- --ignored --nocapture
#[test]
#[ignore]
fn how_far_the_pin_zooms_on_the_map() {
    for name in ["ironkrieg.evolve"] {
        println!("--- {name}");
        let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg").join(name);
        let Ok(project) = evolve_engine::io::load_project(&map) else { continue };
        let mut app = EditorApp::from_project(project, 0);
        let ctx = egui::Context::default();
        for _ in 0..2 {
            run(&ctx, |ui| {
                ui.allocate_ui(egui::vec2(1000.0, 700.0), |ui| app.canvas_ui(ui));
            });
        }
        let ids = app.scene.all_ids();
        let whole = app.scene.selection_bounds(&ids).unwrap();
        println!("the scene: {} objects, bounds {:?} .. {:?}; the Scene view is {:?} points, zoom {}", ids.len(), whole.min, whole.max, app.camera.viewport, app.camera.zoom);
        let mut sizes: Vec<(f32, evolve_engine::scene::NodeId)> = ids.iter().filter_map(|&id| app.scene.selection_bounds(&[id]).map(|b| (b.size().max_element(), id))).collect();
        sizes.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Every object, one by one: the views the pin would give, the widest first.
        let mut views: Vec<(f32, String, bool)> = Vec::new();
        let mut without_bounds = 0;
        for &id in &ids {
            if app.scene.selection_bounds(&[id]).is_none() {
                without_bounds += 1;
            }
            app.locate(id);
            let shown = app.camera.viewport / app.camera.zoom;
            let node = app.scene.find(id).unwrap();
            views.push((shown.x, node.name.clone(), node.is_group()));
        }
        views.sort_by(|a, b| b.0.total_cmp(&a.0));
        // For every object that is not tiny, the pin must give the view the plain "zoom to selection" gives (what it always gave).
        let (mut same, mut different, mut tiny) = (0, 0, 0);
        for &id in &ids {
            let b = app.scene.selection_bounds(&[id]).unwrap();
            if b.size().max_element() * 1.2 < whole.size().max_element() * crate::app::LOCATE_CONTEXT {
                tiny += 1;
                continue;
            }
            app.locate(id);
            let (center, zoom) = (app.camera.center, app.camera.zoom);
            app.selection = vec![id];
            app.frame_selection();
            if app.camera.center.distance(center) < 1e-2 && (app.camera.zoom - zoom).abs() < zoom * 1e-4 {
                same += 1;
            } else {
                different += 1;
                if different <= 8 {
                    println!("  different: {:?} size {:?}: pin center {center:?} zoom {zoom}, plain center {:?} zoom {}", app.scene.find(id).unwrap().name, b.size(), app.camera.center, app.camera.zoom);
                }
            }
        }
        println!("the pin against the plain zoom to selection: {same} the same, {different} different, {tiny} tiny ones that get the least view");
        println!("{without_bounds} objects have no bounds; the widest views: {:?}", &views[..8]);
        let widest_plain = views.iter().find(|v| !v.2).unwrap();
        println!("the widest view for an object that is not a group: {widest_plain:?}");
        for q in [0.0, 0.1, 0.5, 0.9, 0.99, 1.0] {
            let (size, id) = sizes[((sizes.len() - 1) as f32 * q) as usize];
            app.locate(id);
            let right_after = app.camera.viewport / app.camera.zoom;
            for _ in 0..3 {
                run(&ctx, |ui| {
                    ui.allocate_ui(egui::vec2(1000.0, 700.0), |ui| app.canvas_ui(ui));
                });
            }
            let later = app.camera.viewport / app.camera.zoom;
            println!("object at {:>4.0}%: largest side {:>9.2} {:?}: view right after {:?}, after drawing {:?}", q * 100.0, size, app.scene.find(id).unwrap().name, right_after, later);
        }
    }
}

/// What the Scene view's picture looks like in the frames right after a jump of the camera from the whole map to an object (the pin): how
/// much of the picture is covered, frame by frame, for a few sizes of view.
///
///     cargo test --release -p evolve_editor what_the_picture_looks_like_after_a_jump -- --ignored --nocapture
#[test]
#[ignore]
fn what_the_picture_looks_like_after_a_jump() {
    use evolve_engine::geometry::Affine2;
    use evolve_gpu::renderer::{RenderOptions, Renderer};
    let Some(gpu) = evolve_gpu::Gpu::headless() else { return };
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    let Ok(project) = evolve_engine::io::load_project(&map) else { return };
    let scene = project.scenes[0].clone();
    let options = RenderOptions { camera_frame: false, ..RenderOptions::default() };
    let size = (1000u32, 700u32);
    let ids = scene.all_ids();
    let target = *ids.iter().find(|&&id| scene.find(id).is_some_and(|n| n.name.starts_with("Border 128-604 "))).unwrap();
    let center = scene.selection_bounds(&[target]).unwrap().center();
    println!("target {center:?}");
    let view_at = |zoom: f32, center: Vec2| Affine2::from_translation(Vec2::new(size.0 as f32 / 2.0, size.1 as f32 / 2.0)) * Affine2::from_scale(Vec2::splat(zoom)) * Affine2::from_translation(-center);
    for width_km in [1.2f32, 12.0, 49.0, 170.0, 1000.0] {
        let mut renderer = Renderer::new(&gpu).unwrap();
        renderer.set_tessellation_budget(Some(4.0));
        let whole = view_at(0.0754, Vec2::new(5135.0, 4177.0));
        for _ in 0..3 {
            renderer.render(&scene, whole, size, &options).unwrap();
        }
        gpu.wait_idle();
        let zoom = size.0 as f32 / width_km;
        let mut line = format!("view {width_km:>6} km (zoom {zoom:>8.2}):");
        for frame in 0..4 {
            let stats = renderer.render(&scene, view_at(zoom, center), size, &options).unwrap();
            gpu.wait_idle();
            let picture = gpu.read_texture_part(renderer.picture().unwrap(), 0, 0, size.0, size.1);
            let covered = picture.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count() as f64 / (size.0 * size.1) as f64;
            if frame == 1 {
                let pixels = picture.as_chunks::<4>().0;
                let dark = pixels.iter().filter(|p| p[3] > 0 && (p[0] as u32 + p[1] as u32 + p[2] as u32) < 60).count() as f64 / pixels.len() as f64;
                let mean = |i: usize| pixels.iter().map(|p| p[i] as f64).sum::<f64>() / pixels.len() as f64;
                println!("   [{width_km} km: dark pixels {:.1}%, mean color ({:.0}, {:.0}, {:.0}), alpha {:.0}]", dark * 100.0, mean(0), mean(1), mean(2), mean(3));
            }
            line += &format!("  frame {frame}: covered {:>5.1}% items {} made {} deferred {} update {:.1} ms;", covered * 100.0, stats.items, stats.shapes_tessellated, stats.deferred, stats.update_ms);
        }
        println!("{line}");
    }
}
