//! Tests of the egui glue: whole egui frames drawn with egui-wgpu into offscreen textures, on every kind of device. Run with
//! the cargo feature `egui` (the editor turns it on, so `cargo test --workspace` includes them).

use crate::alloc::Category;
use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::draw_list::Unsupported;
use crate::egui_glue::{GpuContext, GpuView, Painted};
use crate::renderer::{RenderError, RenderOptions};
use egui_wgpu::ScreenDescriptor;
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::render;
use evolve_engine::scene::{NodeKind, Scene, Shape};
use evolve_engine::style::{Color, Paint};
use evolve_engine::tiny_skia::Pixmap;
use evolve_engine::Vec2;
use std::sync::Arc;

const SIZE: (u32, u32) = (160, 120);

fn context(gpu: &Gpu, format: wgpu::TextureFormat) -> GpuContext {
    let renderer = egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
    GpuContext::new(gpu.clone(), Arc::new(egui::mutex::RwLock::new(renderer)), format)
}

/// Runs one egui frame whose contents `build` paints, draws it with egui-wgpu into an offscreen texture of `format`, and
/// returns the bytes of that texture.
fn frame(context: &GpuContext, format: wgpu::TextureFormat, build: impl FnOnce(&egui::Painter, egui::Rect)) -> Vec<u8> {
    let gpu = &context.gpu;
    let egui_context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32));
    let input = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
    let mut build = Some(build);
    let mut output = egui_context.run_ui(input, |ui| {
        if let Some(build) = build.take() {
            build(&ui.painter().clone(), rect);
        }
    });
    let paint_jobs = egui_context.tessellate(output.shapes, 1.0);
    let target = gpu
        .alloc
        .create_texture(
            Category::Other,
            &wgpu::TextureDescriptor {
                label: Some("window"),
                size: wgpu::Extent3d { width: SIZE.0, height: SIZE.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        )
        .expect("a window-sized texture");
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let screen = ScreenDescriptor { size_in_pixels: [SIZE.0, SIZE.1], pixels_per_point: 1.0 };
    let mut egui_renderer = context.egui_renderer().write();
    for (id, deltas) in &output.textures_delta.set {
        for delta in deltas {
            egui_renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
    let extra = egui_renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &paint_jobs, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        egui_renderer.render(&mut pass, &paint_jobs, &screen);
    }
    gpu.queue.submit(extra.into_iter().chain([encoder.finish()]));
    for id in &output.textures_delta.free {
        egui_renderer.free_texture(id);
    }
    drop(egui_renderer);
    output.textures_delta.clear(); // applied above
    gpu.read_texture_part(&target, 0, 0, SIZE.0, SIZE.1)
}

fn rect_scene(color: Color) -> Scene {
    let mut scene = Scene::default();
    let mut shape = Shape::new(Path::rect(Vec2::new(-10.0, -10.0), Vec2::new(SIZE.0 as f32 + 10.0, SIZE.1 as f32 + 10.0)));
    shape.fill = Some(Paint::Solid(color));
    shape.stroke = None;
    let node = scene.new_node("fill", NodeKind::Shape(shape));
    scene.add(node, None);
    scene
}

fn options() -> RenderOptions {
    RenderOptions { camera_frame: false, ..RenderOptions::default() }
}

const FORMATS: [wgpu::TextureFormat; 2] = [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb];

/// Task 2.7. A picture of 50% gray must reach the window as 128, on a window format that is not sRGB (egui's usual choice)
/// and on one that is; otherwise colors come out washed out or too dark.
#[test]
fn a_gray_picture_reaches_the_window_unchanged_on_both_kinds_of_window_format() {
    Gpu::for_each_test_mode(|gpu| {
        for format in FORMATS {
            let context = context(gpu, format);
            let mut view = GpuView::new(&context, "Test view").unwrap();
            let scene = rect_scene(Color::rgb(128, 128, 128));
            let pixels = frame(&context, format, |painter, rect| {
                let painted = view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap();
                assert!(matches!(painted, Painted::Drawn(_)));
            });
            for (x, y) in [(5u32, 5u32), (80, 60), (150, 110)] {
                let at = ((y * SIZE.0 + x) * 4) as usize;
                for channel in 0..3 {
                    let v = pixels[at + channel] as i32;
                    assert!((v - 128).abs() <= 1, "{format:?} ({}): pixel ({x},{y}) channel {channel} is {v}, not 128", gpu.mode.name());
                }
                assert_eq!(pixels[at + 3], 255);
            }
        }
    });
}

/// What the GPU picture looks like in the window must be what the CPU picture looks like there, shown by egui's own image
/// path: the same colors, also where the picture is partly transparent over a background.
#[test]
fn the_gpu_picture_looks_in_the_window_like_the_cpu_picture_does() {
    Gpu::for_each_test_mode(|gpu| {
        for format in FORMATS {
            let context = context(gpu, format);
            let mut scene = rect_scene(Color::rgba(220, 40, 30, 255));
            let mut shape = Shape::new(Path::ellipse(Vec2::new(80.0, 60.0), Vec2::new(50.0, 40.0)));
            shape.fill = Some(Paint::Solid(Color::rgba(30, 90, 220, 140)));
            shape.stroke = Some(evolve_engine::style::Stroke::solid(Color::rgba(10, 10, 10, 200), 6.0));
            let node = scene.new_node("ellipse", NodeKind::Shape(shape));
            scene.add(node, None);
            let background = egui::Color32::from_rgb(70, 120, 90);

            // The GPU way: egui paints the background, the paint callback shows our picture over it.
            let mut view = GpuView::new(&context, "Test view").unwrap();
            let gpu_pixels = frame(&context, format, |painter, rect| {
                painter.rect_filled(rect, 0.0, background);
                view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap();
            });

            // The CPU way, as the editor draws it today: the background in the bitmap, shown with egui's image shader.
            let mut pixmap = Pixmap::new(SIZE.0, SIZE.1).unwrap();
            let [r, g, b, _] = background.to_array();
            pixmap.fill(evolve_engine::tiny_skia::Color::from_rgba8(r, g, b, 255));
            render::render_content(&scene, Affine2::IDENTITY, &mut pixmap);
            let image = egui::ColorImage::from_rgba_premultiplied([SIZE.0 as usize, SIZE.1 as usize], pixmap.data());
            let cpu_pixels = frame(&context, format, |painter, rect| {
                let texture = painter.ctx().load_texture("cpu view", image, egui::TextureOptions::NEAREST);
                painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            });

            let result = compare_pictures(&cpu_pixels, &gpu_pixels, SIZE.0, Rule::default());
            assert!(result.passes(), "{format:?} ({}): {result}", gpu.mode.name());
            // The colors must line up: a wrong color or a wrongly mixed alpha costs several levels of average. A software rasterizer
            // (WARP, llvmpipe) puts its anti-aliasing samples a little differently along the edges of the ellipse: 0.56 against 0.23
            // on a graphics card (measured on the CI runners and on an RTX 3060), so it gets the rule's own bound, which `passes` checks.
            let bound = if gpu.info.device_type == wgpu::DeviceType::Cpu { Rule::default().mean_difference } else { 0.5 };
            assert!(result.mean_difference <= bound, "colors line up: {result}");
        }
    });
}

#[test]
fn a_view_larger_than_it_needs_to_be_is_shown_without_stretching() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        // The targets are made 25% larger than the view; the picture must still be shown at its own size.
        let mut scene = Scene::default();
        let mut shape = Shape::new(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(40.0, 30.0)));
        shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
        shape.stroke = None;
        let node = scene.new_node("corner", NodeKind::Shape(shape));
        scene.add(node, None);
        for options in [options(), RenderOptions { targets: crate::targets::TargetPolicy::exact(), ..options() }] {
            let mut view = GpuView::new(&context, "Test view").unwrap();
            let pixels = frame(&context, format, |painter, rect| {
                view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options).unwrap();
            });
            let at = |x: u32, y: u32| ((y * SIZE.0 + x) * 4) as usize;
            assert_eq!(&pixels[at(20, 15)..at(20, 15) + 4], &[255, 0, 0, 255], "inside the rectangle");
            assert_eq!(&pixels[at(45, 15)..at(45, 15) + 4], &[0, 0, 0, 255], "right of it: the clear color, not red");
            assert_eq!(&pixels[at(20, 35)..at(20, 35) + 4], &[0, 0, 0, 255], "below it");
        }
    });
}

/// Task 2.10 and rules A3 and A4.
#[test]
fn a_frame_where_nothing_changed_draws_nothing_and_a_view_without_size_costs_nothing() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let mut view = GpuView::new(&context, "Test view").unwrap();
        let mut scene = rect_scene(Color::rgb(10, 200, 10));
        let paint = |view: &mut GpuView, scene: &Scene, camera: Affine2, rect_size: (f32, f32)| {
            let mut result = None;
            frame(&context, format, |painter, _| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(rect_size.0, rect_size.1));
                result = Some(view.paint(painter, rect, scene, |_| camera, &options()).unwrap());
            });
            result.unwrap()
        };
        let full = (SIZE.0 as f32, SIZE.1 as f32);
        let zoomed = Affine2::from_scale(Vec2::splat(2.0));
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, full), Painted::Drawn(_)), "the first paint draws");
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, full), Painted::Idle), "the same again is idle");
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, full), Painted::Idle));
        // A scene change of any kind, the camera, or the size, draw again.
        let id = scene.all_ids()[0];
        scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(3.0, 0.0)));
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, full), Painted::Drawn(_)), "a moved shape");
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, full), Painted::Idle));
        assert!(matches!(paint(&mut view, &scene, zoomed, full), Painted::Drawn(_)), "the camera");
        assert!(matches!(paint(&mut view, &scene, zoomed, (100.0, 100.0)), Painted::Drawn(_)), "the size");
        assert!(matches!(paint(&mut view, &scene, zoomed, (100.0, 100.0)), Painted::Idle));
        // A copy of the scene with the same stamps is the same picture.
        let copy = scene.clone();
        assert!(matches!(paint(&mut view, &copy, zoomed, (100.0, 100.0)), Painted::Idle));
        // No size: nothing at all.
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, (0.0, 50.0)), Painted::Nothing));
        assert!(matches!(paint(&mut view, &scene, Affine2::IDENTITY, (50.0, 0.0)), Painted::Nothing));
        // Asked to forget, it draws again.
        view.invalidate();
        assert!(matches!(paint(&mut view, &scene, zoomed, (100.0, 100.0)), Painted::Drawn(_)));
    });
}

#[test]
fn only_the_first_of_three_identical_frames_sends_anything_to_the_card() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let mut view = GpuView::new(&context, "Test view").unwrap();
        let scene = rect_scene(Color::rgb(10, 200, 10));
        let mut stats = Vec::new();
        for _ in 0..3 {
            frame(&context, format, |painter, rect| {
                if let Painted::Drawn(s) = view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap() {
                    stats.push(s);
                }
            });
        }
        assert_eq!(stats.len(), 1, "only the first of three identical frames drew");
        assert!(stats[0].table_bytes_uploaded > 0);
    });
}

#[test]
fn a_scene_the_gpu_cannot_draw_is_an_error_for_the_view_and_leaves_nothing_marked_as_shown() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let mut view = GpuView::new(&context, "Test view").unwrap();
        let mut scene = rect_scene(Color::rgb(10, 200, 10));
        // A clip group whose mask is skinned is not drawn (opacity, clips, gradients, instances and skinned shapes are).
        let mut skinned = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(5.0)));
        skinned.skin = Some(Box::new(evolve_engine::rig::Skin { bones: vec![], inverse_bind: vec![], bind_world: Affine2::IDENTITY, weights: vec![] }));
        let content = scene.new_node("content", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(9.0)))));
        let mask = scene.new_node("skinned mask", NodeKind::Shape(skinned));
        let mut clip = scene.new_node("clip", NodeKind::Group(vec![content, mask]));
        clip.clip = true;
        let id = scene.add(clip, None);
        frame(&context, format, |painter, rect| match view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()) {
            Err(RenderError::Unsupported(what)) => assert!(what.contains_key(&Unsupported::Skinned)),
            other => panic!("expected a refusal, got {other:?}"),
        });
        // Fixing the scene makes the next paint draw (not Idle).
        scene.remove(id);
        frame(&context, format, |painter, rect| {
            assert!(matches!(view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap(), Painted::Drawn(_)));
        });
    });
}

// --- Quality profiles (plan 5.8, 5.9) ------------------------------------------------------------------------------------------

use crate::profile::{Quality, QualityProfile};

/// A red rectangle in the top left corner of the view, 40 by 30 pixels, on a view that is otherwise empty.
fn corner_scene() -> Scene {
    let mut scene = Scene::default();
    let mut shape = Shape::new(Path::rect(Vec2::new(0.0, 0.0), Vec2::new(40.0, 30.0)));
    shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
    shape.stroke = None;
    let node = scene.new_node("corner", NodeKind::Shape(shape));
    scene.add(node, None);
    scene
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE.0 + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
fn the_size_a_picture_is_drawn_at_is_the_views_scaled_and_never_zero_nor_larger() {
    use crate::egui_glue::scaled;
    assert_eq!(scaled((160, 120), 1.0), (160, 120));
    assert_eq!(scaled((160, 120), 0.75), (120, 90));
    assert_eq!(scaled((1, 1), 0.75), (1, 1));
    assert_eq!(scaled((3, 3), 0.2), (1, 1));
    assert_eq!(scaled((160, 120), 2.0), (160, 120), "never larger than the view");
    assert_eq!(scaled((1920, 1080), 0.75), (1440, 810));
}

#[test]
fn a_lower_profile_draws_the_picture_smaller_and_shows_it_stretched_over_the_whole_view_and_the_best_one_draws_it_exactly() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let scene = corner_scene();
        let mut view = GpuView::new(&context, "Test view").unwrap();

        // Low: drawn at three quarters of the size, shown over the whole view.
        context.set_quality(Quality::Fixed(QualityProfile::Low));
        let low = frame(&context, format, |painter, rect| {
            let painted = view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap();
            assert!(matches!(painted, Painted::Drawn(_)));
        });
        assert_eq!(view.renderer().view_size(), (120, 90), "{}", gpu.mode.name());
        assert_eq!(view.profile(), QualityProfile::Low);
        let scale = if cfg!(target_arch = "wasm32") { 0.5 } else { 1.0 };
        assert_eq!(view.renderer().tessellation_budget(), Some(2.0 * scale), "Low spends less time on shapes that are not needed at once");
        assert_eq!(pixel(&low, 20, 15), [255, 0, 0, 255], "inside the rectangle");
        assert_eq!(pixel(&low, 60, 15), [0, 0, 0, 255], "right of it");
        assert_eq!(pixel(&low, 20, 45), [0, 0, 0, 255], "below it");
        // The rectangle is shown 40 by 30 pixels again, not 30 by 22: its edge is where it was, a little soft (the picture is
        // filtered when it is stretched).
        let edge = (0..SIZE.0).rev().find(|&x| pixel(&low, x, 10)[0] > 127).unwrap();
        assert!((38..=41).contains(&edge), "the right edge is at about 40, not at {edge}");
        assert!((0..SIZE.0).any(|x| (10..245).contains(&pixel(&low, x, 10)[0])), "and soft: a stretched picture is smoothed, not repeated texel by texel");

        // The edges of the view get the picture too: it is not left short by the slack of the texture it is drawn in.
        let full = full_red_view(&context, format);
        for (x, y) in [(0, 0), (SIZE.0 - 1, 0), (SIZE.0 - 1, SIZE.1 - 1), (0, SIZE.1 - 1), (SIZE.0 - 1, 60), (80, SIZE.1 - 1)] {
            assert_eq!(pixel(&full, x, y), [255, 0, 0, 255], "({x}, {y}) of a view filled with red, drawn at three quarters: {}", gpu.mode.name());
        }

        // High: the view's own size, texel for texel, as before.
        context.set_quality(Quality::Fixed(QualityProfile::High));
        let high = frame(&context, format, |painter, rect| {
            let painted = view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap();
            assert!(matches!(painted, Painted::Drawn(_)), "another profile is another picture: it is drawn, not shown again");
        });
        assert_eq!(view.renderer().view_size(), SIZE);
        assert_eq!(view.renderer().tessellation_budget(), Some(4.0 * scale));
        assert_eq!(pixel(&high, 39, 15), [255, 0, 0, 255]);
        assert_eq!(pixel(&high, 40, 15), [0, 0, 0, 255], "an exact edge");
        assert_eq!(pixel(&high, 20, 29), [255, 0, 0, 255]);
        assert_eq!(pixel(&high, 20, 30), [0, 0, 0, 255]);
    });
}

/// A view filled with red, drawn with whatever quality the context has.
fn full_red_view(context: &GpuContext, format: wgpu::TextureFormat) -> Vec<u8> {
    let scene = rect_scene(Color::rgb(255, 0, 0));
    let mut view = GpuView::new(context, "Full view").unwrap();
    frame(context, format, |painter, rect| {
        view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap();
    })
}

#[test]
fn a_view_that_has_not_changed_is_shown_again_and_a_change_of_the_pictures_size_draws_it_again() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let scene = corner_scene();
        let mut view = GpuView::new(&context, "Test view").unwrap();
        let paint = |view: &mut GpuView| {
            let mut painted = None;
            frame(&context, format, |painter, rect| painted = Some(view.paint(painter, rect, &scene, |_| Affine2::IDENTITY, &options()).unwrap()));
            painted.unwrap()
        };
        context.set_quality(Quality::Fixed(QualityProfile::Low));
        assert!(matches!(paint(&mut view), Painted::Drawn(_)));
        assert!(matches!(paint(&mut view), Painted::Idle), "the same picture again: nothing is drawn");
        context.set_quality(Quality::Fixed(QualityProfile::Balanced));
        assert!(matches!(paint(&mut view), Painted::Drawn(_)), "Balanced draws at the view's size, Low did not");
        assert!(matches!(paint(&mut view), Painted::Idle));
        // Balanced and High draw at the same size: the picture would be the same, so it is shown again.
        context.set_quality(Quality::Fixed(QualityProfile::High));
        assert!(matches!(paint(&mut view), Painted::Idle), "the picture would be the same");
    });
}

#[test]
fn the_choice_sets_the_memory_caps_of_the_device_and_automatic_goes_back_to_the_adapters() {
    Gpu::for_each_test_mode(|gpu| {
        let context = context(gpu, wgpu::TextureFormat::Rgba8Unorm);
        let adapter = context.adapter_profile();
        assert_eq!(gpu.alloc.budgets(), adapter.budgets(), "a new context starts with the caps of its adapter");
        assert_eq!((context.quality(), context.ceiling()), (Quality::Auto, adapter));
        for profile in QualityProfile::ALL {
            context.set_quality(Quality::Fixed(profile));
            assert_eq!(gpu.alloc.budgets(), profile.budgets(), "{profile}");
            assert_eq!((context.quality(), context.ceiling()), (Quality::Fixed(profile), profile));
        }
        context.set_quality(Quality::Auto);
        assert_eq!(gpu.alloc.budgets(), adapter.budgets());
        assert_eq!(context.quality(), Quality::Auto);
    });
}

#[test]
fn frames_that_are_too_slow_lower_an_automatic_view_but_never_one_with_a_profile_by_hand() {
    Gpu::for_each_test_mode(|gpu| {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let context = context(gpu, format);
        let scene = corner_scene();
        // Every paint draws: the view moves a little each time.
        let paint = |view: &mut GpuView, n: usize| {
            frame(&context, format, |painter, rect| {
                let matrix = Affine2::from_translation(Vec2::new(n as f32 * 0.01, 0.0));
                view.paint(painter, rect, &scene, |_| matrix, &options()).unwrap();
            })
        };
        // Automatic, on a card that deserves its adapter's best: every frame is taken to take 80 ms, and the profile goes down a step
        // after a window of them and another after the cooldown and a window more, and no further.
        let mut view = GpuView::new(&context, "Test view").unwrap();
        paint(&mut view, 0);
        let best = context.adapter_profile();
        assert_eq!(view.profile(), best);
        view.pretend_frame_ms = Some(80.0);
        let mut seen = vec![best];
        for n in 1..=400 {
            paint(&mut view, n);
            if seen.last() != Some(&view.profile()) {
                seen.push(view.profile());
            }
        }
        let expected: Vec<QualityProfile> = QualityProfile::ALL.into_iter().filter(|p| *p >= best).collect();
        assert_eq!(seen, expected, "one step at a time, down to Low");
        assert_eq!(view.profile(), QualityProfile::Low);
        assert_eq!(view.renderer().view_size(), (120, 90), "and so it is drawn smaller");
        // A profile by hand is used as it is at once, and slow frames do not lower it.
        context.set_quality(Quality::Fixed(QualityProfile::High));
        paint(&mut view, 401);
        assert_eq!((view.profile(), view.renderer().view_size()), (QualityProfile::High, SIZE));
        for n in 402..800 {
            paint(&mut view, n);
        }
        assert_eq!(view.profile(), QualityProfile::High, "a profile by hand stays");
        // Back to automatic: it keeps what it has when that is lower than what the adapter deserves, and starts from there.
        context.set_quality(Quality::Auto);
        paint(&mut view, 800);
        assert_eq!(view.profile(), best.max(QualityProfile::High));
        // And fast frames bring it back up to what the adapter deserves.
        view.pretend_frame_ms = Some(80.0);
        for n in 801..1200 {
            paint(&mut view, n);
        }
        assert_eq!(view.profile(), QualityProfile::Low);
        view.pretend_frame_ms = Some(4.0);
        for n in 1200..2000 {
            paint(&mut view, n);
        }
        assert_eq!(view.profile(), best, "back to the best the adapter allows");
    });
}
