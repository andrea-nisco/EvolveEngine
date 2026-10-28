//! The scene renderer: tessellation cache, shape table, geometry store, draw list and one pass (plan Phase 2).
//!
//! [`Renderer::prepare`] brings everything up to date for one view of one scene (what changed since the last call is
//! found with the Phase 1 stamps and the scale bucket; nothing else is redone) and [`Renderer::record`] encodes the pass
//! into a command encoder. [`Renderer::render`] does both and submits, for headless use. The picture ends up in the
//! resolved texture of the targets ([`Renderer::picture`]).
//!
//! The renderer never produces a wrong picture on purpose: when the scene holds something it cannot draw yet, `prepare`
//! says so and does nothing, unless it is told to draw what it can (`skip_unsupported`, for timing only).

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use crate::device::Gpu;
use crate::cull::Cull;
use crate::draw_list::{collect, collect_node, unsupported, DrawKind, DrawList, DrawReport, FrameMesh, ItemEdit, Key, LayerMap, Part, PixelRect, ScopeTable, Unsupported};
use crate::fast_map::FastMap;
use crate::table::{premultiplied, RowData};
use crate::errors::ErrorLog;
use crate::layers::{size_class, LayerPool};
use crate::pipeline::{Pipeline, UNIFORM_SLOT_BYTES, VIEW_UNIFORM_BYTES};
use crate::ramps::RampAtlas;
use crate::store::GeometryStore;
use crate::table::ShapeTable;
use crate::targets::{TargetPolicy, Targets};
use crate::tess_cache::{largest_scale, TessCache};
use crate::tessellate::{tessellate_shape, TessLimits, TessWarning};
use evolve_engine::geometry::Affine2;
use evolve_engine::revision::{Changes, Revs, Stamp};
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::Paint;
use evolve_engine::Vec2;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

/// A frame that takes longer than this, on the CPU or until the card is done, is reported in development builds
/// (GPU_SAFETY.md B4): it catches runaway work early.
pub const WATCHDOG_LIMIT: Duration = Duration::from_millis(100);

/// What the watchdog says about a frame, if anything: a sentence naming the view and the scene.
pub fn watchdog_message(view: &str, scene: &str, cpu_ms: f64, card_ms: Option<f64>) -> Option<String> {
    let limit = WATCHDOG_LIMIT.as_secs_f64() * 1000.0;
    let mut slow = Vec::new();
    if cpu_ms > limit {
        slow.push(format!("{cpu_ms:.0} ms of CPU time to prepare it"));
    }
    if let Some(card_ms) = card_ms.filter(|ms| *ms > limit) {
        slow.push(format!("{card_ms:.0} ms until the card was done with it"));
    }
    (!slow.is_empty()).then(|| format!("evolve_gpu watchdog: the {view} of scene \"{scene}\" took {} (limit {limit:.0} ms)", slow.join(" and ")))
}

const NO_COMPLETION: u64 = u64::MAX;

/// Layers in one frame. Each is a render pass of its own, so this bounds the work of one frame (GPU_SAFETY.md B2); a scene with
/// more is drawn by the CPU renderer.
pub const MAX_LAYERS: usize = 2048;
const MAX_PASSES: usize = MAX_LAYERS + 1;
/// A layer that does not fit is drawn at 1/2, then 1/4 of its size, and no smaller.
const MAX_LAYER_DIVISOR: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    /// Draw the game camera's frame (a rectangle in its background color) first, as the CPU renderer's `render` does. Off
    /// gives what `render_content` draws.
    pub camera_frame: bool,
    /// Draw the items the GPU path can draw and leave out the rest, instead of refusing. The picture is then incomplete:
    /// for timing only.
    pub skip_unsupported: bool,
    /// Make and draw only the shapes that can reach the view (see `cull.rs`). The picture is the same; the work and the
    /// memory are much smaller when zoomed in on a large scene.
    pub cull: bool,
    pub targets: TargetPolicy,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { camera_frame: true, skip_unsupported: false, cull: true, targets: TargetPolicy::default() }
    }
}

#[derive(Debug)]
pub enum RenderError {
    /// The scene has things the GPU path cannot draw yet. Nothing was changed.
    Unsupported(BTreeMap<Unsupported, usize>),
    /// A resource could not be had (over the budget, or the card said no).
    Alloc(AllocError),
    /// More draw items than the depth buffer can tell apart.
    TooManyItems(usize),
    /// More layers than one frame may have (each is a render pass).
    TooManyLayers(usize),
    /// The graphics device is gone; use the CPU renderer.
    DeviceLost(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Unsupported(what) => write!(f, "the GPU renderer cannot draw this scene yet: {what:?}"),
            RenderError::Alloc(error) => write!(f, "{error}"),
            RenderError::TooManyItems(n) => write!(f, "{n} draw items are more than the depth buffer can order"),
            RenderError::TooManyLayers(n) => write!(f, "{n} layers are more than a frame may have ({MAX_LAYERS})"),
            RenderError::DeviceLost(why) => write!(f, "the graphics device was lost: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<AllocError> for RenderError {
    fn from(error: AllocError) -> Self {
        RenderError::Alloc(error)
    }
}

/// What a frame did and cost, for the benchmark and the tests.
#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    /// The frame was made from the last one: nothing in the scene changed and the camera only moved inside what was made for it
    /// (see `LastFrame`), so nothing was walked, made or uploaded.
    pub reused_frame: bool,
    /// The frame was made from the last one by bringing only the items of the nodes the scene said had changed up to date (see
    /// `LastFrame` and `Scene::nodes_changed_since`): this many items. Nothing else was walked.
    pub patched_items: usize,
    pub items: usize,
    pub draw_calls: usize,
    /// Render passes of the frame: one for every layer and the main one.
    pub passes: usize,
    /// Layers drawn in this frame, and the pixels of their textures (the size classes, a little more than the layers' own).
    pub layers: usize,
    /// Of them, layers drawn smaller than they are shown because the memory for layers was full.
    pub layers_reduced: usize,
    /// Layers and clips that were not made because they were nested too deep (`MAX_LAYER_DEPTH`, `MAX_CLIP_LEVEL`): that part of
    /// the picture is not exact.
    pub layers_flattened: usize,
    pub clips_flattened: usize,
    pub layer_pixels: u64,
    pub triangles: usize,
    pub shapes_tessellated: usize,
    pub shapes_reused: usize,
    /// Different gradient color ramps in use.
    pub ramps_in_use: usize,
    /// Shapes left out because they cannot reach the view.
    pub shapes_culled: usize,
    /// Shapes whose making waited because the time for it ran out (`set_tessellation_budget`): the picture is a little coarse
    /// or fine for some of them, or lacks shapes that are not in the view yet. Draw again to finish.
    pub deferred: usize,
    pub chunks_rebuilt: usize,
    pub vertices_compacted: bool,
    pub indices_compacted: bool,
    pub vertices_uploaded: usize,
    pub table_bytes_uploaded: u64,
    pub geometry_bytes_uploaded: u64,
    /// Left out because the GPU path cannot draw it (only when `skip_unsupported` is on).
    pub skipped: BTreeMap<Unsupported, usize>,
    pub warnings: Vec<(NodeId, TessWarning)>,
    pub targets_replaced: bool,
    /// CPU time to walk the scene and bring the caches, table and geometry up to date, in milliseconds.
    pub update_ms: f64,
    /// The parts of it: making the triangles of the shapes that need them, finding the draw items, and bringing rows, vertices
    /// and chunks up to date.
    pub tessellate_ms: f64,
    pub collect_ms: f64,
    pub list_ms: f64,
    /// CPU time to encode the pass and submit it (only from `render`), in milliseconds.
    pub record_ms: f64,
}

/// What the last full `prepare` made, kept to tell whether the next one can use it as it is. That is so when the scene has the same
/// stamps, the options are the same, the view only moved or (for a frame of plain shapes) was made larger or smaller by a factor no
/// shape's triangles mind, and everything that can touch the new view was made for the old one (it lies inside the area the old
/// frame kept shapes for). Then the items, rows, vertices and chunks are right as they are and only the view uniform changes, and
/// for a frame with layers their rectangles and composites (`place_layers`). Not kept while anything is waiting, nor when a layer
/// was drawn smaller than its place.
struct LastFrame {
    revs: Revs,
    /// The name of the scene's journal of changes when the frame was made (see `Scene::journal_epoch`).
    epoch: Stamp,
    /// Every item is the fill or stroke of a shape at full opacity in one pass: nodes that changed can be brought up to date alone.
    patchable: bool,
    /// Where the shapes are in the tree, made when first needed (and again when the tree changed shape).
    index: Option<NodeIndex>,
    camera_frame: bool,
    skip_unsupported: bool,
    view: Affine2,
    /// The pixel area shapes were kept for (the view and its margin), or `None` when no cull was used: everything was made.
    region: Option<(Vec2, Vec2)>,
    /// How far the zoom can change before some shape wants other triangles (`Collected::window`), for a frame whose shapes are all
    /// plain ones; `None` for any other frame, which is only drawn again for a view of the same scale.
    window: Option<(f32, f32)>,
    /// Where the frame's layers are, to move them with the view (empty for a frame without layers).
    layers: LayerMap,
    /// The size of the view the frame was made for (the rectangles of layers are clipped to it).
    size: (u32, u32),
    stats: FrameStats,
}

/// A change of zoom is drawn from the last frame only within this factor either way (the margins of the cull and of anti-aliasing
/// are measured in pixels, and a pixel of the new view is this much of one of the old).
const ZOOM_REUSE_RANGE: (f32, f32) = (0.5, 2.0);
/// How far inside the area kept for the old view the new one must lie when the zoom changed: more than the cull's slack of
/// three pixels grown by the largest change above, so that a shape that touches the new view only through its soft edge is there.
const ZOOM_INSET_PX: f32 = 4.0;

/// The factor by which the scale of `new` differs from that of `old` when `new` is `old` made larger or smaller and nothing else
/// (the same turn); 1 for views that differ only in where they are. `None` for any other change.
fn scale_ratio(old: &Affine2, new: &Affine2) -> Option<f32> {
    if old.matrix2 == new.matrix2 {
        return Some(1.0);
    }
    let (before, after) = (largest_scale(old), largest_scale(new));
    let ratio = after / before;
    if !(ratio.is_finite() && ratio > 0.0) {
        return None;
    }
    let off = (new.matrix2.x_axis - old.matrix2.x_axis * ratio).abs().max_element().max((new.matrix2.y_axis - old.matrix2.y_axis * ratio).abs().max_element());
    (off <= 1e-4 * after).then_some(ratio)
}

impl LastFrame {
    /// Whether a view of `size` pixels through `view` can be drawn from what was made for this one: it is this one moved, or (for a
    /// frame of plain shapes) made larger or smaller by a factor that no shape's triangles mind; and everything that can touch it
    /// was made for the old view (it lies inside the area kept).
    fn covers(&self, view: &Affine2, size: (u32, u32)) -> bool {
        // The rectangles of layers are clipped to the view, so a view of another size is another frame.
        if !self.layers.is_empty() && size != self.size {
            return false;
        }
        let Some(ratio) = scale_ratio(&self.view, view) else { return false };
        let mut inset = 0.0;
        if ratio != 1.0 {
            let Some((low, high_inverse)) = self.window else { return false };
            if !(ratio >= low && ratio * high_inverse <= 1.0 && (ZOOM_REUSE_RANGE.0..=ZOOM_REUSE_RANGE.1).contains(&ratio)) {
                return false;
            }
            inset = ZOOM_INSET_PX;
        }
        let Some((lo, hi)) = self.region else { return true };
        // The new view's corners, in the old view's pixels: all inside the area that was kept.
        let to_old = self.view * view.inverse();
        let (w, h) = (size.0 as f32, size.1 as f32);
        [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].into_iter().map(|c| to_old.transform_point2(c)).all(|c| {
            c.is_finite() && c.x >= lo.x + inset && c.x <= hi.x - inset && c.y >= lo.y + inset && c.y <= hi.y - inset
        })
    }
}

/// Where each node of a scene's tree is (the indices of its place among the children at every level; the art of symbols is not in
/// the tree), so that the nodes a journal names can be found without a walk. Valid for one shape of the tree (`structure`, a stamp of
/// the scene). Where a node's parent is in the scene is worked out when it is asked for, from the transforms along the path as they
/// are now: a group may have moved since the index was made.
struct NodeIndex {
    structure: Stamp,
    nodes: FastMap<NodeId, Place>,
    /// The paths of all the nodes, one after the other.
    paths: Vec<u32>,
}

#[derive(Clone, Copy)]
struct Place {
    path: u32,
    depth: u32,
}

impl NodeIndex {
    fn build(scene: &Scene, structure: Stamp) -> NodeIndex {
        fn visit(nodes: &[Node], path: &mut Vec<u32>, index: &mut NodeIndex) {
            for (at, node) in nodes.iter().enumerate() {
                path.push(at as u32);
                if !matches!(node.kind, NodeKind::Camera(_)) {
                    let start = index.paths.len() as u32;
                    index.paths.extend_from_slice(path);
                    index.nodes.insert(node.id, Place { path: start, depth: path.len() as u32 });
                    visit(node.children(), path, index);
                }
                path.pop();
            }
        }
        let mut index = NodeIndex { structure, nodes: FastMap::default(), paths: Vec::new() };
        visit(&scene.nodes, &mut Vec::new(), &mut index);
        index
    }

    /// The node `id` and the world transform of its parent.
    fn find<'s>(&self, scene: &'s Scene, id: NodeId) -> Option<(&'s Node, Affine2)> {
        let place = self.nodes.get(&id)?;
        let path = &self.paths[place.path as usize..(place.path + place.depth) as usize];
        let mut nodes = scene.nodes.as_slice();
        let (mut node, mut parent) = (None, Affine2::IDENTITY);
        for (step, &at) in path.iter().enumerate() {
            let here = nodes.get(at as usize)?;
            if step + 1 < path.len() {
                parent *= here.transform;
            }
            nodes = here.children();
            node = Some(here);
        }
        node.filter(|n| n.id == id).map(|n| (n, parent))
    }
}

/// The view as double-precision numbers, for working out what the shader will want near the middle of a pass.
struct View64 {
    x_axis: (f64, f64),
    y_axis: (f64, f64),
    translation: (f64, f64),
}

impl View64 {
    fn of(view: &Affine2) -> View64 {
        let c = |v: Vec2| (v.x as f64, v.y as f64);
        View64 { x_axis: c(view.matrix2.x_axis), y_axis: c(view.matrix2.y_axis), translation: c(view.translation) }
    }

    fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (self.x_axis.0 * p.0 + self.y_axis.0 * p.1 + self.translation.0, self.x_axis.1 * p.0 + self.y_axis.1 * p.1 + self.translation.1)
    }

    /// The scene point `center` (as the single-precision number the shader will use) that is under the pixel `middle` of the whole
    /// view, and where that very point is, in a pass whose pixels start at `shift` and are `scale` of the view's. A view that
    /// cannot be turned back (no area) gives the origin and the view's own translation: the old way, which still works.
    fn center_and_anchor(&self, middle: (f64, f64), shift: (f64, f64), scale: f64) -> ((f64, f64), (f64, f64)) {
        let det = self.x_axis.0 * self.y_axis.1 - self.y_axis.0 * self.x_axis.1;
        let moved = |p: (f64, f64)| ((p.0 - shift.0) * scale, (p.1 - shift.1) * scale);
        if !(det.is_finite() && det.abs() > 1e-30) {
            return ((0.0, 0.0), moved(self.translation));
        }
        let d = (middle.0 - self.translation.0, middle.1 - self.translation.1);
        let exact = ((self.y_axis.1 * d.0 - self.y_axis.0 * d.1) / det, (-self.x_axis.1 * d.0 + self.x_axis.0 * d.1) / det);
        let center = (exact.0 as f32 as f64, exact.1 as f32 as f64);
        if !(center.0.is_finite() && center.1.is_finite()) {
            return ((0.0, 0.0), moved(self.translation));
        }
        (center, moved(self.apply(center)))
    }
}

/// A buffer with room for the view uniform of `slots` passes.
fn create_uniform(alloc: &Alloc, slots: usize) -> Result<Tracked<wgpu::Buffer>, AllocError> {
    alloc.create_buffer(
        Category::Other,
        &wgpu::BufferDescriptor { label: Some("view"), size: slots as u64 * UNIFORM_SLOT_BYTES, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false },
    )
}

/// A texture for a layer that is to show `rect`: at its own size if the memory and the device allow, else at half, else at a
/// quarter. Returns the layer's number in the pool and how many times smaller than its place it is.
fn acquire_layer(layers: &mut LayerPool, limit: u32, rect: PixelRect) -> Result<(usize, u32), AllocError> {
    let mut divisor = 1;
    loop {
        let size = (rect.w.div_ceil(divisor), rect.h.div_ceil(divisor));
        let attempt = if size.0 > limit || size.1 > limit {
            Err(AllocError::TooLarge { what: "layer side", requested: size.0.max(size.1) as u64, limit: limit as u64 })
        } else {
            layers.acquire(size)
        };
        match attempt {
            Ok(layer) => return Ok((layer, divisor)),
            Err(AllocError::OverBudget { .. } | AllocError::TooLarge { .. }) if divisor < MAX_LAYER_DIVISOR => divisor *= 2,
            Err(error) => return Err(error),
        }
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    alloc: Alloc,
    errors: ErrorLog,
    pipeline: Pipeline,
    /// The view uniform of every pass of the frame, one slot each (see `UNIFORM_SLOT_BYTES`).
    uniform: Tracked<wgpu::Buffer>,
    uniform_slots: usize,
    uniform_generation: u64,
    bind_group: Option<((u64, u64, u64), wgpu::BindGroup)>,
    cache: TessCache,
    table: ShapeTable,
    ramps: RampAtlas,
    store: GeometryStore,
    list: DrawList,
    /// The numbers of the instances, kept from frame to frame.
    scopes: ScopeTable,
    layers: LayerPool,
    /// For each pass of the frame: the layer it draws into (a number for `layers`), `None` for the main pass.
    pass_layers: Vec<Option<usize>>,
    /// Where each layer's picture is in the view, for each pass (`None` for the main pass).
    pass_rects: Vec<Option<PixelRect>>,
    /// How many times smaller than its place each layer was drawn (1 for the main pass and for layers that fit).
    pass_divisors: Vec<u32>,
    /// `prepare` finished: the state is complete and can be drawn.
    ready: bool,
    last: Option<LastFrame>,
    /// Whether a frame may be made from the last one (see `set_frame_reuse`).
    reuse_frames: bool,
    targets: Targets,
    frame: Option<FrameMesh>,
    view_size: (u32, u32),
    /// Microseconds from the last submit until the card reported the work done (set by a callback, so measuring it never
    /// makes anything wait).
    completion_micros: Arc<AtomicU64>,
}

impl Renderer {
    pub fn new(gpu: &Gpu) -> Result<Renderer, AllocError> {
        let uniform = create_uniform(&gpu.alloc, 1)?;
        let pipeline = Pipeline::new(&gpu.device);
        Ok(Renderer {
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            alloc: gpu.alloc.clone(),
            errors: gpu.errors.clone(),
            layers: LayerPool::new(&gpu.alloc, &pipeline.layer_layout, &pipeline.layer_sampler),
            pipeline,
            uniform,
            uniform_slots: 1,
            uniform_generation: 0,
            bind_group: None,
            cache: TessCache::new(TessLimits::default()),
            table: ShapeTable::new(&gpu.alloc, &gpu.queue)?,
            ramps: RampAtlas::new(&gpu.alloc, &gpu.queue)?,
            store: GeometryStore::new(&gpu.alloc, &gpu.queue)?,
            list: DrawList::new(),
            scopes: ScopeTable::default(),
            pass_layers: Vec::new(),
            pass_rects: Vec::new(),
            pass_divisors: Vec::new(),
            ready: false,
            last: None,
            reuse_frames: true,
            targets: Targets::new(&gpu.alloc),
            frame: None,
            view_size: (0, 0),
            completion_micros: Arc::new(AtomicU64::new(NO_COMPLETION)),
        })
    }

    /// What the GPU path cannot draw in `scene`: the feature gate. Empty means the picture will be complete.
    pub fn unsupported(scene: &Scene) -> BTreeMap<Unsupported, usize> {
        unsupported(scene)
    }

    /// The same, node by node: which nodes the GPU path leaves out and why.
    pub fn unsupported_nodes(scene: &Scene) -> Vec<(NodeId, Unsupported)> {
        crate::draw_list::unsupported_nodes(scene)
    }

    /// The resolved single-sample picture of the last `prepare`d view, in the top left corner of the texture.
    pub fn picture(&self) -> Option<&wgpu::Texture> {
        self.targets.resolve_texture()
    }

    pub fn picture_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.resolve_view()
    }

    /// Size of the last view in pixels.
    pub fn view_size(&self) -> (u32, u32) {
        self.view_size
    }

    /// How long the card took over the last frame whose completion it has reported, in milliseconds. It arrives a frame or
    /// two after the submit (the report is a callback); `None` before the first.
    pub fn last_completion_ms(&self) -> Option<f64> {
        let micros = self.completion_micros.load(Ordering::Relaxed);
        (micros != NO_COMPLETION).then(|| micros as f64 / 1000.0)
    }

    /// Whether a frame whose scene, or camera, changed only a little may be made from the last one (the default). Turned off, every
    /// frame walks the scene: for tests that compare the two, and for finding out whether a wrong picture comes from reuse.
    pub fn set_frame_reuse(&mut self, reuse: bool) {
        self.reuse_frames = reuse;
    }

    /// How many milliseconds one frame may spend making the triangles of shapes that are not needed at once (see
    /// [`TessCache::set_budget`]); `None` makes everything at once. Frames that put work off say so in `FrameStats::deferred`.
    pub fn set_tessellation_budget(&mut self, milliseconds: Option<f64>) {
        self.cache.set_budget(milliseconds);
    }

    /// The same, as it is set now.
    pub fn tessellation_budget(&self) -> Option<f64> {
        self.cache.budget()
    }

    /// Changes when the picture texture was replaced.
    pub fn picture_generation(&self) -> u64 {
        self.targets.generation()
    }

    pub fn targets(&self) -> &Targets {
        &self.targets
    }

    pub fn cache(&self) -> &TessCache {
        &self.cache
    }

    pub fn store(&self) -> &GeometryStore {
        &self.store
    }

    pub fn table(&self) -> &ShapeTable {
        &self.table
    }

    /// Brings everything up to date for drawing `scene` through `view` (world to pixels) into `size` pixels.
    pub fn prepare(&mut self, scene: &Scene, view: Affine2, size: (u32, u32), options: &RenderOptions) -> Result<FrameStats, RenderError> {
        if let Some(why) = self.errors.lost() {
            return Err(RenderError::DeviceLost(why));
        }
        let started = Instant::now();
        let mut stats = FrameStats::default();
        let view_size = (size.0.max(1), size.1.max(1));

        // The same scene seen from nearby: nothing to walk, make or send, only the view moves. Or the same scene with some shapes
        // changed, and the scene says which: only their items are brought up to date.
        let revs = scene.revision();
        if self.ready && self.reuse_frames {
            if let Some(mut last) = self.last.take() {
                let same_setup = last.camera_frame == options.camera_frame
                    && last.skip_unsupported == options.skip_unsupported
                    && last.region.is_some() == options.cull
                    && last.covers(&view, view_size);
                // (Without the camera's frame in the picture a camera is nothing to draw: a game that moves or zooms its camera changes
                // the scene and the view, and the picture of the scene stays as it was.)
                let same_picture = last.revs == revs || (!options.camera_frame && scene.only_cameras_changed_since(&last.revs, last.epoch));
                if same_setup && same_picture {
                    last.revs = revs;
                    if let Some(table_bytes) = self.place_layers(&last, &view, view_size) {
                        stats = FrameStats {
                            reused_frame: true,
                            items: last.stats.items,
                            draw_calls: last.stats.draw_calls,
                            triangles: last.stats.triangles,
                            passes: last.stats.passes,
                            layers: last.stats.layers,
                            layers_flattened: last.stats.layers_flattened,
                            clips_flattened: last.stats.clips_flattened,
                            layer_pixels: last.stats.layer_pixels,
                            ramps_in_use: last.stats.ramps_in_use,
                            skipped: last.stats.skipped.clone(),
                            table_bytes_uploaded: table_bytes,
                            ..FrameStats::default()
                        };
                        stats.targets_replaced = self.targets.ensure(size, &options.targets, started)?;
                        self.view_size = view_size;
                        self.write_view_uniforms(view)?;
                        stats.update_ms = started.elapsed().as_secs_f64() * 1000.0;
                        self.last = Some(last);
                        return Ok(stats);
                    }
                }
                // (An edit is only brought up to date at the scale the frame was made for: the shapes it touches are made for it.)
                if same_setup && last.patchable && last.view.matrix2 == view.matrix2 && last.epoch == scene.journal_epoch() {
                    if let Some(changes) = scene.nodes_changed_since(&last.revs) {
                        if let Some(mut patched) = self.try_patch(scene, &mut last, &changes, view)? {
                            patched.targets_replaced = self.targets.ensure(size, &options.targets, started)?;
                            self.view_size = view_size;
                            self.write_view_uniforms(view)?;
                            patched.update_ms = started.elapsed().as_secs_f64() * 1000.0;
                            last.revs = revs;
                            last.stats = FrameStats { patched_items: 0, ..patched.clone() };
                            self.last = Some(last);
                            return Ok(patched);
                        }
                    }
                }
            }
        }
        self.last = None;

        let cull = options.cull.then(|| Cull::new(view, size));
        let tess = self.cache.update_in(scene, largest_scale(&view), cull.as_ref());
        stats.tessellate_ms = started.elapsed().as_secs_f64() * 1000.0;
        stats.shapes_culled = tess.culled;
        stats.deferred = tess.deferred;
        stats.shapes_tessellated = tess.tessellated.len();
        stats.shapes_reused = tess.reused;
        stats.warnings = tess.warnings;

        if options.camera_frame {
            self.update_frame(scene);
        } else {
            self.frame = None;
        }
        self.ready = false;
        self.layers.begin_frame();
        self.scopes.begin_frame();
        let collecting = Instant::now();
        let collected = collect(scene, &self.cache, self.frame.as_ref(), &mut self.scopes, view, view_size, cull);
        self.scopes.end_frame();
        stats.collect_ms = collecting.elapsed().as_secs_f64() * 1000.0;
        let flat = collected.is_flat();
        let (collected_window, collected_instanced) = (collected.window, collected.instanced);
        if !collected.unsupported.is_empty() {
            if !options.skip_unsupported {
                return Err(RenderError::Unsupported(collected.unsupported));
            }
            stats.skipped = collected.unsupported.clone();
        }
        if collected.items() > crate::draw_list::MAX_ITEMS {
            return Err(RenderError::TooManyItems(collected.items()));
        }

        // A texture for every layer: the passes before the last one. (The pool takes them all back at the next frame.) A layer
        // that does not fit the memory is drawn at half, then a quarter, of its size and shown stretched, and the frame says so:
        // a layer that is left out would give wrong colors (plan D13). This comes first because the rows of the composites
        // depend on it.
        let pass_rects = collected.pass_rects();
        if pass_rects.len() > MAX_PASSES {
            return Err(RenderError::TooManyLayers(pass_rects.len() - 1));
        }
        self.pass_rects = pass_rects;
        self.pass_layers.clear();
        self.pass_divisors.clear();
        let limit = self.device.limits().max_texture_dimension_2d;
        for number in 0..self.pass_rects.len() {
            let (layer, divisor) = match self.pass_rects[number] {
                Some(rect) => {
                    let (layer, divisor) = acquire_layer(&mut self.layers, limit, rect)?;
                    (Some(layer), divisor)
                }
                None => (None, 1),
            };
            self.pass_layers.push(layer);
            self.pass_divisors.push(divisor);
        }
        self.layers.end_frame();
        stats.passes = self.pass_rects.len();
        stats.layers = self.layers.in_use();
        stats.layers_reduced = self.pass_divisors.iter().filter(|d| **d > 1).count();
        (stats.layers_flattened, stats.clips_flattened) = (collected.layers_flattened, collected.clips_flattened);
        stats.layer_pixels = self.layers.pixels_in_use();

        let listing = Instant::now();
        let written_before = self.store.vertices.written_bytes() + self.store.indices.written_bytes();
        self.ramps.begin_frame();
        let report: DrawReport = self.list.update(&collected, &self.pass_divisors, &mut self.table, &mut self.ramps, &mut self.store)?;
        self.ramps.end_frame();
        let layer_map = LayerMap::of(&collected, &report);
        stats.ramps_in_use = self.ramps.len();
        stats.table_bytes_uploaded += self.ramps.upload();
        stats.geometry_bytes_uploaded = self.store.vertices.written_bytes() + self.store.indices.written_bytes() - written_before;
        stats.table_bytes_uploaded += self.table.upload();
        stats.items = report.items;
        stats.draw_calls = report.chunks;
        stats.triangles = report.triangles;
        stats.chunks_rebuilt = report.chunks_rebuilt;
        stats.vertices_compacted = report.vertices_compacted;
        stats.indices_compacted = report.indices_compacted;
        stats.vertices_uploaded = report.vertices_uploaded;
        stats.list_ms = listing.elapsed().as_secs_f64() * 1000.0;

        stats.targets_replaced = self.targets.ensure(size, &options.targets, started)?;
        self.view_size = view_size;
        self.write_view_uniforms(view)?;
        let generations = (self.table.generation(), self.ramps.generation(), self.uniform_generation);
        if self.bind_group.as_ref().is_none_or(|(seen, _)| *seen != generations) {
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene"),
                layout: &self.pipeline.bind_group_layout,
                entries: &[
                    // One slot of the uniform buffer at a time, chosen with a dynamic offset by every pass.
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniform, offset: 0, size: wgpu::BufferSize::new(VIEW_UNIFORM_BYTES) }) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(self.table.view()) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(self.ramps.view()) },
                ],
            });
            self.bind_group = Some((generations, group));
        }
        self.ready = true;
        stats.update_ms = started.elapsed().as_secs_f64() * 1000.0;
        // (A layer drawn smaller than its place has a texture of another size, which a move of the layers would not know about.)
        if stats.layers_reduced == 0 && stats.deferred == 0 {
            self.last = Some(LastFrame {
                revs,
                epoch: scene.journal_epoch(),
                patchable: flat && stats.skipped.is_empty(),
                index: None,
                camera_frame: options.camera_frame,
                skip_unsupported: options.skip_unsupported,
                view,
                region: cull.map(|c| c.region()),
                window: (flat && !collected_instanced).then_some(collected_window),
                layers: layer_map,
                size: view_size,
                stats: stats.clone(),
            });
        }
        Ok(stats)
    }

    /// Moves the layers of the last frame to where `view` puts them: it is that frame's view moved (the same scale and turn), so each
    /// layer's box has moved by the same number of pixels, and what is in a layer is drawn again, as in every frame, into the same
    /// picture. The rectangle of every layer is found again (it is clipped to the view), and the lines of the shape table that say
    /// where the composites are written. Returns the bytes of table lines sent, or `None` when it cannot be done with what there is:
    /// a layer that was in the view is out of it, or one that was out has come in (nothing was collected for what is inside it), or a
    /// texture of the size class a layer now needs cannot be had. Nothing has changed then except that some layers may have a
    /// texture of another class, which the frame made whole starts again anyway.
    fn place_layers(&mut self, last: &LastFrame, view: &Affine2, size: (u32, u32)) -> Option<u64> {
        if last.layers.is_empty() {
            return Some(0);
        }
        let moved = last.layers.moved(view.translation - last.view.translation, size, self.pass_rects.len())?;
        // A layer that grew or shrank past the class of its texture gets one of the new class (a layer at the edge of the view does so
        // as it goes in or out). If that cannot be had, the frame is made whole, which starts every layer again.
        let limit = self.device.limits().max_texture_dimension_2d;
        for (pass, rect) in moved.rects.iter().enumerate() {
            if let (Some(rect), Some(layer)) = (rect, self.pass_layers[pass]) {
                if size_class((rect.w, rect.h), limit) != self.layers.get(layer).size() && self.layers.resize(layer, (rect.w, rect.h)).is_err() {
                    return None;
                }
            }
        }
        self.pass_rects = moved.rects;
        for (row, place, depth, opacity) in moved.rows {
            self.table.set(row, RowData::layer(place, depth, opacity, 1.0));
        }
        Some(self.table.upload())
    }

    /// Brings the items of the nodes that `changes` names up to date, when that is all it takes: every named node is either a shape of
    /// the last frame (not skinned, visible, at full opacity, with solid colors) whose parts are the same as before, or a group or an
    /// instance that only moved (visible, at full opacity, no clip, no bone): then everything below it is walked again, and its items
    /// must be the ones the last frame had. Nothing else may have changed in the scene. Returns `Ok(None)` and changes nothing that
    /// matters when it is not so (the caller then makes the whole frame); an `Err` is a failure to get memory half way, as for any
    /// frame.
    fn try_patch(&mut self, scene: &Scene, last: &mut LastFrame, changes: &[(NodeId, Changes)], view: Affine2) -> Result<Option<FrameStats>, RenderError> {
        let structure = scene.revision().structure;
        if last.index.as_ref().is_none_or(|index| index.structure != structure) {
            last.index = Some(NodeIndex::build(scene, structure));
        }
        let index = last.index.as_ref().expect("just made");
        let zoom = largest_scale(&view);

        struct Changed<'s> {
            node: &'s Node,
            shape: &'s Shape,
            world: Affine2,
            geometry: bool,
            fill: Option<[f32; 4]>,
            stroke: Option<([f32; 4], f32)>,
        }
        let mut changed: Vec<Changed> = Vec::new();
        let mut moved: Vec<(&Node, Affine2)> = Vec::new();
        for &(id, kinds) in changes {
            if id.0 == 0 || kinds.intersects(Changes::STRUCTURE) {
                return Ok(None); // not one node, or the tree changed
            }
            if !kinds.intersects(Changes::GEOMETRY | Changes::TRANSFORM | Changes::PAINT) {
                continue; // a name, a script: nothing that is drawn
            }
            // (The lookup is the index's: `scene.find` searches the tree from the top, which for the thousands of nodes of a game that
            // restyles its whole map in a frame was 11 ms of 14. The index has no cameras, so a miss is the one place to search.)
            let Some((node, parent)) = index.find(scene, id) else {
                if !last.camera_frame && scene.find(id).is_some_and(|node| matches!(node.kind, NodeKind::Camera(_)) && node.children().is_empty()) {
                    continue; // a camera, with no frame in the picture: nothing that is drawn
                }
                return Ok(None);
            };
            let shape = match &node.kind {
                NodeKind::Shape(shape) => shape,
                NodeKind::Group(_) | NodeKind::Instance(_) => {
                    // A node that only moved. (A change to its opacity, its visibility or its clip is a different frame; a bone moves
                    // the shapes bound to it, wherever they are.)
                    if kinds.intersects(Changes::GEOMETRY | Changes::PAINT) || !node.visible || node.opacity != 1.0 || node.clip || node.bone.is_some() {
                        return Ok(None);
                    }
                    moved.push((node, parent));
                    continue;
                }
                NodeKind::Camera(_) => return Ok(None),
            };
            if shape.skin.is_some() || !node.visible || node.opacity != 1.0 {
                return Ok(None);
            }
            let solid = |paint: &Paint| match paint {
                Paint::Solid(color) => Some(premultiplied(*color)),
                Paint::Gradient(_) => None,
            };
            let fill = match &shape.fill {
                Some(paint) => match solid(paint) {
                    Some(color) => Some(color),
                    None => return Ok(None),
                },
                None => None,
            };
            let stroke = match shape.stroke.as_ref().filter(|s| s.width > 0.0 && s.width.is_finite()) {
                Some(stroke) => match solid(&stroke.paint) {
                    Some(color) => Some((color, stroke.width)),
                    None => return Ok(None),
                },
                None => None,
            };
            // A shape that has no items in the list (it was hidden, or outside the view) needs the whole frame; finding that out is
            // cheap, making its triangles first and then finding it out is not.
            if !self.list.contains(&(id, Part::Fill, 0)) && !self.list.contains(&(id, Part::Stroke, 0)) {
                return Ok(None);
            }
            changed.push(Changed { node, shape, world: parent * node.transform, geometry: kinds.intersects(Changes::GEOMETRY), fill, stroke });
        }

        // The triangles that have to be made again (the outline changed, or the shape is at another scale now).
        let mut remade = 0;
        for c in &changed {
            if c.geometry || !self.cache.holds(c.node, zoom * largest_scale(&c.world)) {
                self.cache.remake(c.node, c.shape, &c.world, zoom);
                remade += 1;
            }
        }
        // The shapes below the nodes that moved: made as the full walk would make them (those that came near enough, those at another
        // zoom bucket), with the cull of the frame being patched.
        let cull = last.region.is_some().then(|| Cull::new(last.view, last.size));
        let mut tessellated = remade;
        for &(node, parent) in &moved {
            let update = self.cache.update_node(scene, node, parent, zoom, cull.as_ref());
            if update.deferred > 0 || update.skinned > 0 {
                return Ok(None); // work is waiting, or the node holds what the cache cannot make
            }
            tessellated += update.tessellated.len();
        }
        // Their items. Without the cull, so that none is missing for being out of the area (one that was left out then is not in the
        // list, and the frame is made whole below).
        let mut below: Vec<ItemEdit> = Vec::new();
        for &(node, parent) in &moved {
            let collected = collect_node(scene, &self.cache, &mut self.scopes, last.view, last.size, None, node, parent);
            let Some(mut items) = collected.as_edits() else { return Ok(None) };
            if items.iter().any(|item| !self.list.contains(&item.key)) {
                return Ok(None);
            }
            if let Some(window) = &mut last.window {
                *window = (window.0.max(collected.window.0), window.1.max(collected.window.1));
            }
            below.append(&mut items);
        }
        let mut edits: Vec<ItemEdit> = below;
        edits.reserve(changed.len() * 2);
        for c in &changed {
            let Some(entry) = self.cache.get(c.node.id) else { return Ok(None) };
            let made_from = entry.identity();
            // What the shape's triangles now allow of a change of zoom is part of what the frame allows.
            if let Some(window) = &mut last.window {
                let (low, high_inverse) = entry.scale_window(zoom * largest_scale(&c.world));
                *window = (window.0.max(low), window.1.max(high_inverse));
            }
            let (fill_key, stroke_key) = ((c.node.id, Part::Fill, 0), (c.node.id, Part::Stroke, 0));
            // The parts must be the ones the last frame drew: a part that appeared or went changes the order of the items.
            let fill = c.fill.zip(entry.mesh.fill.as_ref());
            let stroke = c.stroke.zip(entry.mesh.stroke.as_ref());
            if self.list.contains(&fill_key) != fill.is_some() || self.list.contains(&stroke_key) != stroke.is_some() {
                return Ok(None);
            }
            if let Some((color, mesh)) = fill {
                edits.push(ItemEdit { key: fill_key, world: c.world, color, width: 0.0, mesh, made_from });
            }
            if let Some(((color, width), mesh)) = stroke {
                edits.push(ItemEdit { key: stroke_key, world: c.world, color, width, mesh, made_from });
            }
        }

        let written_before = self.store.vertices.written_bytes() + self.store.indices.written_bytes();
        let (cache, frame) = (&self.cache, self.frame.as_ref());
        let mesh_of = |key: Key, made_from: (Stamp, i32)| match key.1 {
            Part::Fill => cache.entry_made_from(key.0, made_from)?.mesh.fill.as_ref(),
            Part::Stroke => cache.entry_made_from(key.0, made_from)?.mesh.stroke.as_ref(),
            Part::Frame => frame?.mesh.fill.as_ref(),
            _ => None,
        };
        let listed = Instant::now();
        let report = match self.list.patch(&edits, &mut self.table, &mut self.store, mesh_of) {
            Ok(report) => report,
            Err(error) => {
                // Some of it was done: the next frame must start from nothing that is half true.
                self.ready = false;
                return Err(error.into());
            }
        };
        let mut stats = FrameStats {
            patched_items: report.items,
            items: last.stats.items,
            draw_calls: last.stats.draw_calls,
            triangles: last.stats.triangles,
            passes: last.stats.passes,
            ramps_in_use: last.stats.ramps_in_use,
            skipped: last.stats.skipped.clone(),
            shapes_tessellated: tessellated,
            chunks_rebuilt: report.chunks_rebuilt,
            vertices_uploaded: report.vertices_uploaded,
            ..FrameStats::default()
        };
        stats.table_bytes_uploaded = self.table.upload();
        stats.geometry_bytes_uploaded = self.store.vertices.written_bytes() + self.store.indices.written_bytes() - written_before;
        stats.list_ms = listed.elapsed().as_secs_f64() * 1000.0;
        Ok(Some(stats))
    }

    /// Writes the view uniform of every pass: the camera, shifted by the corner of the layer for a layer's pass (its picture
    /// starts there), and the size of the target the pass draws in.
    fn write_view_uniforms(&mut self, view: Affine2) -> Result<(), AllocError> {
        let passes = self.pass_rects.len().max(1);
        if passes > self.uniform_slots {
            let slots = passes.next_power_of_two();
            self.uniform = create_uniform(&self.alloc, slots)?;
            self.uniform_slots = slots;
            self.uniform_generation += 1;
        }
        let slot = UNIFORM_SLOT_BYTES as usize;
        let mut bytes = vec![0u8; passes * slot];
        let m = view.matrix2;
        let view64 = View64::of(&view);
        for (number, rect) in self.pass_rects.iter().enumerate() {
            let (shift, size) = match (rect, self.pass_layers[number]) {
                (Some(rect), Some(layer)) => (Vec2::new(rect.x as f32, rect.y as f32), self.layers.get(layer).size()),
                _ => (Vec2::ZERO, self.view_size),
            };
            // A layer that is drawn smaller than it is shown has the whole picture scaled down: 1 / divisor of every length.
            let k = 1.0 / self.pass_divisors[number] as f32;
            // The scene point under the middle of what the pass shows (in the pixels of the whole view), and where it is in this
            // pass: worked out in double precision, so that everything the shader adds up is small (see the view uniform).
            let middle = match rect {
                Some(rect) => (shift.x as f64 + rect.w as f64 * 0.5, shift.y as f64 + rect.h as f64 * 0.5),
                None => (self.view_size.0 as f64 * 0.5, self.view_size.1 as f64 * 0.5),
            };
            let (center, anchor) = view64.center_and_anchor(middle, (shift.x as f64, shift.y as f64), k as f64);
            let uniform: [f32; 16] = [
                m.x_axis.x * k, m.x_axis.y * k, m.y_axis.x * k, m.y_axis.y * k, //
                anchor.0 as f32, anchor.1 as f32, 0.0, 0.0, //
                size.0 as f32, size.1 as f32, 0.0, 0.0, //
                center.0 as f32, center.1 as f32, 0.0, 0.0,
            ];
            bytes[number * slot..number * slot + VIEW_UNIFORM_BYTES as usize].copy_from_slice(bytemuck::cast_slice(&uniform));
        }
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        Ok(())
    }

    /// Encodes the passes that draw what `prepare` set up: every layer, then the main pass. Does nothing before a `prepare`
    /// that succeeded.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        let (true, Some(color), Some(depth), Some(resolve), Some((_, group))) =
            (self.ready, self.targets.msaa_color_view(), self.targets.msaa_depth_view(), self.targets.resolve_view(), self.bind_group.as_ref())
        else {
            return;
        };
        let passes = self.list.pass_count();
        for number in 0..passes {
            // The picture of a layer is drawn in the scratch of its class and resolved into its texture; the last pass is the
            // main one, into the targets of the view.
            let (target, size, scissor, label) = match self.pass_layers[number] {
                Some(layer) => {
                    let (scratch, texture) = (self.layers.scratch_of(layer), self.layers.get(layer));
                    let rect = self.pass_rects[number].expect("a layer has a rectangle");
                    let divisor = self.pass_divisors[number];
                    ((scratch.color_view(), scratch.depth_view(), texture.view()), texture.size(), (rect.w.div_ceil(divisor), rect.h.div_ceil(divisor)), "layer")
                }
                None => ((color, depth, resolve), self.view_size, self.view_size, "scene"),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.0,
                    depth_slice: None,
                    resolve_target: Some(target.2),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target.1,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(0.0, 0.0, size.0 as f32, size.1 as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, scissor.0, scissor.1);
            pass.set_bind_group(0, group, &[number as u32 * UNIFORM_SLOT_BYTES as u32]);
            pass.set_vertex_buffer(0, self.store.vertices.buffer().slice(..));
            pass.set_index_buffer(self.store.indices.buffer().slice(..), wgpu::IndexFormat::Uint32);
            let mut current = None;
            for draw in self.list.draws(number) {
                if draw.kind == DrawKind::Composite {
                    pass.set_pipeline(&self.pipeline.composite);
                    pass.set_bind_group(1, self.layers.get(self.pass_layers[draw.layer].expect("a layer's pass")).bind_group(), &[]);
                    current = None;
                } else if current != Some(draw.kind) {
                    pass.set_pipeline(match draw.kind {
                        DrawKind::MaskPush => &self.pipeline.mask_push,
                        DrawKind::MaskPop => &self.pipeline.mask_pop,
                        DrawKind::Items | DrawKind::Composite => &self.pipeline.pipeline,
                    });
                    current = Some(draw.kind);
                }
                pass.set_stencil_reference(draw.level);
                pass.draw_indexed(draw.start..draw.start + draw.count, 0, 0..1);
            }
        }
    }

    /// `prepare`, then draw and submit. For headless use (tests, the benchmark).
    pub fn render(&mut self, scene: &Scene, view: Affine2, size: (u32, u32), options: &RenderOptions) -> Result<FrameStats, RenderError> {
        let mut stats = self.prepare(scene, view, size, options)?;
        let started = Instant::now();
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
        self.record(&mut encoder);
        let submitted = Instant::now();
        self.queue.submit([encoder.finish()]);
        let done = self.completion_micros.clone();
        self.queue.on_submitted_work_done(move || done.store(submitted.elapsed().as_micros() as u64, Ordering::Relaxed));
        stats.record_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(stats)
    }

    /// Makes the triangles of the camera frame's background, again only when the camera changed.
    fn update_frame(&mut self, scene: &Scene) {
        let view = scene.camera_view();
        if self.frame.as_ref().is_some_and(|f| f.view == view) {
            return;
        }
        let mut shape = Shape::new(view.outline());
        shape.fill = Some(Paint::Solid(view.background));
        shape.stroke = None;
        self.frame = Some(FrameMesh { view, bucket: 0, identity: Stamp::fresh(), mesh: tessellate_shape(&shape, 0, &TessLimits::default()) });
    }

    /// The bytes this renderer holds on the card, by what they are for.
    pub fn memory(&self) -> Vec<(Category, u64)> {
        self.alloc.snapshot()
    }
}
