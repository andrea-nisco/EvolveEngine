//! The Scene and Game views with the GPU renderer: what is drawn once, what is reused, what falls back to the CPU renderer
//! and says so, and what happens when the graphics device is lost or reports an error (plan 2.9, 2.10, 2.11, 2.17).
//!
//! The real panes are run headless (no window); the GPU renderer draws for real on the test device.

use crate::app::EditorApp;
use crate::renderers::{RendererChoice, RendererSettings};
use eframe::egui;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::scene::{NodeKind, Scene, Shape};
use evolve_engine::style::{Color, Paint, Stroke};
use evolve_engine::Vec2;
use evolve_gpu::egui_glue::GpuContext;
use evolve_gpu::Gpu;
use std::sync::Arc;

fn context(gpu: &Gpu) -> GpuContext {
    let format = eframe::wgpu::TextureFormat::Rgba8Unorm;
    let renderer = eframe::egui_wgpu::Renderer::new(&gpu.device, format, eframe::egui_wgpu::RendererOptions::default());
    GpuContext::new(gpu.clone(), Arc::new(egui::mutex::RwLock::new(renderer)), format)
}

/// A scene the GPU renderer can draw completely: a camera and a few solid shapes.
fn plain_scene() -> Scene {
    let mut scene = Scene::with_camera(Vec2::new(400.0, 300.0));
    for i in 0..30 {
        let mut shape = Shape::new(Path::ellipse(Vec2::new(40.0 + i as f32 * 11.0, 60.0 + (i % 5) as f32 * 40.0), Vec2::new(14.0, 9.0)));
        shape.fill = Some(Paint::Solid(Color::rgb(40 + i as u8 * 6, 90, 200 - i as u8 * 5)));
        shape.stroke = Some(Stroke::solid(Color::rgba(0, 0, 0, 200), 2.0));
        let node = scene.new_node(format!("s{i}"), NodeKind::Shape(shape));
        scene.add(node, None);
    }
    scene
}

fn app_with(scene: Scene, renderers: RendererSettings) -> EditorApp {
    let mut app = EditorApp::from_project(evolve_engine::project::Project::single(scene), 0);
    app.renderers = renderers;
    app
}

/// Runs the Scene pane and the Game pane for one frame, as the editor does while nobody does anything.
fn frame(app: &mut EditorApp, ctx: &egui::Context) {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 900.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        ui.allocate_ui(egui::vec2(480.0, 380.0), |ui| app.canvas_ui(ui));
        ui.allocate_ui(egui::vec2(480.0, 380.0), |ui| app.game_pane(ui));
    });
    output.textures_delta.clear(); // there is no window to hand them to
}

#[test]
fn a_scene_the_gpu_can_draw_is_drawn_once_and_idle_frames_reuse_the_picture() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx); // the first frame lays things out
        let stamps = app.scene.revision();
        let before = (app.canvas.scene_target.gpu_counts(), app.canvas.game_target.gpu_counts());
        for _ in 0..5 {
            frame(&mut app, &ctx);
        }
        let (scene_view, game_view) = (app.canvas.scene_target.gpu_counts().expect("the Scene view uses the GPU"), app.canvas.game_target.gpu_counts().expect("so does the Game view"));
        assert!(app.canvas.scene_target.notice().is_none() && app.canvas.game_target.notice().is_none(), "no fallback, no notice");
        let (scene_before, game_before) = (before.0.unwrap(), before.1.unwrap());
        assert_eq!(scene_view.0, scene_before.0, "five more idle frames drew the Scene view no more times");
        assert_eq!(game_view.0, game_before.0, "nor the Game view");
        assert_eq!((scene_view.1 - scene_before.1, game_view.1 - game_before.1), (5, 5), "all five reused the picture");
        assert!(scene_view.0 >= 1 && game_view.0 >= 1);
        assert!(app.scene.changed_since(&stamps).is_empty(), "painting the views with the GPU renderer does not change the scene");
    });
}

#[test]
fn editing_the_scene_draws_again_and_nothing_else_does() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        frame(&mut app, &ctx);
        let drawn = |app: &EditorApp| (app.canvas.scene_target.gpu_counts().unwrap().0, app.canvas.game_target.gpu_counts().unwrap().0);
        let before = drawn(&app);
        let id = app.scene.all_ids()[3];
        app.scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(25.0, 0.0)));
        frame(&mut app, &ctx);
        let after = drawn(&app);
        assert_eq!((after.0 - before.0, after.1 - before.1), (1, 1), "one more draw for each view");
        frame(&mut app, &ctx);
        assert_eq!(drawn(&app), after, "and then idle again");
        // Moving the editor camera is a change for the Scene view only (the Game view looks through the game camera).
        app.camera.zoom_at(Vec2::new(100.0, 100.0), 1.3);
        frame(&mut app, &ctx);
        assert_eq!(drawn(&app).0, after.0 + 1);
        assert_eq!(drawn(&app).1, after.1, "the Game view did not move");
    });
}

#[test]
fn the_sample_project_with_its_skinned_tentacle_is_drawn_by_the_gpu_renderer_with_no_notice() {
    Gpu::for_each_test_mode(|gpu| {
        // The sample project has a skinned, IK-driven tentacle; the GPU renderer draws it from its bent outline (Phase 6).
        let project = crate::sample::project();
        let mut app = EditorApp::from_project(project, crate::sample::OPEN_AT_START);
        app.renderers = RendererSettings::with_context(RendererChoice::Gpu, context(gpu));
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx);
        }
        assert!(app.canvas.scene_target.notice().is_none(), "{:?}", app.canvas.scene_target.notice());
        assert!(app.canvas.scene_target.gpu_counts().is_some_and(|c| c.0 > 0), "the GPU drew it");
        assert!(app.canvas.game_target.notice().is_none());
        // The same scene with the CPU choice has no notice either, and the GPU is not asked.
        app.renderers.choice = RendererChoice::Cpu;
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_none());
    });
}

#[test]
fn the_cpu_choice_never_touches_the_graphics_card() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Cpu, context(gpu)));
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut app, &ctx);
        }
        assert!(app.canvas.scene_target.gpu_counts().is_none() && app.canvas.game_target.gpu_counts().is_none());
        assert!(app.canvas.scene_target.notice().is_none());
        assert_eq!(gpu.alloc.live_resources(), 0, "not a single buffer or texture was made for the views");
    });
}

#[test]
fn switching_the_renderer_while_the_editor_runs_works_both_ways() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Cpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        for choice in [RendererChoice::Gpu, RendererChoice::Cpu, RendererChoice::Auto, RendererChoice::Gpu] {
            app.renderers.choice = choice;
            frame(&mut app, &ctx);
            frame(&mut app, &ctx);
            assert!(app.canvas.scene_target.notice().is_none(), "{choice}: a plain scene needs no fallback");
        }
        let drawn = app.canvas.scene_target.gpu_counts().expect("it used the GPU while that was the choice").0;
        assert!(drawn >= 1);
    });
}

#[test]
fn without_a_graphics_device_the_gpu_choice_falls_back_and_says_so() {
    let mut app = app_with(plain_scene(), RendererSettings::without_gpu(RendererChoice::Gpu));
    let ctx = egui::Context::default();
    for _ in 0..2 {
        frame(&mut app, &ctx);
    }
    assert_eq!(app.canvas.scene_target.notice(), Some("CPU renderer: no usable graphics card"));
    assert!(app.canvas.scene_target.gpu_counts().is_none());
}

/// GPU_SAFETY.md D3: a lost device (driver reset, sleep) is survivable.
#[test]
fn a_lost_device_switches_the_gpu_renderer_off_and_the_views_go_on_with_the_cpu_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_none());

        gpu.errors.record_loss("simulated driver reset".into());
        let message = app.renderers.poll().expect("the loss is noticed");
        assert!(message.contains("lost") && message.contains("simulated"), "{message}");
        assert!(app.renderers.poll().is_none(), "and told only once");
        assert!(app.renderers.usable_context().is_none());
        for _ in 0..3 {
            frame(&mut app, &ctx); // does not panic, does not touch the lost device
        }
        let notice = app.canvas.scene_target.notice().expect("the view says why it uses the CPU renderer");
        assert!(notice.contains("lost"), "{notice}");
        // Choosing the CPU renderer by hand clears the notice, as for any CPU view.
        app.renderers.choice = RendererChoice::Cpu;
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_none());
        gpu.errors.clear_loss(); // the test device is shared; give it back as it was
    });
}

/// GPU_SAFETY.md D1: an error the card reports does not crash the editor either.
#[test]
fn an_error_reported_by_the_card_switches_the_gpu_renderer_off_without_a_crash() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        gpu.errors.record("Validation Error: something is wrong".into());
        let message = app.renderers.poll().expect("the error is noticed");
        assert!(message.contains("reported an error") && message.contains("Validation Error"), "{message}");
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_some_and(|n| n.contains("reported an error")));
    });
}

/// The quality chosen in the editor reaches the device: its memory caps are those of the profile.
#[test]
fn the_quality_chosen_in_the_editor_sets_the_memory_caps_of_the_device() {
    Gpu::for_each_test_mode(|gpu| {
        let mut settings = RendererSettings::with_context(RendererChoice::Gpu, context(gpu));
        let adapter = settings.adapter_profile().expect("a card");
        assert_eq!(gpu.alloc.budgets(), adapter.budgets());
        for profile in evolve_gpu::profile::QualityProfile::ALL {
            settings.set_quality(evolve_gpu::profile::Quality::Fixed(profile));
            assert_eq!(gpu.alloc.budgets(), profile.budgets(), "{profile}");
        }
        settings.set_quality(evolve_gpu::profile::Quality::Auto);
        assert_eq!(gpu.alloc.budgets(), adapter.budgets());
    });
}

#[test]
fn a_view_that_cannot_get_graphics_memory_falls_back_instead_of_failing() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        // (A new context applies the caps of the card's quality profile, so the small budget is set after it.)
        gpu.alloc.set_budgets(evolve_gpu::alloc::Budgets::high().with(evolve_gpu::alloc::Category::Geometry, 1000));
        let ctx = egui::Context::default();
        for _ in 0..2 {
            frame(&mut app, &ctx);
        }
        let notice = app.canvas.scene_target.notice().expect("it fell back");
        assert!(notice.contains("budget") || notice.contains("memory") || notice.contains("Geometry"), "{notice}");
        gpu.alloc.set_budgets(evolve_gpu::alloc::Budgets::high());
    });
}

#[test]
fn a_scene_that_becomes_unsupported_falls_back_at_once_and_comes_back_when_it_is_fixed() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_none());
        let drawn = app.canvas.scene_target.gpu_counts().unwrap().0;

        // The scene gets a clip group whose mask is skinned (not drawn by the GPU renderer): the next frame must not show a wrong picture.
        let mut skinned = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(5.0)));
        skinned.skin = Some(Box::new(evolve_engine::rig::Skin { bones: vec![], inverse_bind: vec![], bind_world: Affine2::IDENTITY, weights: vec![] }));
        let content = app.scene.new_node("content", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(9.0)))));
        let mask = app.scene.new_node("skinned mask", NodeKind::Shape(skinned));
        let mut clip = app.scene.new_node("clip", NodeKind::Group(vec![content, mask]));
        clip.clip = true;
        let skinned = app.scene.add(clip, None);
        frame(&mut app, &ctx);
        let notice = app.canvas.scene_target.notice().expect("the gate noticed the change");
        assert!(notice.contains("skinned shapes"), "{notice}");
        assert_eq!(app.canvas.scene_target.gpu_counts().unwrap().0, drawn, "the GPU did not draw the scene with the skinned mask");

        // And back: the GPU renderer takes over again.
        app.scene.remove(skinned);
        frame(&mut app, &ctx);
        assert!(app.canvas.scene_target.notice().is_none());
        assert!(app.canvas.scene_target.gpu_counts().unwrap().0 > drawn);
    });
}

#[test]
fn a_camera_that_moves_every_frame_draws_the_view_again_but_does_not_make_the_gate_look_through_the_scene() {
    Gpu::for_each_test_mode(|gpu| {
        let mut app = app_with(plain_scene(), RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let (gates, draws) = (app.canvas.game_target.gpu_gates().unwrap(), app.canvas.game_target.gpu_counts().unwrap().0);
        let camera = app.scene.main_camera().expect("a camera");
        for step in 1..=5 {
            app.scene.find_mut(camera).unwrap().set_transform(Affine2::from_translation(Vec2::new(200.0 + step as f32 * 7.0, 150.0)));
            frame(&mut app, &ctx);
        }
        assert_eq!(app.canvas.game_target.gpu_gates().unwrap(), gates, "only a camera changed: the gate has nothing new to look at");
        assert_eq!(app.canvas.game_target.gpu_counts().unwrap().0, draws + 5, "but the Game view looks elsewhere, so it is drawn each time");
        // A change to a shape does make the gate look again.
        let shape = app.scene.all_ids()[1];
        app.scene.find_mut(shape).unwrap().set_opacity(0.5);
        frame(&mut app, &ctx);
        assert_eq!(app.canvas.game_target.gpu_gates().unwrap(), gates + 1);
    });
}

#[test]
fn the_game_view_puts_the_cameras_frame_under_the_picture_of_the_card() {
    Gpu::for_each_test_mode(|gpu| {
        let mut scene = plain_scene();
        let camera = scene.main_camera().unwrap();
        if let NodeKind::Camera(data) = &mut scene.find_mut(camera).unwrap().kind {
            data.background = Color::rgb(11, 22, 33);
        }
        let mut app = app_with(scene, RendererSettings::with_context(RendererChoice::Gpu, context(gpu)));
        let ctx = egui::Context::default();
        let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 900.0))), ..Default::default() };
        for _ in 0..2 {
            let mut output = ctx.run_ui(raw(), |ui| {
                ui.allocate_ui(egui::vec2(480.0, 380.0), |ui| app.game_pane(ui));
            });
            output.textures_delta.clear();
            let frames = output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Path(p) if p.fill == egui::Color32::from_rgb(11, 22, 33))).count();
            assert_eq!(frames, 1, "the camera's frame in its background color, once");
        }
    });
}

#[test]
fn the_cameras_frame_is_painted_under_the_gpu_picture_in_the_cameras_background_color_and_where_the_camera_is() {
    use crate::canvas::paint_camera_frame;
    let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
    let camera = scene.main_camera().unwrap();
    if let NodeKind::Camera(data) = &mut scene.find_mut(camera).unwrap().kind {
        data.background = Color::rgb(10, 20, 30);
    }
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(50.0, 40.0), egui::vec2(400.0, 200.0));
    let painted = |scene: &Scene, view: &dyn Fn(Vec2) -> Affine2| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
        let mut output = ctx.run_ui(raw, |ui| paint_camera_frame(&ui.painter().clone(), rect, scene, view));
        output.textures_delta.clear();
        output.shapes.iter().filter_map(|s| if let egui::Shape::Path(p) = &s.shape { Some((p.points.clone(), p.fill)) } else { None }).collect::<Vec<_>>()
    };
    // The view that shows exactly the frame: it covers the whole rectangle.
    let polygons = painted(&scene, &|pixels| scene.camera_view().view_matrix(pixels));
    assert_eq!(polygons.len(), 1);
    let (points, fill) = &polygons[0];
    assert_eq!(*fill, egui::Color32::from_rgb(10, 20, 30));
    assert_eq!(*points, vec![egui::pos2(50.0, 40.0), egui::pos2(450.0, 40.0), egui::pos2(450.0, 240.0), egui::pos2(50.0, 240.0)]);
    // On a screen with twice the pixels per point the same rectangle of points is covered.
    ctx.set_pixels_per_point(2.0);
    let polygons = painted(&scene, &|pixels| scene.camera_view().view_matrix(pixels));
    assert_eq!(polygons[0].0, *points, "the view is given pixels, the polygon is in points");
    ctx.set_pixels_per_point(1.0);
    // A view that shows the world 100 units to the right of the frame's left edge: the frame starts 200 points to the left of the rectangle.
    let polygons = painted(&scene, &|pixels| Affine2::from_scale(pixels / Vec2::new(200.0, 100.0)) * Affine2::from_translation(Vec2::new(-100.0, 0.0)));
    assert_eq!(polygons[0].0[0], egui::pos2(50.0 - 200.0, 40.0));
    assert_eq!(polygons[0].0[1], egui::pos2(50.0 + 200.0, 40.0));
}
