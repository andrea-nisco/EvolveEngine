//! `--benchmark`: measures how the CPU renderer and the editor's bookkeeping cope with a big scene.
//!
//! ```text
//! evolve_editor --benchmark <project.evolve> <report> [--allow-long] [--gpu-draw-supported]   writes <report>.json and <report>.txt
//! ```
//!
//! Each view is drawn by the CPU renderer and, when the machine has a graphics card, by the GPU renderer: one cold frame
//! (tessellation, uploads) and a fixed number of steady frames, timed from submit to completion. A scene with features the
//! GPU renderer cannot draw yet is reported as not drawn, unless `--gpu-draw-supported` asks for the supported items to be
//! timed anyway (the picture is then incomplete, and the report says what was left out).
//!
//! A benchmark never runs unbounded (GPU_SAFETY.md, rules A5 and F2): every view renders a fixed number of frames, and
//! the whole run stops with an error after five minutes unless `--allow-long` is given. The report says how long it took.
//!
//! Three camera views are rendered headless at 1920 x 1080: the whole map, a region (about 10% of the
//! objects visible) and a close-up on one border (under 1%). The report has fields for GPU time, video
//! memory and draw calls so a GPU renderer can be measured with the same harness; they are empty (`null`) for
//! the CPU renderer, which has no such numbers. Run it with a release build: `cargo run --release`.

use crate::history::History;
use crate::inspector::{show_tree, TreeView};
use eframe::egui;
use evolve_engine::geometry::{mean_scale, Affine2, Rect, Vec2};
use evolve_engine::io;
use evolve_engine::render;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene};
use evolve_engine::tiny_skia::{Color, Pixmap};
use evolve_gpu::alloc::Category;
use evolve_gpu::renderer::{RenderOptions, Renderer};
use evolve_gpu::targets::TargetPolicy;
use evolve_gpu::Gpu;
use serde::Serialize;
use std::collections::HashSet;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

const VIEWPORT: (u32, u32) = (1920, 1080);
const WARMUP_FRAMES: usize = 1;
const MAX_FRAMES: usize = 10;
const MIN_FRAMES: usize = 3;
const FRAME_BUDGET: Duration = Duration::from_secs(20);
/// How long a whole benchmark may take before it stops, unless the owner asked for a long one.
const TIME_LIMIT: Duration = Duration::from_secs(300);

/// The running time of one benchmark, and the limit it must keep.
struct Clock {
    started: Instant,
    limit: Option<Duration>,
}

impl Clock {
    fn new(allow_long: bool) -> Clock {
        Clock { started: Instant::now(), limit: (!allow_long).then_some(TIME_LIMIT) }
    }

    /// Fails when the limit has passed. Called between the stages of the benchmark and between frames.
    fn check(&self, stage: &str) -> io::Result<()> {
        match self.limit {
            Some(limit) if self.started.elapsed() > limit => Err(format!(
                "the benchmark stopped during \"{stage}\": it ran for more than {} seconds. Use --allow-long to run it for longer.",
                limit.as_secs()
            )
            .into()),
            _ => Ok(()),
        }
    }
}
/// What a benchmark was asked to do beyond the usual.
#[derive(Clone, Copy, Debug, Default)]
pub struct BenchOptions {
    /// Run longer than the usual time limit.
    pub allow_long: bool,
    /// Time the GPU renderer on the items it can draw even if the scene has others (picture incomplete).
    pub gpu_draw_supported: bool,
}

/// Fraction of the objects that should be visible in each view.
const REGIONAL_VISIBLE: f32 = 0.10;
const CLOSE_UP_VISIBLE: f32 = 0.005;
const FRAMES_PER_SECOND: f64 = 60.0;

#[derive(Serialize)]
struct Report {
    renderer: &'static str,
    /// The graphics card the GPU columns would be measured on, if this machine has one.
    gpu_adapter: Option<String>,
    /// The quality profile this card gets (plan D15): the caps its memory is held to.
    gpu_profile: Option<String>,
    file: String,
    file_mb: f64,
    objects: usize,
    anchors: usize,
    load_ms: f64,
    save_to_text_ms: f64,
    project_text_mb: f64,
    ram_mb_before_load: Option<f64>,
    ram_mb_after_load: Option<f64>,
    scene_clone_ms: f64,
    history_check_ms: f64,
    find_node_by_id_us: f64,
    /// Walking the whole tree and computing every shape's world matrix: what any renderer pays per frame at least.
    scene_walk_ms: f64,
    hit_test_ms: f64,
    /// One frame of the Objects panel (layout, painting and tessellation of its rows), headless, 300 x 700 px.
    objects_panel_ms: f64,
    selection_costs: Vec<SelectionCost>,
    views: Vec<ViewReport>,
    /// How long the whole benchmark took.
    total_seconds: f64,
}

/// Time to process a selection of `selected` shapes (see `selection_costs`).
#[derive(Serialize)]
struct SelectionCost {
    selected: usize,
    prune_ms: f64,
    top_level_ms: f64,
    bounds_ms: f64,
    /// Deleting them one by one, as the Delete key does, and grouping them (on a copy of the scene).
    delete_ms: f64,
    group_ms: f64,
}

#[derive(Serialize)]
struct ViewReport {
    name: &'static str,
    viewport: (u32, u32),
    half_height: f32,
    objects_visible: usize,
    objects_visible_percent: f32,
    /// Objects the renderer is asked to draw: the CPU renderer has no culling, so all of them.
    objects_submitted: usize,
    frames: usize,
    cpu_render_ms_median: f64,
    cpu_render_ms_p95: f64,
    cpu_render_ms_max: f64,
    /// Turning the finished bitmap into an egui image (one of the two copies before it reaches the card).
    frame_prep_ms: f64,
    bitmap_mb_per_frame: f64,
    bitmap_mb_per_second_at_60fps: f64,
    gpu_ms: Option<f64>,
    vram_mb: Option<f64>,
    draw_calls: Option<u64>,
    /// The GPU renderer's numbers for this view, if this machine has a card and the scene could be drawn.
    gpu: Option<GpuViewReport>,
}

/// What the GPU renderer did with one view.
#[derive(Serialize)]
struct GpuViewReport {
    /// False when the scene has things the GPU renderer cannot draw yet (see `note`), or something went wrong.
    drawn: bool,
    note: Option<String>,
    /// What was left out in `--gpu-draw-supported` mode.
    skipped: Vec<(String, usize)>,
    /// The first frame: tessellation, uploads and drawing.
    cold_ms: f64,
    frames: usize,
    /// Steady frames, from submit to completion on the card.
    frame_ms_median: f64,
    frame_ms_p95: f64,
    frame_ms_max: f64,
    /// CPU time to bring the caches up to date and to encode and submit, in steady frames.
    update_ms_median: f64,
    record_ms_median: f64,
    items: usize,
    draw_calls: usize,
    triangles: usize,
    cold_vertices_uploaded: usize,
    cold_upload_mb: f64,
    /// The most that any steady frame sent to the card, in bytes.
    steady_upload_bytes: u64,
    /// Graphics memory held, as counted by the allocator.
    vram_mb: f64,
    /// Of that, the content (geometry, shape table, ramps, glyphs: what the scene needs, whatever the view), the layers (what its
    /// groups with opacity need at this view), and the targets (what the view's size needs, by formula): plan 5.8.
    content_mb: f64,
    layers_mb: f64,
    targets_mb: f64,
}

impl GpuViewReport {
    fn not_drawn(note: String) -> GpuViewReport {
        GpuViewReport {
            drawn: false,
            note: Some(note),
            skipped: Vec::new(),
            cold_ms: 0.0,
            frames: 0,
            frame_ms_median: 0.0,
            frame_ms_p95: 0.0,
            frame_ms_max: 0.0,
            update_ms_median: 0.0,
            record_ms_median: 0.0,
            items: 0,
            draw_calls: 0,
            triangles: 0,
            cold_vertices_uploaded: 0,
            cold_upload_mb: 0.0,
            steady_upload_bytes: 0,
            vram_mb: 0.0,
            content_mb: 0.0,
            layers_mb: 0.0,
            targets_mb: 0.0,
        }
    }
}

pub fn run(project_path: &str, report_path: &str, options: BenchOptions) -> io::Result<String> {
    let clock = Clock::new(options.allow_long);
    let gpu = Gpu::headless();
    let path = Path::new(project_path);
    let ram_before = ram_mb();
    let started = Instant::now();
    let project = io::load_project(path)?;
    let load_ms = ms(started.elapsed());
    let ram_after = ram_mb();
    let scene = &project.scenes[project.start];

    let started = Instant::now();
    let text = io::project_to_string(&project)?;
    let save_ms = ms(started.elapsed());

    clock.check("saving")?;
    let leaves = shape_bounds(scene);
    let anchors = count_anchors(&scene.nodes);

    let clone_ms = median_ms(5, || { black_box(scene.clone()); });
    let mut history = History::new(scene);
    let history_ms = median_ms(5, || { black_box(history.update(scene, false)); });
    let last = last_id(&scene.nodes);
    let find_us = median_ms(20, || { black_box(scene.find(last)); }) * 1000.0;

    let map = Rect::from_cloud(leaves.iter().flat_map(|r| r.corners())).ok_or("the scene has no shapes")?;
    clock.check("editing costs")?;
    let views = vec![
        view_report("macro", scene, &leaves, map.center(), None, gpu.as_ref(), options, &clock)?,
        view_report("regional", scene, &leaves, nearest_center(&leaves, map.center()), Some(REGIONAL_VISIBLE), gpu.as_ref(), options, &clock)?,
        view_report("close-up", scene, &leaves, close_up_center(scene, map.center()), Some(CLOSE_UP_VISIBLE), gpu.as_ref(), options, &clock)?,
    ];
    let mut selection: Vec<SelectionCost> = Vec::new();
    for n in [500, 1000, 2000, 4000] {
        clock.check("selection costs")?;
        selection.push(selection_costs(scene, n));
    }
    let hit_ms = median_ms(5, || { black_box(scene.hit_test(map.center(), 2.0)); });
    clock.check("the Objects panel")?;

    let report = Report {
        renderer: if gpu.is_some() { "cpu (tiny-skia) and gpu (wgpu)" } else { "cpu (tiny-skia)" },
        gpu_adapter: gpu.as_ref().map(|gpu| gpu.to_string()),
        gpu_profile: gpu.as_ref().map(|gpu| evolve_gpu::profile::QualityProfile::for_device(gpu).to_string()),
        file: project_path.to_string(),
        file_mb: std::fs::metadata(path).map_or(0.0, |m| m.len() as f64 / 1e6),
        objects: leaves.len(),
        anchors,
        load_ms,
        save_to_text_ms: save_ms,
        project_text_mb: text.len() as f64 / 1e6,
        ram_mb_before_load: ram_before,
        ram_mb_after_load: ram_after,
        scene_clone_ms: clone_ms,
        history_check_ms: history_ms,
        find_node_by_id_us: find_us,
        scene_walk_ms: median_ms(5, || {
            black_box(count_world_matrices(&scene.nodes, Affine2::IDENTITY));
        }),
        hit_test_ms: hit_ms,
        objects_panel_ms: objects_panel_ms(scene),
        selection_costs: selection,
        views,
        total_seconds: clock.started.elapsed().as_secs_f64(),
    };
    std::fs::write(format!("{report_path}.json"), serde_json::to_string_pretty(&report)?)?;
    let table = format_report(&report);
    std::fs::write(format!("{report_path}.txt"), &table)?;
    Ok(table)
}

/// The view of the benchmark around `center` that shows `target` (a share of the objects, zoomed until that is true) or the whole
/// scene (`None`): the half height it shows, how many objects are visible, and the matrix.
fn view_geometry(leaves: &[Rect], center: Vec2, target: Option<f32>) -> (f32, usize, Affine2) {
    let (width, height) = VIEWPORT;
    let aspect = width as f32 / height as f32;
    let whole = Rect::from_cloud(leaves.iter().flat_map(|r| r.corners())).unwrap_or(Rect { min: Vec2::ZERO, max: Vec2::ONE });
    let widest = whole.size().y.max(whole.size().x / aspect) * 0.51;
    let half_height = match target {
        None => widest,
        Some(share) => {
            let (mut low, mut high) = (1.0_f32, widest);
            for _ in 0..40 {
                let middle = (low + high) * 0.5;
                if visible_count(leaves, center, middle, aspect) as f32 / (leaves.len() as f32) < share {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            high
        }
    };
    let visible = visible_count(leaves, center, half_height, aspect);
    let scale = height as f32 / (2.0 * half_height);
    let view = Affine2::from_translation(Vec2::new(width as f32, height as f32) * 0.5) * Affine2::from_scale(Vec2::splat(scale)) * Affine2::from_translation(-center);
    (half_height, visible, view)
}

/// The three views of the benchmark for `scene` (the whole scene, a region with a tenth of the objects, a close-up with a
/// two-hundredth), for tools that look at them more closely than the report does.
#[cfg(test)]
pub(crate) fn benchmark_views(scene: &Scene) -> Vec<(&'static str, Affine2)> {
    let leaves = shape_bounds(scene);
    let Some(map) = Rect::from_cloud(leaves.iter().flat_map(|r| r.corners())) else { return Vec::new() };
    vec![
        ("macro", view_geometry(&leaves, map.center(), None).2),
        ("regional", view_geometry(&leaves, nearest_center(&leaves, map.center()), Some(REGIONAL_VISIBLE)).2),
        ("close-up", view_geometry(&leaves, close_up_center(scene, map.center()), Some(CLOSE_UP_VISIBLE)).2),
    ]
}

/// Renders one view repeatedly and measures it. `target` is the share of objects that should be visible
/// (the view is zoomed until that is true), or `None` for the whole map.
#[allow(clippy::too_many_arguments)]
fn view_report(
    name: &'static str,
    scene: &Scene,
    leaves: &[Rect],
    center: Vec2,
    target: Option<f32>,
    gpu: Option<&Gpu>,
    options: BenchOptions,
    clock: &Clock,
) -> io::Result<ViewReport> {
    let (width, height) = VIEWPORT;
    let (half_height, visible, view) = view_geometry(leaves, center, target);

    let mut pixmap = Pixmap::new(width, height).expect("the viewport has a valid size");
    let mut times = Vec::new();
    let started = Instant::now();
    for frame in 0..WARMUP_FRAMES + MAX_FRAMES {
        clock.check(name)?;
        pixmap.fill(Color::WHITE);
        let frame_start = Instant::now();
        render::render(scene, view, &mut pixmap);
        if frame >= WARMUP_FRAMES {
            times.push(ms(frame_start.elapsed()));
        }
        if times.len() >= MIN_FRAMES && started.elapsed() > FRAME_BUDGET {
            break;
        }
    }
    times.sort_by(f64::total_cmp);
    let percentile = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    let prep_ms = median_ms(5, || { black_box(egui::ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data())); });
    let bitmap_mb = (width * height * 4) as f64 / 1e6;
    let gpu_report = match gpu {
        Some(gpu) => Some(gpu_view(gpu, scene, view, options, clock, name)?),
        None => None,
    };
    Ok(ViewReport {
        name,
        viewport: VIEWPORT,
        half_height,
        objects_visible: visible,
        objects_visible_percent: visible as f32 * 100.0 / leaves.len() as f32,
        objects_submitted: leaves.len(),
        frames: times.len(),
        cpu_render_ms_median: percentile(0.5),
        cpu_render_ms_p95: percentile(0.95),
        cpu_render_ms_max: *times.last().unwrap_or(&0.0),
        frame_prep_ms: prep_ms,
        bitmap_mb_per_frame: bitmap_mb,
        bitmap_mb_per_second_at_60fps: bitmap_mb * FRAMES_PER_SECOND,
        gpu_ms: gpu_report.as_ref().filter(|g| g.drawn).map(|g| g.frame_ms_median),
        vram_mb: gpu_report.as_ref().filter(|g| g.drawn).map(|g| g.vram_mb),
        draw_calls: gpu_report.as_ref().filter(|g| g.drawn).map(|g| g.draw_calls as u64),
        gpu: gpu_report,
    })
}

/// Draws one view with the GPU renderer: a cold frame, then a fixed number of steady frames, each timed until the card is
/// done. A fresh renderer per view, so that the cold frame is a cold frame. Bounded by the frame count and the time budget.
fn gpu_view(gpu: &Gpu, scene: &Scene, view: Affine2, options: BenchOptions, clock: &Clock, name: &str) -> io::Result<GpuViewReport> {
    let (width, height) = VIEWPORT;
    let render_options = RenderOptions { camera_frame: false, skip_unsupported: options.gpu_draw_supported, cull: true, targets: TargetPolicy::exact() };
    let mut renderer = match Renderer::new(gpu) {
        Ok(renderer) => renderer,
        Err(error) => return Ok(GpuViewReport::not_drawn(format!("no renderer: {error}"))),
    };
    let wait = || gpu.wait_idle();
    clock.check(name)?;
    let cold_start = Instant::now();
    let cold = match renderer.render(scene, view, (width, height), &render_options) {
        Ok(stats) => stats,
        Err(error) => return Ok(GpuViewReport::not_drawn(error.to_string())),
    };
    wait();
    let cold_ms = ms(cold_start.elapsed());

    let (mut frame_times, mut updates, mut records) = (Vec::new(), Vec::new(), Vec::new());
    let mut steady_upload = 0u64;
    let started = Instant::now();
    for _ in 0..MAX_FRAMES {
        clock.check(name)?;
        let start = Instant::now();
        let stats = match renderer.render(scene, view, (width, height), &render_options) {
            Ok(stats) => stats,
            Err(error) => return Ok(GpuViewReport::not_drawn(error.to_string())),
        };
        wait();
        frame_times.push(ms(start.elapsed()));
        updates.push(stats.update_ms);
        records.push(stats.record_ms);
        steady_upload = steady_upload.max(stats.table_bytes_uploaded + stats.geometry_bytes_uploaded);
        if frame_times.len() >= MIN_FRAMES && started.elapsed() > FRAME_BUDGET {
            break;
        }
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let mut sorted = frame_times.clone();
    sorted.sort_by(f64::total_cmp);
    Ok(GpuViewReport {
        drawn: true,
        note: (!cold.skipped.is_empty()).then(|| "the picture is incomplete: only the supported items were drawn (timing only)".to_string()),
        skipped: cold.skipped.iter().map(|(what, count)| (format!("{what:?}"), *count)).collect(),
        cold_ms,
        frames: frame_times.len(),
        frame_ms_median: sorted[(sorted.len() - 1) / 2],
        frame_ms_p95: sorted[((sorted.len() - 1) as f64 * 0.95).round() as usize],
        frame_ms_max: *sorted.last().unwrap_or(&0.0),
        update_ms_median: median(&mut updates),
        record_ms_median: median(&mut records),
        items: cold.items,
        draw_calls: cold.draw_calls,
        triangles: cold.triangles,
        cold_vertices_uploaded: cold.vertices_uploaded,
        cold_upload_mb: (cold.table_bytes_uploaded + cold.geometry_bytes_uploaded) as f64 / 1e6,
        steady_upload_bytes: steady_upload,
        vram_mb: gpu.alloc.total_used() as f64 / 1e6,
        content_mb: [Category::Geometry, Category::ShapeTable, Category::Ramps, Category::Glyphs].iter().map(|&c| gpu.alloc.used(c)).sum::<u64>() as f64 / 1e6,
        layers_mb: gpu.alloc.used(Category::Layers) as f64 / 1e6,
        targets_mb: gpu.alloc.used(Category::Targets) as f64 / 1e6,
    })
}

fn format_report(r: &Report) -> String {
    let mut text = format!("Benchmark: {} ({})\n", r.file, r.renderer);
    text.push_str(&format!("  graphics card for the GPU columns: {}\n", r.gpu_adapter.as_deref().unwrap_or("none found")));
    if let Some(profile) = &r.gpu_profile {
        text.push_str(&format!("  quality profile of that card: {profile}\n"));
    }
    let mut line = |label: &str, value: String| text.push_str(&format!("  {label:<34}{value}\n"));
    line("objects (shapes)", r.objects.to_string());
    line("anchor points", r.anchors.to_string());
    line("file size on disk", format!("{:.1} MB", r.file_mb));
    line("load (read + parse)", format!("{:.0} ms", r.load_ms));
    line("write to text (save, autosave)", format!("{:.0} ms ({:.1} MB)", r.save_to_text_ms, r.project_text_mb));
    if let (Some(before), Some(after)) = (r.ram_mb_before_load, r.ram_mb_after_load) {
        line("RAM, before / after load", format!("{before:.0} MB / {after:.0} MB"));
    }
    line("clone the scene (clip preview, once)", format!("{:.1} ms", r.scene_clone_ms));
    line("undo check, nothing changed", format!("{:.1} ms per frame", r.history_check_ms));
    line("find the last object by id", format!("{:.0} us", r.find_node_by_id_us));
    line("walk the scene, world matrices", format!("{:.2} ms", r.scene_walk_ms));
    line("find the object under a click", format!("{:.1} ms", r.hit_test_ms));
    line("Objects panel, one frame", format!("{:.1} ms", r.objects_panel_ms));
    for c in &r.selection_costs {
        line(&format!("{} selected: prune / top-level / bounds", c.selected), format!("{:.1} / {:.1} / {:.1} ms", c.prune_ms, c.top_level_ms, c.bounds_ms));
        line(&format!("{} selected: delete / group", c.selected), format!("{:.0} / {:.0} ms", c.delete_ms, c.group_ms));
    }
    line("total running time", format!("{:.0} s", r.total_seconds));
    for v in &r.views {
        text.push_str(&format!("\n  view \"{}\" at {} x {} (half height {:.0}): {} of {} objects visible ({:.1}%)\n", v.name, v.viewport.0, v.viewport.1, v.half_height, v.objects_visible, v.objects_submitted, v.objects_visible_percent));
        let mut line = |label: &str, value: String| text.push_str(&format!("    {label:<38}{value}\n"));
        line("CPU render, median / p95 / max", format!("{:.0} / {:.0} / {:.0} ms ({} frames)", v.cpu_render_ms_median, v.cpu_render_ms_p95, v.cpu_render_ms_max, v.frames));
        line("bitmap to egui image", format!("{:.1} ms", v.frame_prep_ms));
        line("bitmap data per frame / at 60 fps", format!("{:.1} MB / {:.0} MB/s", v.bitmap_mb_per_frame, v.bitmap_mb_per_second_at_60fps));
        line("objects drawn (no culling)", v.objects_submitted.to_string());
        match &v.gpu {
            None => line("GPU time / VRAM / draw calls", "no graphics card found".into()),
            Some(g) if !g.drawn => line("GPU renderer", format!("not drawn: {}", g.note.as_deref().unwrap_or("unknown"))),
            Some(g) => {
                line("GPU cold frame (tessellate, upload, draw)", format!("{:.0} ms ({:.1} MB sent, {} vertices)", g.cold_ms, g.cold_upload_mb, g.cold_vertices_uploaded));
                line("GPU steady frame, median / p95 / max", format!("{:.2} / {:.2} / {:.2} ms ({} frames)", g.frame_ms_median, g.frame_ms_p95, g.frame_ms_max, g.frames));
                line("GPU frame, CPU update / record+submit", format!("{:.2} / {:.2} ms", g.update_ms_median, g.record_ms_median));
                line("GPU items / draw calls / triangles", format!("{} / {} / {}", g.items, g.draw_calls, g.triangles));
                line("GPU sent per steady frame / memory held", format!("{} bytes / {:.0} MB", g.steady_upload_bytes, g.vram_mb));
                line("GPU memory: content / layers / targets", format!("{:.0} / {:.0} / {:.0} MB", g.content_mb, g.layers_mb, g.targets_mb));
                if let Some(note) = &g.note {
                    line("GPU note", format!("{note}; left out: {:?}", g.skipped));
                }
            }
        }
    }
    text
}

/// One frame of the Objects panel, run without a window, including turning its shapes into triangles.
fn objects_panel_ms(scene: &Scene) -> f64 {
    let ctx = egui::Context::default();
    let collapsed = HashSet::new();
    let view = TreeView { collapsed: &collapsed, selection: &[], renaming: None };
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 700.0))), ..Default::default() };
    let mut frame = || {
        let mut output = ctx.run_ui(input.clone(), |ui| {
            black_box(show_tree(ui, scene, &view));
        });
        output.textures_delta.clear(); // there is no graphics card to hand them to
        black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
    };
    frame(); // the first frame sets up fonts
    median_ms(3, &mut frame)
}

/// What the editor does with a selection of `count` shapes, each measured once: dropping ids that no longer
/// exist (done every frame), reducing to top-level objects, and the combined bounds.
fn selection_costs(scene: &Scene, count: usize) -> SelectionCost {
    let ids: Vec<NodeId> = scene.all_ids().into_iter().filter(|&id| scene.find(id).is_some_and(|n| matches!(n.kind, NodeKind::Shape(_)))).take(count).collect();
    let once = |work: &mut dyn FnMut()| {
        let start = Instant::now();
        work();
        ms(start.elapsed())
    };
    let prune_ms = once(&mut || {
        let mut kept = ids.clone();
        scene.retain_existing(&mut kept);
        black_box(kept);
    });
    let top_level_ms = once(&mut || {
        black_box(scene.top_level_selection(&ids));
    });
    let bounds_ms = once(&mut || {
        black_box(scene.selection_bounds(&ids));
    });
    let delete_ms = {
        let mut copy = scene.clone();
        once(&mut || {
            let top = copy.top_level_selection(&ids);
            copy.remove_all(&top);
        })
    };
    let group_ms = {
        let mut copy = scene.clone();
        once(&mut || {
            black_box(copy.group(&ids));
        })
    };
    SelectionCost { selected: ids.len(), prune_ms, top_level_ms, bounds_ms, delete_ms, group_ms }
}

// --- Measuring helpers ---------------------------------------------------------------------------

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median_ms(runs: usize, mut work: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            work();
            ms(start.elapsed())
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn ram_mb() -> Option<f64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as f64 / 1e6)
}

// --- Looking at the scene ------------------------------------------------------------------------

/// World-space bounds of every shape, with half the stroke width added on each side.
fn shape_bounds(scene: &Scene) -> Vec<Rect> {
    fn walk(nodes: &[Node], parent: Affine2, out: &mut Vec<Rect>) {
        for node in nodes {
            let world = parent * node.transform;
            match &node.kind {
                NodeKind::Group(children) => walk(children, world, out),
                NodeKind::Shape(shape) => {
                    let points = shape.path.subpaths.iter().flat_map(|s| s.anchors.iter().map(|a| world.transform_point2(a.pos)));
                    let margin = shape.stroke.as_ref().map_or(0.0, |s| s.width * 0.5 * mean_scale(&world));
                    out.extend(Rect::from_cloud(points).map(|r| r.expanded(margin)));
                }
                NodeKind::Instance(_) | NodeKind::Camera(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&scene.nodes, Affine2::IDENTITY, &mut out);
    out
}

/// Visits every node and combines transforms down the tree, returning how many shapes it reached.
fn count_world_matrices(nodes: &[Node], parent: Affine2) -> usize {
    nodes
        .iter()
        .map(|node| {
            let world = parent * node.transform;
            match &node.kind {
                NodeKind::Group(children) => count_world_matrices(children, world),
                NodeKind::Shape(_) => usize::from(world.translation.is_finite()),
                NodeKind::Instance(_) | NodeKind::Camera(_) => 0,
            }
        })
        .sum()
}

fn count_anchors(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|node| match &node.kind {
            NodeKind::Group(children) => count_anchors(children),
            NodeKind::Shape(shape) => shape.path.subpaths.iter().map(|s| s.anchors.len()).sum(),
            NodeKind::Instance(_) | NodeKind::Camera(_) => 0,
        })
        .sum()
}

fn last_id(nodes: &[Node]) -> NodeId {
    nodes
        .iter()
        .map(|node| match &node.kind {
            NodeKind::Group(children) => last_id(children).max(node.id),
            _ => node.id,
        })
        .max()
        .unwrap_or(NodeId(0))
}

fn visible_count(leaves: &[Rect], center: Vec2, half_height: f32, aspect: f32) -> usize {
    let half = Vec2::new(half_height * aspect, half_height);
    let view = Rect { min: center - half, max: center + half };
    leaves.iter().filter(|r| r.intersects(&view)).count()
}

/// The center of the shape whose center is closest to `point`.
fn nearest_center(leaves: &[Rect], point: Vec2) -> Vec2 {
    leaves.iter().map(|r| r.center()).min_by(|a, b| a.distance(point).total_cmp(&b.distance(point))).unwrap_or(point)
}

/// A point on a border line near the middle of the map: the middle anchor of the line in the last
/// top-level group (the nation borders in the imported map) that is closest to `point`.
fn close_up_center(scene: &Scene, point: Vec2) -> Vec2 {
    let Some(NodeKind::Group(children)) = scene.nodes.last().map(|n| &n.kind) else { return point };
    let middles = children.iter().filter_map(|node| match &node.kind {
        NodeKind::Shape(shape) => shape.path.subpaths.first().and_then(|s| s.anchors.get(s.anchors.len() / 2)).map(|a| node.transform.transform_point2(a.pos)),
        _ => None,
    });
    middles.min_by(|a, b| a.distance(point).total_cmp(&b.distance(point))).unwrap_or(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_benchmark_that_ran_past_its_limit_stops_with_a_message_unless_it_was_allowed_to_run_long() {
        let expired = Clock { started: Instant::now() - Duration::from_secs(301), limit: Some(TIME_LIMIT) };
        let message = expired.check("the close-up view").unwrap_err().to_string();
        assert!(message.contains("close-up") && message.contains("300 seconds") && message.contains("--allow-long"), "{message}");
        let recent = Clock { started: Instant::now(), limit: Some(TIME_LIMIT) };
        assert!(recent.check("anything").is_ok());
        let long = Clock { limit: None, ..expired };
        assert!(long.check("anything").is_ok(), "--allow-long switches the limit off");
        assert_eq!(Clock::new(false).limit, Some(TIME_LIMIT));
        assert_eq!(Clock::new(true).limit, None);
    }
}
