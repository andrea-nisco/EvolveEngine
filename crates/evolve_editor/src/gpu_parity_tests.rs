//! The GPU renderer against the CPU renderer on whole projects (plan 3.9): the editor's sample project, and the two benchmark
//! scenes, each at three zooms, in both device modes (native, and WebGL2 limits). Both renderers draw the same scene into the
//! same-sized bitmap and the pictures are compared with the edge-aware rule.
//!
//! What the GPU renderer cannot draw yet (groups with opacity, clip groups, symbol instances, skinned shapes: Phase 4 and 6)
//! is taken out of both scenes first, so that what is compared is everything it *can* draw. When those features arrive, the
//! stripping shrinks and finally goes away.

use crate::bench_scene;
use evolve_engine::geometry::Affine2;
use evolve_engine::render;
use evolve_engine::scene::Scene;
use evolve_engine::tiny_skia::Pixmap;
use evolve_engine::Vec2;
use evolve_gpu::compare::{compare_pictures, Rule};
use evolve_gpu::renderer::{RenderOptions, Renderer};
use evolve_gpu::targets::TargetPolicy;
use evolve_gpu::Gpu;

const SIZE: (u32, u32) = (480, 270);

/// Takes out every node the GPU renderer does not draw (and what is inside it), until nothing is left to take out.
fn strip_unsupported(scene: &mut Scene) -> usize {
    let mut removed = 0;
    loop {
        let nodes = Renderer::unsupported_nodes(scene);
        if nodes.is_empty() {
            return removed;
        }
        for (id, _) in nodes {
            removed += scene.remove(id).is_some() as usize;
        }
    }
}

/// Draws `scene` both ways, with the camera's frame, and compares them. Returns the comparison text on failure.
fn compare(gpu: &Gpu, scene: &Scene, view: Affine2, rule: Rule) -> Result<String, String> {
    let mut cpu = Pixmap::new(SIZE.0, SIZE.1).expect("a bitmap");
    render::render(scene, view, &mut cpu);
    let mut renderer = Renderer::new(gpu).expect("a renderer");
    let options = RenderOptions { camera_frame: true, skip_unsupported: false, cull: true, targets: TargetPolicy::exact() };
    renderer.render(scene, view, SIZE, &options).map_err(|e| e.to_string())?;
    let picture = gpu.read_texture_part(renderer.picture().expect("a picture"), 0, 0, SIZE.0, SIZE.1);
    let result = compare_pictures(cpu.data(), &picture, SIZE.0, rule);
    if result.passes() {
        Ok(result.to_string())
    } else {
        if let Ok(folder) = std::env::var("EVOLVE_DUMP_PICTURES") {
            let name = format!("parity-{}", gpu.mode.name().replace(' ', "-"));
            let _ = cpu.save_png(std::path::Path::new(&folder).join(format!("{name}-cpu.png")));
            if let Some(gpu_pixmap) = Pixmap::from_vec(picture, evolve_engine::tiny_skia::IntSize::from_wh(SIZE.0, SIZE.1).expect("a size")) {
                let _ = gpu_pixmap.save_png(std::path::Path::new(&folder).join(format!("{name}-gpu.png")));
            }
        }
        Err(result.to_string())
    }
}

/// The whole scene, a region and a close-up: the three views of the benchmark, found from the scene's bounds.
fn three_views(scene: &Scene) -> [(&'static str, Affine2); 3] {
    let bounds = scene.selection_bounds(&scene.all_ids()).expect("something to draw");
    let (center, extent) = (bounds.center(), bounds.size());
    let fit = (SIZE.0 as f32 / extent.x).min(SIZE.1 as f32 / extent.y);
    let view = |factor: f32| {
        Affine2::from_translation(Vec2::new(SIZE.0 as f32 * 0.5, SIZE.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(fit * factor)) * Affine2::from_translation(-center)
    };
    [("whole", view(1.0)), ("region", view(3.16)), ("close-up", view(14.0))]
}

#[test]
fn the_supported_part_of_the_sample_project_matches_the_cpu_renderer() {
    Gpu::for_each_test_mode(|gpu| {
        let project = crate::sample::project();
        for (index, original) in project.scenes.iter().enumerate() {
            let mut scene = original.clone();
            let removed = strip_unsupported(&mut scene);
            assert!(Renderer::unsupported(&scene).is_empty());
            let view = scene.camera_view().view_matrix(Vec2::new(SIZE.0 as f32, SIZE.1 as f32));
            match compare(gpu, &scene, view, Rule { interior_limit: 4, ..Rule::default() }) {
                Ok(text) => println!("sample scene {index} ({removed} nodes left out): {text}"),
                Err(text) => panic!("sample scene {index} ({}, {removed} nodes left out): {text}", gpu.mode.name()),
            }
        }
    });
}

#[test]
fn the_benchmark_scenes_match_the_cpu_renderer_at_three_zooms() {
    Gpu::for_each_test_mode(|gpu| {
        for kind in bench_scene::KINDS {
            let mut scene = bench_scene::generate(kind).expect("a scene").scenes.remove(0);
            let removed = strip_unsupported(&mut scene);
            // Both scenes are dense: the stress scene is 20,000 lines of 12 points each in the size of a postage stamp
            // (hairlines everywhere, where 4x MSAA and tiny-skia's analytic coverage differ by a few levels in nearly every
            // pixel), and the illustration scene has thousands of see-through strokes piled up, where the card rounds to 8
            // bits after every blend and tiny-skia does not (the picture comes out about two levels darker on average).
            // A shift of the picture, a wrong blend or a missing hairline rule is far above that.
            let rule = Rule { interior_limit: 4, interior_exceeding: 0.005, mean_difference: 3.5, ..Rule::default() };
            for (name, view) in three_views(&scene) {
                match compare(gpu, &scene, view, rule) {
                    Ok(text) => println!("{kind}, {name} ({removed} nodes left out): {text}"),
                    Err(text) => panic!("{kind}, {name} ({}, {removed} nodes left out): {text}", gpu.mode.name()),
                }
            }
        }
    });
}
