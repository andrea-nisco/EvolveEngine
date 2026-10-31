//! Phase 2 demo: a game that is running (a scene with a script that moves a ball, stepped for a second), with the UI demo drawn on
//! top of it by the CPU renderer, on four screens: 1920x1080, 1280x720, 2560x1080 and a phone held upright.
//!
//! `cargo run -p evolve_ui_scene --release --example ui_demo -- <folder>` writes `ui_demo_<width>x<height>.png` there.
//! (The GPU renderer is compared with the CPU one by `tests/gpu_parity.rs`; set `EVOLVE_DUMP_PICTURES` to see its pictures.)

use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::path::Path;
use evolve_engine::project::Project;
use evolve_engine::render;
use evolve_engine::runtime::{Input, Runtime};
use evolve_engine::scene::{NodeKind, Scene, Shape};
use evolve_engine::style::{Color, Paint, Stroke};
use evolve_engine::tiny_skia::{Color as SkColor, Pixmap};
use evolve_ui_scene::demo::UiDemo;
use evolve_ui_scene::draw_over;
use std::path::PathBuf;

/// A small game for a screen of `size` game units: a ground, a few boxes, and a ball that a script moves to the right.
fn game(size: Vec2) -> Scene {
    let mut scene = Scene::with_camera(size);
    let add = |scene: &mut Scene, name: &str, path: Path, fill: Color, at: Vec2, script: Option<&str>| {
        let mut shape = Shape::new(path);
        shape.fill = Some(Paint::Solid(fill));
        shape.stroke = Some(Stroke::solid(Color::rgb(20, 28, 48), size.x / 640.0));
        let mut node = scene.new_node(name, NodeKind::Shape(shape));
        node.transform = Affine2::from_translation(at);
        node.script = script.map(str::to_string);
        scene.add(node, None);
    };
    let unit = size.x.min(size.y) / 10.0;
    add(&mut scene, "sky", Path::rect(Vec2::ZERO, size), Color::rgb(120, 170, 235), Vec2::ZERO, None);
    add(&mut scene, "ground", Path::rect(Vec2::new(0.0, size.y * 0.78), size), Color::rgb(86, 156, 98), Vec2::ZERO, None);
    for i in 0..5 {
        let x = size.x * (0.1 + 0.2 * i as f32);
        add(&mut scene, "box", Path::rect(Vec2::ZERO, Vec2::splat(unit * (1.0 + 0.3 * i as f32))), Color::rgb(230, 170 - 20 * i as u8, 60), Vec2::new(x, size.y * 0.78 - unit * 1.6), None);
    }
    add(&mut scene, "ball", Path::ellipse(Vec2::ZERO, Vec2::splat(unit * 0.6)), Color::rgb(236, 72, 88), Vec2::new(size.x * 0.42, size.y * 0.35), Some("fn on_update(me, dt) { move_by(me, 140.0 * dt, 40.0 * dt); }"));
    scene
}

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).expect("usage: ui_demo <folder>"));
    std::fs::create_dir_all(&folder).expect("the folder");
    let mut demo = UiDemo::new();

    for (width, height) in [(1920u32, 1080u32), (1280, 720), (2560, 1080), (720, 1280)] {
        // The game's camera frame has the shape of the screen, as a game's would.
        let mut runtime = Runtime::start(&Project::single(game(Vec2::new(width as f32 * 0.66, height as f32 * 0.66))), 0);
        for _ in 0..60 {
            runtime.update(1.0 / 60.0, Input::default());
        }
        let mut pixmap = Pixmap::new(width, height).expect("a screen");
        pixmap.fill(SkColor::BLACK);
        let scene = runtime.scene();
        let camera = scene.camera_view();
        render::render(&scene, camera.view_matrix(Vec2::new(width as f32, height as f32)), &mut pixmap);
        drop(scene);
        draw_over(&mut demo, &mut pixmap);
        let file = folder.join(format!("ui_demo_{width}x{height}.png"));
        pixmap.save_png(&file).expect("write the picture");
        println!("{}", file.display());
    }

    // What the UI costs when nothing changes: the demo still lays out its text every frame (a UI runtime will not), the layer finds
    // that the list is the same and leaves the scene alone.
    let started = std::time::Instant::now();
    for _ in 0..200 {
        std::hint::black_box(evolve_ui_scene::UiSource::frame(&mut demo, &evolve_ui_core::ScreenMetrics::new((1920.0, 1080.0))));
    }
    println!("an unchanged frame of the UI demo: {:.2} ms (text layout and list comparison, no scene work)", started.elapsed().as_secs_f64() * 1000.0 / 200.0);
}
