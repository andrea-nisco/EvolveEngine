//! The draw list and its chunks (plan D11, D17, task 2.4).
//!
//! Every frame the scene is walked once, in document order (bottom to top). Each shape gives up to two **draw items**, its
//! fill and then its stroke. An item has
//! - a **row** in the shape table (kept as long as the item exists, so moving or recoloring a shape rewrites that row and
//!   nothing else),
//! - a **depth value**, strictly increasing along the list, so that the depth test "greater" keeps painter's order and an
//!   item cannot blend over itself,
//! - **vertices** in the geometry store, uploaded when the shape was tessellated, with the row baked in.
//!
//! Items are grouped into **chunks** of [`CHUNK_ITEMS`] consecutive items. A chunk is one index list (the vertex positions of
//! its items' triangles added up) in the index buffer and costs one draw call. A chunk is rebuilt only when what it holds
//! changed: an item's vertices were replaced, moved by a compaction, or the items that fall into it are different. Editing
//! the geometry of one shape therefore rebuilds one chunk; inserting or removing an item shifts the later chunks.
//!
//! **Passes, ops and chunks** (plan Phase 4). A frame is a list of passes: one for every layer, inner layers before the layers
//! that use them, and the main pass last. A pass is a list of ops in drawing order: an item, a clip mask going into the
//! stencil, the same mask coming out, or the composite of a layer (a rectangle showing the layer's picture). Items are
//! grouped into chunks at one clip level; every mask and every composite is a chunk of its own.
//!
//! **Opacity** (plans 3.6 and 4.4) goes into the rows when that is exactly what the CPU renderer's layer gives: for a shape
//! that is only a fill or only a stroke (its opacity is the row's), for a shape with both whose paints are opaque (the stroke
//! is drawn first and the fill gets the *same* depth value, so that the depth test "greater" lets the fill show only where
//! the stroke is not, and each pixel gets one of the two at the shape's opacity), and for a group with one child or whose
//! children cannot touch each other (their boxes on the screen, strokes and anti-aliasing included, are apart): the group's
//! opacity is multiplied into the children. Everything else with opacity is drawn on a **layer**: its own pass, as large as
//! the box of the node on the screen, composited with the opacity.
//!
//! What the GPU path cannot draw yet (a skinned shape inside a symbol instance, a skinned clip mask) is not drawn; it is counted, and
//! [`Collected::unsupported`] says what and how often. The caller decides whether that means "use the CPU renderer for this
//! view" (the feature gate) or "draw what you can" (timing only).

use crate::alloc::AllocError;
use crate::cull::{control_extent, pixel_box, Cull, SLACK_PX};
use crate::ranges::{class_size, Block};
use crate::store::GeometryStore;
use crate::ramps::RampAtlas;
use crate::table::{premultiplied, PaintKind, RowData, ShapeTable};
use crate::tess_cache::{largest_scale, CachedShape, TessCache, MAX_INSTANCE_DEPTH};
use crate::tessellate::ShapeMesh;
use crate::vertex::{Mesh, Vertex};
use evolve_engine::game_camera::CameraView;
use evolve_engine::geometry::Affine2;
use evolve_engine::revision::Stamp;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Gradient, GradientKind, GradientStop, Paint};
use evolve_engine::symbol::{Instance, Overrides};
use evolve_engine::tiny_skia as sk;
use evolve_engine::Vec2;
use crate::fast_map::FastMap;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Items per chunk, and so per draw call.
pub const CHUNK_ITEMS: usize = 512;
/// Depth values are multiples of 2^-23, which a 24-bit depth buffer tells apart; so there can be fewer than this many items.
pub const MAX_ITEMS: usize = (1 << 23) - 2;
const DEPTH_STEP: f32 = 1.0 / 8_388_608.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Fill,
    Stroke,
    /// The background of the game camera's frame (not a node).
    Frame,
    /// The silhouette of the top child of a clip group, drawn into the stencil (the node's fill mesh, made whatever its paints
    /// are). It has a row (for the transform) but is never painted.
    Mask,
    /// The composite of a node's layer: a rectangle (four vertices, always the same) with a row that says where it is.
    Layer,
}

/// Which instance of a symbol an item is in: 0 for the scene itself, otherwise a number that stands for the chain of instances
/// (see [`ScopeTable`]). The art of a symbol is the same nodes for every instance, so a node and a part do not say which item.
pub type Scope = u32;

/// Which draw item: a part of a node, in a scope. The frame uses node id 0, which no node has.
pub type Key = (NodeId, Part, Scope);
pub const FRAME_NODE: NodeId = NodeId(0);

/// Things the GPU path does not draw yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unsupported {
    /// A shape bent by bones that cannot be drawn: a skinned clip mask, or a skinned shape inside a symbol instance (the instances
    /// would need a pose each). Skinned shapes elsewhere are drawn from their outline as the bones bend it (plan 6.1).
    Skinned,
}

/// Gives the instances numbers (scopes) that stay the same from frame to frame, so that an item is the same item in every
/// frame and keeps its row and its vertices. A scope stands for an instance node seen from inside another scope (instances
/// can be nested in symbols). Numbers of instances that were not seen in a frame are forgotten.
#[derive(Default)]
pub struct ScopeTable {
    ids: FastMap<(Scope, NodeId), (Scope, u64)>,
    next: Scope,
    generation: u64,
}

impl ScopeTable {
    pub fn begin_frame(&mut self) {
        self.generation += 1;
    }

    /// Forgets the instances that were not seen in this frame.
    pub fn end_frame(&mut self) {
        let generation = self.generation;
        self.ids.retain(|_, (_, seen)| *seen == generation);
    }

    /// The scope of `instance` seen from inside `parent`.
    fn scope(&mut self, parent: Scope, instance: NodeId) -> Scope {
        let generation = self.generation;
        let next = &mut self.next;
        let entry = self.ids.entry((parent, instance)).or_insert_with(|| {
            *next = next.checked_add(1).unwrap_or(1);
            (*next, generation)
        });
        entry.1 = generation;
        entry.0
    }

    /// Instances known now.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// One thing a pass does, in order (indices are into the specs of the walk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Draw an item.
    Item(usize),
    /// Draw a clip mask into the stencil only, adding one where the current clip level passes: from then on items are drawn
    /// one level deeper.
    Push(usize),
    /// Draw the same mask again, taking the one away: back to the level before the push.
    Pop(usize),
    /// Draw the picture of a layer (the pass with this number) as a rectangle. `spec` is its item.
    Composite { spec: usize, layer: usize },
}

/// What a draw call is for, and so which pipeline it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    /// Painted items, tested against the stencil level.
    Items,
    /// A clip mask going in (stencil incremented where the level passes).
    MaskPush,
    /// A clip mask coming out (stencil decremented where the level passes).
    MaskPop,
    /// The picture of a layer.
    Composite,
}

/// One draw call of a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    pub kind: DrawKind,
    /// The stencil reference: the level the call is tested against (for a push, the level before it; for a pop, the level
    /// inside the mask).
    pub level: u32,
    /// For a composite: the number of the pass that made the picture it shows (a pass before this one).
    pub layer: usize,
    /// First index and number of indices.
    pub start: u32,
    pub count: u32,
}

/// A rectangle of whole pixels: the size of a layer, and where it is in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl PixelRect {
    /// The pixels that a box (min, max) touches, within a view of `size` pixels; `None` when there are none.
    pub fn of_box(min: Vec2, max: Vec2, size: (u32, u32)) -> Option<PixelRect> {
        if !(min.is_finite() && max.is_finite()) {
            return None;
        }
        let x0 = min.x.floor().clamp(0.0, size.0 as f32) as i32;
        let y0 = min.y.floor().clamp(0.0, size.1 as f32) as i32;
        let x1 = max.x.ceil().clamp(0.0, size.0 as f32) as i32;
        let y1 = max.y.ceil().clamp(0.0, size.1 as f32) as i32;
        (x1 > x0 && y1 > y0).then_some(PixelRect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 })
    }
}

/// One pass of a frame: what it does, and where its picture is if it is a layer (the main pass has no rectangle).
pub struct PassSpec {
    ops: Vec<Op>,
    pub rect: Option<PixelRect>,
}

/// The triangles of the camera frame's background, made by the renderer (it is not a node).
pub struct FrameMesh {
    pub view: CameraView,
    pub bucket: i32,
    pub identity: Stamp,
    pub mesh: ShapeMesh,
}

/// What an item is painted with, as the shape table wants it.
#[derive(Clone)]
enum ItemPaint<'a> {
    /// A premultiplied color.
    Solid([f32; 4]),
    /// A gradient: how the position along it is computed from a point of the shape's own space (see `table.rs`), and the
    /// stops that make its ramp.
    Gradient { kind: PaintKind, geometry: [f32; 8], stops: &'a [GradientStop] },
    /// A layer's composite: its rectangle (x, y, width, height) in the pixels of the pass it is drawn in.
    Layer([f32; 4]),
}

struct Spec<'a> {
    key: Key,
    world: Affine2,
    paint: ItemPaint<'a>,
    width: f32,
    /// The opacity of the node this item belongs to (1 for most).
    opacity: f32,
    /// This item gets the same depth value as the one before it (a stroke and a fill of a shape with opacity, see above).
    shares_depth: bool,
    /// The triangles of the item (shared by all the instances of a symbol that are at the same scale) and what they were made
    /// from: the geometry stamp and the scale bucket (or the frame's identity).
    mesh: &'a Mesh,
    made_from: (Stamp, i32),
}

impl ItemPaint<'_> {
    /// Whether every pixel painted with it is fully opaque.
    fn is_opaque(&self) -> bool {
        match self {
            ItemPaint::Solid(color) => color[3] >= 1.0,
            ItemPaint::Gradient { stops, .. } => stops.iter().all(|stop| stop.color.a == 255),
            ItemPaint::Layer(_) => false,
        }
    }
}

/// The shape-table form of a paint, or `None` when the CPU renderer draws nothing for it (a gradient with no stops, one that
/// cannot be drawn at all). Asking tiny-skia decides that, and also which gradients are really a single color: one stop, or a
/// degenerate gradient (start and end the same, a radius of zero).
fn item_paint(paint: &Paint) -> Option<ItemPaint<'_>> {
    match paint {
        Paint::Solid(color) => Some(ItemPaint::Solid(premultiplied(*color))),
        Paint::Gradient(gradient) => match evolve_engine::render::to_skia_shader(paint)? {
            sk::Shader::SolidColor(c) => Some(ItemPaint::Solid([c.red() * c.alpha(), c.green() * c.alpha(), c.blue() * c.alpha(), c.alpha()])),
            _ => match gradient_geometry(gradient) {
                Some((kind, geometry)) => Some(ItemPaint::Gradient { kind, geometry, stops: &gradient.stops }),
                // tiny-skia makes a gradient of it but its geometry cannot be used (a transform that cannot be inverted):
                // it paints the last color, and so do we.
                None => gradient.stops.last().map(|stop| ItemPaint::Solid(premultiplied(stop.color))),
            },
        },
    }
}

/// How the shader finds the position along a gradient (0 at the first stop, 1 at the last) from a point `p` of the shape's
/// own space. The gradient is defined in its own space, which its transform places in the shape's: `q = inverse(T) p`.
/// - Linear, from `s` to `e`: the position is `(q - s) . d / |d|^2` with `d = e - s`, which is `a x + b y + c` in `p`.
/// - Radial, around `c` with radius `r`: the position is `|q - c| / r`, the length of `M p + offset`.
pub fn gradient_geometry(gradient: &Gradient) -> Option<(PaintKind, [f32; 8])> {
    let inverse = gradient.transform.inverse();
    if !(inverse.matrix2.to_cols_array().iter().all(|v| v.is_finite()) && inverse.translation.is_finite()) {
        return None;
    }
    match gradient.kind {
        GradientKind::Linear { start, end } => {
            let d = end - start;
            let length_squared = d.length_squared();
            if !(length_squared.is_finite() && length_squared > 0.0) {
                return None;
            }
            let w = d / length_squared;
            let a = inverse.matrix2.transpose() * w;
            let c = w.dot(inverse.translation - start);
            Some((PaintKind::Linear, [a.x, a.y, c, 0.0, 0.0, 0.0, 0.0, 0.0]))
        }
        GradientKind::Radial { center, radius } => {
            if !(radius.is_finite() && radius > 0.0) {
                return None;
            }
            let m = inverse.matrix2 * (1.0 / radius);
            let offset: Vec2 = (inverse.translation - center) / radius;
            Some((PaintKind::Radial, [m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y, offset.x, offset.y, 0.0, 0.0]))
        }
    }
}

/// What a walk of the scene found.
#[derive(Default)]
pub struct Collected<'a> {
    specs: Vec<Spec<'a>>,
    /// The passes of the frame: every layer, inner ones first, and the main pass last.
    passes: Vec<PassSpec>,
    /// What was left out, and how many nodes or paints it concerned.
    pub unsupported: BTreeMap<Unsupported, usize>,
    /// The nodes that were left out, and why (a group with opacity is one node, whatever it holds).
    pub unsupported_nodes: Vec<(NodeId, Unsupported)>,
    /// Layers that were not made because layers were nested more than [`MAX_LAYER_DEPTH`] deep, and clips that were not made
    /// because clips were nested more than [`MAX_CLIP_LEVEL`] deep.
    pub layers_flattened: usize,
    pub clips_flattened: usize,
    /// Some item is in an instance of a symbol (whether it is drawn depends on the view, shape by shape).
    pub instanced: bool,
    /// Something is not a plain shape: a group or shape with opacity below 1, a clip, a layer. In a frame that has none of these
    /// every item is the fill or stroke of a shape at full opacity (in a symbol's art too), which can be brought up to date one by
    /// one. (An instance of a symbol with full opacity is not one of them: its parts are such items.)
    pub not_flat: bool,
    /// By what factors the zoom can change before some shape drawn here wants triangles for another bucket, as `(low, high_inverse)`
    /// (see `tessellate::bucket_window`; `(0, 0)` is no limit). Only meaningful for a frame without instances, whose shapes all
    /// use the main entry of the cache.
    pub window: (f32, f32),
    /// The layers of the walk, in the order they were found (a layer comes before the layers inside it), for moving them.
    layer_slots: Vec<LayerSlot>,
}

impl<'a> Collected<'a> {
    /// The items as edits to bring them up to date one by one (see [`DrawList::patch`]), when every one of them is the fill or the
    /// stroke of a shape at full opacity with a solid color, in the main pass, and nothing was left out. `None` otherwise.
    pub fn as_edits(&self) -> Option<Vec<ItemEdit<'a>>> {
        if !self.is_flat() || !self.unsupported.is_empty() {
            return None;
        }
        self.specs
            .iter()
            .map(|spec| match (&spec.paint, spec.key.1) {
                (ItemPaint::Solid(color), Part::Fill | Part::Stroke) if spec.opacity >= 1.0 && !spec.shares_depth => {
                    Some(ItemEdit { key: spec.key, world: spec.world, color: *color, width: spec.width, mesh: spec.mesh, made_from: spec.made_from })
                }
                _ => None,
            })
            .collect()
    }
}

impl Collected<'_> {
    /// Every item is the fill or the stroke of a shape at full opacity, in the main pass, in document order (see `not_flat`).
    pub fn is_flat(&self) -> bool {
        !self.not_flat && self.passes.len() == 1
    }
}

impl Collected<'_> {
    /// Counts a node that cannot be drawn.
    fn flag(&mut self, what: Unsupported, node: NodeId) {
        *self.unsupported.entry(what).or_default() += 1;
        self.unsupported_nodes.push((node, what));
    }

    pub fn items(&self) -> usize {
        self.specs.len()
    }

    /// Where the picture of each pass is: `Some` for a layer, `None` for the main pass (the last).
    pub fn pass_rects(&self) -> Vec<Option<PixelRect>> {
        self.passes.iter().map(|p| p.rect).collect()
    }
}

struct Placed {
    vertices: Block,
    row: u32,
    /// What the uploaded vertices were made from: the geometry stamp and the scale bucket (or the frame's identity).
    made_from: (Stamp, i32),
    seen: u64,
}

/// What a chunk holds of one item: the item, where its vertices are, how many indices it adds, and what its triangles were made from.
type Signed = (Key, u32, u32, (Stamp, i32));

struct Chunk {
    /// What the chunk was built from, to tell whether it must be built again.
    signature: Vec<Signed>,
    block: Block,
    count: u32,
    /// How it is drawn (set on every update: a change of clip level does not change the indices).
    kind: DrawKind,
    level: u32,
    layer: usize,
}

/// A draw call to be: which ops of the pass it takes and how it is drawn.
struct Planned {
    kind: DrawKind,
    level: u32,
    layer: usize,
    first: usize,
    len: usize,
}

/// What one update did, for the benchmark.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DrawReport {
    pub items: usize,
    pub chunks: usize,
    pub chunks_rebuilt: usize,
    pub vertices_uploaded: usize,
    pub items_added: usize,
    pub items_removed: usize,
    /// The vertex buffer was packed (and replaced by a smaller one) in this update.
    pub vertices_compacted: bool,
    /// The index buffer was.
    pub indices_compacted: bool,
    pub triangles: usize,
    /// The row of every layer's composite, as written (for moving the layers, see [`LayerMap`]).
    pub composites: Vec<CompositeRow>,
}

/// The row of a layer's composite: where it is in the shape table and what depth and opacity it was written with.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositeRow {
    /// The item of the walk that shows the layer.
    pub spec: usize,
    pub row: u32,
    pub depth: f32,
    pub opacity: f32,
}

/// Where the layers of a frame are, kept to move them when the view only moved (see [`LayerMap::moved`]).
#[derive(Debug, Clone, Default)]
pub struct LayerMap {
    slots: Vec<MapSlot>,
}

#[derive(Debug, Clone, Copy)]
struct MapSlot {
    bounds: Bounds,
    fate: MapFate,
    parent: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum MapFate {
    Drawn { pass: usize, row: u32, depth: f32, opacity: f32 },
    OutOfView,
    Empty,
}

/// The layers of a frame after the view moved: the rectangle of every pass and the line of the shape table of every composite.
pub struct MovedLayers {
    pub rects: Vec<Option<PixelRect>>,
    /// For every composite: its row, the rectangle in the pixels of the pass it is drawn in (x, y, width, height), depth and opacity.
    pub rows: Vec<(u32, [f32; 4], f32, f32)>,
}

impl LayerMap {
    /// Remembers where the layers of `collected` are, with the rows that `report` says were written for them.
    pub fn of(collected: &Collected, report: &DrawReport) -> LayerMap {
        let rows: FastMap<usize, &CompositeRow> = report.composites.iter().map(|c| (c.spec, c)).collect();
        let slots = collected
            .layer_slots
            .iter()
            .map(|slot| MapSlot {
                bounds: slot.bounds,
                parent: slot.parent,
                fate: match slot.fate {
                    Fate::Drawn { pass, spec } => {
                        let row = rows.get(&spec).expect("a drawn layer has a composite");
                        MapFate::Drawn { pass, row: row.row, depth: row.depth, opacity: row.opacity }
                    }
                    Fate::OutOfView => MapFate::OutOfView,
                    Fate::Empty => MapFate::Empty,
                },
            })
            .collect();
        LayerMap { slots }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Where the layers are after the view moved by `shift` pixels (without being scaled or turned), in a view of `size` pixels, for a
    /// frame of `passes` passes. `None` when the frame cannot be moved as it is: a layer that was in the view is not any more (it
    /// has no picture to show), or one that was not has come into it (nothing was collected for what is inside it).
    pub fn moved(&self, shift: Vec2, size: (u32, u32), passes: usize) -> Option<MovedLayers> {
        let mut rects: Vec<Option<PixelRect>> = vec![None; passes];
        for slot in &self.slots {
            let rect = layer_rect(slot.bounds.shifted(shift), size);
            match slot.fate {
                MapFate::Drawn { pass, .. } => rects[pass] = Some(rect?),
                MapFate::OutOfView if rect.is_some() => return None,
                MapFate::OutOfView | MapFate::Empty => {}
            }
        }
        let mut rows = Vec::new();
        for slot in &self.slots {
            if let MapFate::Drawn { pass, row, depth, opacity } = slot.fate {
                let rect = rects[pass].expect("set above");
                let (parent_x, parent_y) = slot
                    .parent
                    .and_then(|p| match self.slots[p].fate {
                        MapFate::Drawn { pass, .. } => rects[pass],
                        _ => None,
                    })
                    .map_or((0, 0), |r| (r.x, r.y));
                rows.push((row, [(rect.x - parent_x) as f32, (rect.y - parent_y) as f32, rect.w as f32, rect.h as f32], depth, opacity));
            }
        }
        Some(MovedLayers { rects, rows })
    }
}

#[derive(Default)]
pub struct DrawList {
    placed: FastMap<Key, Placed>,
    chunks: Vec<Chunk>,
    /// Where each pass's chunks end in `chunks` (the passes of the last update, in order).
    pass_ends: Vec<usize>,
    generation: u64,
}

/// A walk of the scene that finds the draw items. The tessellation cache must be up to date. `view` maps scene space to pixels
/// and `size` is the size of the view in pixels (layers are sized from them); `cull` is the one the cache was updated with.
pub fn collect<'a>(
    scene: &'a Scene,
    cache: &'a TessCache,
    frame: Option<&'a FrameMesh>,
    scopes: &mut ScopeTable,
    view: Affine2,
    size: (u32, u32),
    cull: Option<Cull>,
) -> Collected<'a> {
    let mut walker = Walker::new(scene, Some(cache), scopes, view, size, cull);
    if let Some(frame) = frame {
        if let Some(mesh) = frame.mesh.fill.as_ref() {
            walker.item(Spec {
                key: (FRAME_NODE, Part::Frame, 0),
                world: Affine2::IDENTITY,
                paint: ItemPaint::Solid(premultiplied(frame.view.background)),
                width: 0.0,
                opacity: 1.0,
                shares_depth: false,
                mesh,
                made_from: (frame.identity, frame.bucket),
            });
        }
    }
    walker.walk(&scene.nodes, Affine2::IDENTITY, 1.0);
    walker.finish()
}

/// The items of one node of the scene and everything below it, seen as the whole walk would see them when it came to that node: its
/// parent's world transform is `parent`, no opacity is inherited and it is in no instance. For bringing the items of a node that moved
/// up to date without a walk of the scene (the caller makes sure of the rest: that the frame is flat, that the node is visible).
#[allow(clippy::too_many_arguments)]
pub fn collect_node<'a>(
    scene: &'a Scene,
    cache: &'a TessCache,
    scopes: &mut ScopeTable,
    view: Affine2,
    size: (u32, u32),
    cull: Option<Cull>,
    node: &'a Node,
    parent: Affine2,
) -> Collected<'a> {
    let mut walker = Walker::new(scene, Some(cache), scopes, view, size, cull);
    walker.walk(std::slice::from_ref(node), parent, 1.0);
    walker.finish()
}

/// What the GPU path cannot draw in `scene`, and how often. This is the feature gate: a view whose scene has anything in
/// it should be drawn by the CPU renderer instead, unless the picture is only wanted for timing.
pub fn unsupported(scene: &Scene) -> BTreeMap<Unsupported, usize> {
    gate(scene).unsupported
}

/// The nodes the GPU path leaves out, with the reason: what the feature gate counts, node by node.
pub fn unsupported_nodes(scene: &Scene) -> Vec<(NodeId, Unsupported)> {
    gate(scene).unsupported_nodes
}

/// A walk without a cache: it finds what cannot be drawn and makes no items.
fn gate(scene: &Scene) -> Collected<'_> {
    let mut scopes = ScopeTable::default();
    let mut walker = Walker::new(scene, None, &mut scopes, Affine2::IDENTITY, (1, 1), None);
    walker.walk(&scene.nodes, Affine2::IDENTITY, 1.0);
    walker.finish()
}

/// Layers inside layers: deeper ones are not made (GPU_SAFETY.md B2): the opacity goes down into what is inside, which is not exact
/// where it overlaps, and the frame says how many times that happened.
pub const MAX_LAYER_DEPTH: usize = 32;
/// Clips inside clips: the stencil has 8 bits and saturates at 255. Deeper clips are not made (the content is drawn with the
/// clips it is already in) and the frame says so.
pub const MAX_CLIP_LEVEL: u32 = 250;
/// More children than this are not tested for touching each other: the group gets a layer.
const MAX_APART_CHECK: usize = 4000;
/// Pixels around the box of a shape when asking whether two shapes can touch: the soft edge of one anti-aliased pixel, and a
/// little more.
const TOUCH_SLACK_PX: f32 = 1.5;

/// A pass that is being built.
struct Building {
    ops: Vec<Op>,
    rect: Option<PixelRect>,
    /// Clips that are open in this pass (every pass has its own stencil).
    clip_level: u32,
    /// The layer this pass is for (an index into `Collected::layer_slots`); `None` for the main pass.
    slot: Option<usize>,
}

/// What a node can paint, as a box of pixels.
#[derive(Clone, Copy, Debug)]
pub enum Bounds {
    /// Nothing at all (it is hidden, empty, or out of the view).
    Nothing,
    Box(Vec2, Vec2),
    /// It cannot be judged: it is taken to be everywhere.
    Unknown,
}

/// What became of a layer in a walk.
#[derive(Clone, Copy, Debug)]
enum Fate {
    /// It was drawn: this pass holds its picture and this item shows it.
    Drawn { pass: usize, spec: usize },
    /// Its box did not touch the view, so nothing of what is inside was collected.
    OutOfView,
    /// Nothing inside it draws anything.
    Empty,
}

/// One layer of a walk: where its box was, and what became of it.
#[derive(Clone, Copy, Debug)]
struct LayerSlot {
    bounds: Bounds,
    fate: Fate,
    /// The layer it is drawn in, `None` for the main pass.
    parent: Option<usize>,
}

/// The pixels a layer with this box takes in a view of `size` pixels: its box, clipped to the view (all of it when it cannot be judged).
fn layer_rect(bounds: Bounds, size: (u32, u32)) -> Option<PixelRect> {
    match bounds {
        Bounds::Nothing => None,
        Bounds::Box(lo, hi) => PixelRect::of_box(lo, hi, size),
        Bounds::Unknown => PixelRect::of_box(Vec2::ZERO, Vec2::new(size.0 as f32, size.1 as f32), size),
    }
}

impl Bounds {
    /// The same box after everything on the screen moved by `by` pixels.
    fn shifted(self, by: Vec2) -> Bounds {
        match self {
            Bounds::Box(lo, hi) => Bounds::Box(lo + by, hi + by),
            other => other,
        }
    }

    fn union(self, other: Bounds) -> Bounds {
        match (self, other) {
            (Bounds::Unknown, _) | (_, Bounds::Unknown) => Bounds::Unknown,
            (Bounds::Nothing, b) | (b, Bounds::Nothing) => b,
            (Bounds::Box(a0, a1), Bounds::Box(b0, b1)) => Bounds::Box(a0.min(b0), a1.max(b1)),
        }
    }

    fn intersect(self, other: Bounds) -> Bounds {
        match (self, other) {
            (Bounds::Nothing, _) | (_, Bounds::Nothing) => Bounds::Nothing,
            (Bounds::Unknown, b) | (b, Bounds::Unknown) => b,
            (Bounds::Box(a0, a1), Bounds::Box(b0, b1)) => {
                let (lo, hi) = (a0.max(b0), a1.min(b1));
                if lo.x <= hi.x && lo.y <= hi.y { Bounds::Box(lo, hi) } else { Bounds::Nothing }
            }
        }
    }
}

/// Whether none of the boxes touches another (they are sorted here).
fn apart(boxes: &mut [(Vec2, Vec2)]) -> bool {
    if boxes.len() > MAX_APART_CHECK {
        return false;
    }
    boxes.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    for i in 0..boxes.len() {
        for j in i + 1..boxes.len() {
            if boxes[j].0.x > boxes[i].1.x {
                break;
            }
            if boxes[j].0.y <= boxes[i].1.y && boxes[i].0.y <= boxes[j].1.y {
                return false;
            }
        }
    }
    true
}

/// The paints of a shape as items, with the color overrides of the instances it is in: the fill, and the stroke with its width
/// (nothing for a part that is not painted). An override replaces every fill (gradients too) or stroke paint by a solid color;
/// a shape without a fill or a stroke stays without.
fn paints<'a>(shape: &'a Shape, overrides: &Overrides) -> (Option<ItemPaint<'a>>, Option<(ItemPaint<'a>, f32)>) {
    let fill = match (&shape.fill, overrides.fill) {
        (Some(_), Some(color)) => Some(ItemPaint::Solid(premultiplied(color))),
        (Some(paint), None) => item_paint(paint),
        (None, _) => None,
    };
    let stroke = shape.stroke.as_ref().filter(|s| s.width > 0.0 && s.width.is_finite()).and_then(|s| {
        let paint = match overrides.stroke {
            Some(color) => Some(ItemPaint::Solid(premultiplied(color))),
            None => item_paint(&s.paint),
        };
        paint.map(|paint| (paint, s.width))
    });
    (fill, stroke)
}

/// Where the walk is: inside which instance, with which color overrides.
#[derive(Clone, Copy)]
struct Context {
    scope: Scope,
    overrides: Overrides,
    /// Instances inside instances.
    depth: usize,
}

/// The walk of the scene: finds the items, masks and layers, in document order.
struct Walker<'a, 's> {
    scene: &'a Scene,
    out: Collected<'a>,
    /// With a cache, a shape or mask without triangles in it is left out (it was culled, or has nothing to draw). Without,
    /// nothing is made: the walk only counts what cannot be drawn.
    cache: Option<&'a TessCache>,
    scopes: &'s mut ScopeTable,
    context: Context,
    /// Scene space to pixels, the camera's zoom, the size of the view in pixels, and the cull the cache was made with.
    view: Affine2,
    zoom: f32,
    size: (u32, u32),
    cull: Option<Cull>,
    /// The passes being built: the main pass at the bottom, the layer that is being walked at the top (it gets the ops).
    stack: Vec<Building>,
}

impl<'a, 's> Walker<'a, 's> {
    fn new(scene: &'a Scene, cache: Option<&'a TessCache>, scopes: &'s mut ScopeTable, view: Affine2, size: (u32, u32), cull: Option<Cull>) -> Self {
        Walker {
            scene,
            out: Collected::default(),
            cache,
            scopes,
            context: Context { scope: 0, overrides: Overrides::default(), depth: 0 },
            view,
            zoom: largest_scale(&view),
            size,
            cull,
            stack: vec![Building { ops: Vec::new(), rect: None, clip_level: 0, slot: None }],
        }
    }

    fn finish(mut self) -> Collected<'a> {
        let main = self.stack.pop().expect("the main pass");
        self.out.passes.push(PassSpec { ops: main.ops, rect: None });
        self.out
    }

    fn ops(&mut self) -> &mut Vec<Op> {
        &mut self.stack.last_mut().expect("a pass").ops
    }

    /// Adds an item to the current pass.
    fn item(&mut self, spec: Spec<'a>) {
        let index = self.out.specs.len();
        self.out.specs.push(spec);
        self.ops().push(Op::Item(index));
    }

    /// The triangles that the cache holds for a shape (or mask) seen at `world`, if it is to be drawn. Inside an instance a node is
    /// made for whichever instance was first to reach it, so whether this instance's copy can reach the view is asked again.
    fn triangles(&self, node: &Node, shape: &Shape, world: &Affine2, mask: bool) -> Option<&'a CachedShape> {
        let cache = self.cache?;
        let entry = cache.find(node.id, mask, self.zoom * largest_scale(world))?;
        if self.context.scope != 0 {
            if let Some(cull) = &self.cull {
                if !cull.may_touch(cache.extent(node.id), world, if mask { None } else { shape.stroke.as_ref() }) {
                    return None;
                }
            }
        }
        Some(entry)
    }

    /// Walks `nodes`. `inherited` is the opacity that parents with opacity have passed down to them instead of drawing
    /// themselves on a layer (1 for most).
    fn walk(&mut self, nodes: &'a [Node], parent: Affine2, inherited: f32) {
        for node in nodes {
            if !node.visible || node.opacity <= 0.0 {
                continue;
            }
            let opacity = inherited * node.opacity;
            let world = parent * node.transform;
            match &node.kind {
                NodeKind::Group(children) => match node.clip_parts() {
                    Some((content, mask_node, mask_shape)) => self.clip(node, content, mask_node, mask_shape, world, opacity),
                    None if opacity < 1.0 => self.faded(node, children, world, opacity),
                    None => self.walk(children, world, 1.0),
                },
                // (A skinned shape inside an instance would need a pose for each instance; the cache makes one outline per node.)
                NodeKind::Shape(shape) if shape.skin.is_some() && self.context.scope != 0 => self.out.flag(Unsupported::Skinned, node.id),
                NodeKind::Shape(shape) => self.shape(node, shape, world, opacity),
                NodeKind::Instance(instance) => self.instance(node, instance, world, opacity),
                NodeKind::Camera(_) => {}
            }
        }
    }

    /// An instance of a symbol: the master's root drawn under the instance's transform, with the instance's color overrides
    /// (outer instances win over inner ones). The instance's own opacity makes it a whole, like a group's.
    fn instance(&mut self, node: &'a Node, instance: &'a Instance, world: Affine2, opacity: f32) {
        let scene = self.scene;
        let outer = self.context;
        let Some(symbol) = scene.symbol(instance.symbol).filter(|_| outer.depth < MAX_INSTANCE_DEPTH) else { return };
        let art = std::slice::from_ref(&symbol.root);
        self.out.instanced = true;
        self.context = Context { scope: self.scopes.scope(outer.scope, node.id), overrides: outer.overrides.inside(instance), depth: outer.depth + 1 };
        if opacity < 1.0 {
            self.faded(node, art, world, opacity);
        } else {
            self.walk(art, world, 1.0);
        }
        self.context = outer;
    }

    /// A group that is not fully opaque. The CPU renderer draws it on a layer and composites that with the opacity. That is
    /// the same as multiplying the opacity into the children when no pixel gets paint from two of them: with one child, or
    /// with children whose boxes are apart. Otherwise it is drawn on a layer here too.
    fn faded(&mut self, node: &'a Node, children: &'a [Node], world: Affine2, opacity: f32) {
        self.out.not_flat = true;
        if self.cache.is_none() {
            return self.walk(children, world, 1.0); // the gate: only what is inside matters
        }
        let mut boxes = Vec::new();
        let mut known = true;
        for child in children {
            match self.bounds(child, world, TOUCH_SLACK_PX, self.context.depth) {
                Bounds::Nothing => {}
                Bounds::Box(lo, hi) => boxes.push((lo, hi)),
                Bounds::Unknown => known = false,
            }
        }
        if known && boxes.is_empty() {
            return; // nothing in it draws anything
        }
        if known && (boxes.len() == 1 || apart(&mut boxes)) {
            return self.walk(children, world, opacity);
        }
        let bounds = self.union_of(children, world, SLACK_PX, self.context.depth);
        self.layer(node.id, bounds, opacity, |walker, inherited| walker.walk(children, world, inherited));
    }

    /// A clip group: the content is drawn through the silhouette of the mask (the top child), which goes into the stencil
    /// before the content and comes out after it. With opacity below 1 the same two choices as for a group: the opacity goes
    /// into the content, or the clipped content is drawn on a layer.
    fn clip(&mut self, node: &'a Node, content: &'a [Node], mask_node: &'a Node, mask_shape: &'a Shape, world: Affine2, opacity: f32) {
        if mask_shape.skin.is_some() {
            self.out.flag(Unsupported::Skinned, mask_node.id);
            return;
        }
        self.out.not_flat = true;
        if self.cache.is_none() {
            return self.walk(content, world, 1.0); // the gate: only what is inside matters
        }
        // A mask with no triangles (it cannot reach the view, or is an empty path) shows nothing of the content.
        let mask_world = world * mask_node.transform;
        let Some(mask_mesh) = self.triangles(mask_node, mask_shape, &mask_world, true).filter(|e| e.mask).and_then(|e| e.mesh.fill.as_ref().map(|mesh| (mesh, e.identity()))) else {
            return;
        };
        if opacity >= 1.0 {
            return self.clipped(content, mask_node, mask_mesh, world, 1.0);
        }
        let mut boxes = Vec::new();
        let mut known = true;
        for child in content {
            match self.bounds(child, world, TOUCH_SLACK_PX, self.context.depth) {
                Bounds::Nothing => {}
                Bounds::Box(lo, hi) => boxes.push((lo, hi)),
                Bounds::Unknown => known = false,
            }
        }
        if known && boxes.is_empty() {
            return;
        }
        if known && (boxes.len() == 1 || apart(&mut boxes)) {
            return self.clipped(content, mask_node, mask_mesh, world, opacity);
        }
        let depth = self.context.depth;
        let bounds = self.union_of(content, world, SLACK_PX, depth).intersect(self.mask_bounds(mask_node, mask_shape, mask_world, SLACK_PX));
        self.layer(node.id, bounds, opacity, |walker, inherited| walker.clipped(content, mask_node, mask_mesh, world, inherited));
    }

    /// The content between a push and a pop of the mask.
    fn clipped(&mut self, content: &'a [Node], mask_node: &'a Node, mask: (&'a Mesh, (Stamp, i32)), world: Affine2, inherited: f32) {
        if self.stack.last().is_some_and(|pass| pass.clip_level >= MAX_CLIP_LEVEL) {
            self.out.clips_flattened += 1;
            return self.walk(content, world, inherited);
        }
        let index = self.out.specs.len();
        self.out.specs.push(Spec {
            key: (mask_node.id, Part::Mask, self.context.scope),
            world: world * mask_node.transform,
            paint: ItemPaint::Solid([0.0; 4]),
            width: 0.0,
            opacity: 1.0,
            shares_depth: false,
            mesh: mask.0,
            made_from: mask.1,
        });
        let before = self.ops().len();
        self.ops().push(Op::Push(index));
        self.stack.last_mut().expect("a pass").clip_level += 1;
        self.walk(content, world, inherited);
        self.stack.last_mut().expect("a pass").clip_level -= 1;
        if self.ops().len() == before + 1 {
            // Nothing inside draws anything: no need to clip it.
            self.ops().pop();
            self.out.specs.pop();
        } else {
            self.ops().push(Op::Pop(index));
        }
    }

    /// A shape with the opacity `opacity` (its own, times what was passed down to it).
    fn shape(&mut self, node: &'a Node, shape: &'a Shape, world: Affine2, opacity: f32) {
        if opacity < 1.0 {
            self.out.not_flat = true;
        }
        let overrides = self.context.overrides;
        let (fill, stroke) = paints(shape, &overrides);
        // With opacity, a fill and a stroke together are only exact when both are opaque; otherwise the shape is faded as a
        // whole, on a layer.
        let layered = opacity < 1.0 && matches!((&fill, &stroke), (Some(f), Some((s, _))) if !(f.is_opaque() && s.is_opaque()));
        if layered {
            if self.cache.is_none() {
                return;
            }
            let bounds = self.shape_bounds(node, shape, world, SLACK_PX);
            return self.layer(node.id, bounds, opacity, |walker, inherited| {
                let (fill, stroke) = paints(shape, &overrides);
                walker.shape_items(node, shape, world, inherited, fill, stroke);
            });
        }
        self.shape_items(node, shape, world, opacity, fill, stroke);
    }

    fn shape_items(&mut self, node: &'a Node, shape: &'a Shape, world: Affine2, opacity: f32, fill: Option<ItemPaint<'a>>, stroke: Option<(ItemPaint<'a>, f32)>) {
        let Some(entry) = self.triangles(node, shape, &world, false) else { return };
        let (partial, scope) = (opacity < 1.0, self.context.scope);
        if entry.depends_on_scale {
            let (low, high_inverse) = entry.scale_window(self.zoom * largest_scale(&world));
            self.out.window = (self.out.window.0.max(low), self.out.window.1.max(high_inverse));
        }
        let both = fill.is_some() && stroke.is_some();
        let made_from = entry.identity();
        let fill_spec = fill.zip(entry.mesh.fill.as_ref()).map(|(paint, mesh)| Spec { key: (node.id, Part::Fill, scope), world, paint, width: 0.0, opacity, shares_depth: false, mesh, made_from });
        let stroke_spec = stroke
            .zip(entry.mesh.stroke.as_ref())
            .map(|((paint, width), mesh)| Spec { key: (node.id, Part::Stroke, scope), world, paint, width, opacity, shares_depth: false, mesh, made_from });
        match (fill_spec, stroke_spec) {
            // A shape with opacity and both parts: the stroke first, the fill at the same depth.
            (Some(fill), Some(stroke)) if partial && both => {
                self.item(stroke);
                self.item(Spec { shares_depth: true, ..fill });
            }
            (fill, stroke) => {
                if let Some(fill) = fill {
                    self.item(fill);
                }
                if let Some(stroke) = stroke {
                    self.item(stroke);
                }
            }
        }
    }

    /// Draws what `build` adds on a layer of its own and composites that, with `opacity`, where it belongs: a pass for the
    /// layer (sized to `bounds`, clipped to the view), then a composite in the pass that is being built.
    fn layer(&mut self, id: NodeId, bounds: Bounds, opacity: f32, build: impl FnOnce(&mut Self, f32)) {
        // Too many layers inside each other: no new one. What is inside is drawn with the opacity passed down to it.
        if self.stack.len() > MAX_LAYER_DEPTH {
            self.out.layers_flattened += 1;
            return build(self, opacity);
        }
        if matches!(bounds, Bounds::Nothing) {
            return;
        }
        let slot = self.out.layer_slots.len();
        let parent = self.stack.last().and_then(|b| b.slot);
        self.out.layer_slots.push(LayerSlot { bounds, fate: Fate::OutOfView, parent });
        let Some(rect) = layer_rect(bounds, self.size) else { return };
        self.stack.push(Building { ops: Vec::new(), rect: Some(rect), clip_level: 0, slot: Some(slot) });
        build(self, 1.0);
        let built = self.stack.pop().expect("the layer's pass");
        if built.ops.is_empty() {
            self.out.layer_slots[slot].fate = Fate::Empty;
            return;
        }
        let (parent_x, parent_y) = self.stack.last().and_then(|b| b.rect).map_or((0, 0), |r| (r.x, r.y));
        self.out.passes.push(PassSpec { ops: built.ops, rect: Some(rect) });
        let layer = self.out.passes.len() - 1;
        let spec = self.out.specs.len();
        self.out.layer_slots[slot].fate = Fate::Drawn { pass: layer, spec };
        let place = [(rect.x - parent_x) as f32, (rect.y - parent_y) as f32, rect.w as f32, rect.h as f32];
        self.out.specs.push(Spec {
            key: (id, Part::Layer, self.context.scope),
            world: Affine2::IDENTITY,
            paint: ItemPaint::Layer(place),
            width: 0.0,
            opacity,
            shares_depth: false,
            mesh: &QUAD,
            made_from: (*QUAD_STAMP, 0),
        });
        self.ops().push(Op::Composite { spec, layer });
    }

    // --- Boxes on the screen -------------------------------------------------------------------------------------------

    /// What `node` (with its place in `parent`) can paint, as a box in pixels, `slack` pixels wider than the geometry. `depth`
    /// counts the instances that are open (the walk stops at the same depth as the others).
    fn bounds(&self, node: &Node, parent: Affine2, slack: f32, depth: usize) -> Bounds {
        if !node.visible || node.opacity <= 0.0 {
            return Bounds::Nothing;
        }
        let world = parent * node.transform;
        match &node.kind {
            NodeKind::Group(children) => match node.clip_parts() {
                Some((content, mask_node, mask_shape)) => {
                    let mask = self.mask_bounds(mask_node, mask_shape, world * mask_node.transform, slack);
                    if matches!(mask, Bounds::Nothing) {
                        return Bounds::Nothing;
                    }
                    self.union_of(content, world, slack, depth).intersect(mask)
                }
                None => self.union_of(children, world, slack, depth),
            },
            NodeKind::Shape(shape) => self.shape_bounds(node, shape, world, slack),
            NodeKind::Instance(instance) => match self.scene.symbol(instance.symbol) {
                Some(symbol) if depth < MAX_INSTANCE_DEPTH => self.bounds(&symbol.root, world, slack, depth + 1),
                _ => Bounds::Nothing,
            },
            NodeKind::Camera(_) => Bounds::Nothing,
        }
    }

    fn union_of(&self, nodes: &[Node], parent: Affine2, slack: f32, depth: usize) -> Bounds {
        nodes.iter().fold(Bounds::Nothing, |all, node| all.union(self.bounds(node, parent, slack, depth)))
    }

    /// The box of what a shape paints (the parts that have triangles in the cache).
    fn shape_bounds(&self, node: &Node, shape: &Shape, world: Affine2, slack: f32) -> Bounds {
        let Some(cache) = self.cache else { return Bounds::Unknown };
        let Some(entry) = cache.get(node.id) else { return Bounds::Nothing };
        let fill = shape.fill.is_some() && entry.mesh.fill.is_some();
        let stroke = shape.stroke.as_ref().filter(|s| s.width > 0.0 && s.width.is_finite() && entry.mesh.stroke.is_some());
        if !fill && stroke.is_none() {
            return Bounds::Nothing;
        }
        // A skinned shape is where the bones bend it, not where its bind pose is.
        let Some(extent) = control_extent(entry.posed_path().unwrap_or(&shape.path)) else { return Bounds::Nothing };
        match pixel_box(extent, &(self.view * world), stroke, slack) {
            Some((lo, hi)) => Bounds::Box(lo, hi),
            None => Bounds::Unknown,
        }
    }

    /// The box of a clip mask's silhouette; nothing when the mask has no triangles.
    fn mask_bounds(&self, mask_node: &Node, mask_shape: &Shape, mask_world: Affine2, slack: f32) -> Bounds {
        let Some(cache) = self.cache else { return Bounds::Unknown };
        if !cache.get(mask_node.id).is_some_and(|e| e.mask && e.mesh.fill.is_some()) {
            return Bounds::Nothing;
        }
        let Some(extent) = control_extent(&mask_shape.path) else { return Bounds::Nothing };
        match pixel_box(extent, &(self.view * mask_world), None, slack) {
            Some((lo, hi)) => Bounds::Box(lo, hi),
            None => Bounds::Unknown,
        }
    }
}

impl DrawList {
    pub fn new() -> DrawList {
        DrawList::default()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.iter().filter(|c| c.count > 0).count()
    }

    /// How many passes the last update planned (the last one is the main pass).
    pub fn pass_count(&self) -> usize {
        self.pass_ends.len()
    }

    /// The draw calls of one pass, in order.
    pub fn draws(&self, pass: usize) -> impl Iterator<Item = Draw> + '_ {
        let first = if pass == 0 { 0 } else { self.pass_ends[pass - 1] };
        self.chunks[first..self.pass_ends[pass]].iter().filter(|c| c.count > 0).map(|c| Draw { kind: c.kind, level: c.level, layer: c.layer, start: c.block.start, count: c.count })
    }

    pub fn items(&self) -> usize {
        self.placed.len()
    }

    /// Brings rows, vertices and chunks up to date with what was collected.
    ///
    /// What is gone or out of date is given back **first**, and only then is anything new asked for: the freed ranges are
    /// reused, and a buffer that is mostly free is packed (or replaced) before it would have to grow next to the old one.
    /// Doing it the other way round made a change of zoom need the old and the new geometry at once, which went over the
    /// geometry budget. If a request fails half way, every item is either complete or marked to be made again, so a later
    /// update (at another zoom, say) starts from a state that is true.
    pub fn update(
        &mut self,
        collected: &Collected,
        divisors: &[u32],
        table: &mut ShapeTable,
        ramps: &mut RampAtlas,
        store: &mut GeometryStore,
    ) -> Result<DrawReport, AllocError> {
        let mut report = DrawReport { items: collected.specs.len(), ..Default::default() };
        self.generation += 1;
        let generation = self.generation;

        // What each wanted item must be made from.
        let made_from: Vec<(Stamp, i32)> = collected.specs.iter().map(|spec| spec.made_from).collect();

        // 1. Give back the vertices and rows of what is gone, and the vertices of what is out of date.
        for (spec, wanted) in collected.specs.iter().zip(&made_from) {
            if let Some(placed) = self.placed.get_mut(&spec.key) {
                placed.seen = generation;
                if placed.made_from != *wanted {
                    give_back(&mut store.vertices, &mut placed.vertices);
                    placed.made_from = (Stamp::fresh(), i32::MIN); // nothing is made from this: it is made again below
                }
            }
        }
        let mut pending_vertices = 0u64;
        for (spec, wanted) in collected.specs.iter().zip(&made_from) {
            if self.placed.get(&spec.key).is_none_or(|p| p.made_from != *wanted) {
                pending_vertices += class_size(spec.mesh.vertices.len() as u32) as u64;
            }
        }
        let gone: Vec<Key> = self.placed.iter().filter(|(_, p)| p.seen != generation).map(|(k, _)| *k).collect();
        report.items_removed = gone.len();
        for key in gone {
            let mut placed = self.placed.remove(&key).expect("just listed");
            give_back(&mut store.vertices, &mut placed.vertices);
            table.free_row(placed.row);
        }

        // 2. Close the gaps before asking for room (the buffer ends up as large as what is live plus what is coming).
        if store.vertices.ranges().needs_compaction() {
            let live: Vec<(Key, Block)> = self.placed.values().zip(self.placed.keys()).filter(|(p, _)| p.vertices.reserved() > 0).map(|(p, k)| (*k, p.vertices)).collect();
            let live_size: u64 = live.iter().map(|(_, b)| b.reserved() as u64).sum();
            let min_capacity = (live_size + pending_vertices).min(u32::MAX as u64).max(1) as u32;
            for (key, block) in store.vertices.compact(live, min_capacity)? {
                self.placed.get_mut(&key).expect("compacted a live item").vertices = block;
            }
            report.vertices_compacted = true;
        }

        // 3. Vertices of the new and the out-of-date items (the room for all of them is made in one step).
        store.vertices.reserve(pending_vertices)?;
        for (spec, wanted) in collected.specs.iter().zip(&made_from) {
            let mesh = spec.mesh;
            match self.placed.get_mut(&spec.key) {
                Some(placed) if placed.made_from == *wanted => {}
                Some(placed) => {
                    placed.vertices = upload(store, mesh, placed.row)?;
                    placed.made_from = *wanted;
                    report.vertices_uploaded += mesh.vertices.len();
                }
                None => {
                    let row = table.alloc_row()?;
                    let vertices = match upload(store, mesh, row) {
                        Ok(vertices) => vertices,
                        Err(error) => {
                            table.free_row(row);
                            return Err(error);
                        }
                    };
                    self.placed.insert(spec.key, Placed { vertices, row, made_from: *wanted, seen: generation });
                    report.vertices_uploaded += mesh.vertices.len();
                    report.items_added += 1;
                }
            }
        }
        // The rows of the table: color, transform and depth of every item, in painter's order. Every pass counts its own depth
        // (it has its own depth buffer); a mask has a row for its transform only.
        for (number, pass) in collected.passes.iter().enumerate() {
            let mut level = 0u32;
            for op in &pass.ops {
                let (spec, layer) = match *op {
                    Op::Item(i) => {
                        if !collected.specs[i].shares_depth {
                            level += 1;
                        }
                        (&collected.specs[i], None)
                    }
                    Op::Composite { spec: i, layer } => {
                        level += 1;
                        report.composites.push(CompositeRow { spec: i, row: self.placed[&collected.specs[i].key].row, depth: level as f32 * DEPTH_STEP, opacity: collected.specs[i].opacity });
                        (&collected.specs[i], Some(layer))
                    }
                    Op::Push(i) => {
                        table.set(self.placed[&collected.specs[i].key].row, RowData::solid(&collected.specs[i].world, 0.0, [0.0; 4], 0.0, 1.0));
                        continue;
                    }
                    Op::Pop(_) => continue,
                };
                let depth = level as f32 * DEPTH_STEP;
                let row = self.placed[&spec.key].row;
                let data = match &spec.paint {
                    ItemPaint::Solid(color) => RowData::solid(&spec.world, depth, *color, spec.width, spec.opacity),
                    ItemPaint::Gradient { kind, geometry, stops } => match ramps.ensure(stops)? {
                        Some(ramp) => RowData::gradient(&spec.world, depth, *kind, *geometry, ramp, spec.width, spec.opacity),
                        None => RowData::solid(&spec.world, depth, [0.0; 4], spec.width, spec.opacity), // no colors to draw with
                    },
                    // The rectangle is in the full-size pixels of the pass; a pass that is drawn smaller has fewer of them, and a layer
                    // that was drawn smaller than its place has fewer texels for each.
                    ItemPaint::Layer(rect) => {
                        let (own, shown) = (divisors[number] as f32, divisors[layer.expect("a composite has a layer")] as f32);
                        RowData::layer([rect[0] / own, rect[1] / own, rect[2] / own, rect[3] / own], depth, spec.opacity, own / shown)
                    }
                };
                table.set(row, data);
            }
        }

        // 4. Chunks. First what every chunk should hold, then give back those that differ, then make them again. A chunk is a
        // run of up to CHUNK_ITEMS items at one clip level, or one mask going in or out.
        let mut order: Vec<usize> = Vec::with_capacity(collected.specs.len() + 8);
        let mut planned: Vec<Planned> = Vec::with_capacity(collected.specs.len() / CHUNK_ITEMS + 8);
        let mut pass_ends: Vec<usize> = Vec::with_capacity(collected.passes.len());
        for pass in &collected.passes {
            let mut level = 0u32;
            let mut run: Option<Planned> = None;
            for op in &pass.ops {
                match *op {
                    Op::Item(i) => {
                        match &mut run {
                            Some(r) if r.level == level && r.len < CHUNK_ITEMS => r.len += 1,
                            _ => {
                                planned.extend(run.take());
                                run = Some(Planned { kind: DrawKind::Items, level, layer: 0, first: order.len(), len: 1 });
                            }
                        }
                        order.push(i);
                    }
                    Op::Push(i) => {
                        planned.extend(run.take());
                        planned.push(Planned { kind: DrawKind::MaskPush, level, layer: 0, first: order.len(), len: 1 });
                        order.push(i);
                        level += 1;
                    }
                    Op::Pop(i) => {
                        planned.extend(run.take());
                        planned.push(Planned { kind: DrawKind::MaskPop, level, layer: 0, first: order.len(), len: 1 });
                        order.push(i);
                        level = level.saturating_sub(1);
                    }
                    Op::Composite { spec, layer } => {
                        planned.extend(run.take());
                        planned.push(Planned { kind: DrawKind::Composite, level, layer, first: order.len(), len: 1 });
                        order.push(spec);
                    }
                }
            }
            planned.extend(run.take());
            pass_ends.push(planned.len());
        }
        self.pass_ends = pass_ends;
        let wanted = planned.len();
        while self.chunks.len() > wanted {
            let mut chunk = self.chunks.pop().expect("longer than wanted");
            give_back(&mut store.indices, &mut chunk.block);
        }
        let signatures: Vec<Vec<Signed>> = planned
            .iter()
            .map(|chunk| {
                order[chunk.first..chunk.first + chunk.len]
                    .iter()
                    .map(|&i| {
                        let spec = &collected.specs[i];
                        (spec.key, self.placed[&spec.key].vertices.start, spec.mesh.indices.len() as u32, spec.made_from)
                    })
                    .collect()
            })
            .collect();
        let mut pending_indices = 0u64;
        let mut rebuild = Vec::new();
        for (number, signature) in signatures.iter().enumerate() {
            if let Some(chunk) = self.chunks.get_mut(number) {
                // How it is drawn can change without its indices changing (a clip level).
                chunk.kind = planned[number].kind;
                chunk.level = planned[number].level;
                chunk.layer = planned[number].layer;
            }
            if self.chunks.get(number).is_some_and(|c| c.signature == *signature) {
                continue;
            }
            if let Some(old) = self.chunks.get_mut(number) {
                give_back(&mut store.indices, &mut old.block);
                old.signature = Vec::new();
                old.count = 0;
            }
            pending_indices += class_size(signature.iter().map(|s| s.2).sum()) as u64;
            rebuild.push(number);
        }
        if store.indices.ranges().needs_compaction() {
            let live: Vec<(usize, Block)> = self.chunks.iter().enumerate().filter(|(_, c)| c.block.reserved() > 0).map(|(i, c)| (i, c.block)).collect();
            let live_size: u64 = live.iter().map(|(_, b)| b.reserved() as u64).sum();
            let min_capacity = (live_size + pending_indices).min(u32::MAX as u64).max(1) as u32;
            for (number, block) in store.indices.compact(live, min_capacity)? {
                self.chunks[number].block = block;
            }
            report.indices_compacted = true;
        }
        store.indices.reserve(pending_indices)?;
        for number in rebuild {
            let signature = &signatures[number];
            let mut indices: Vec<u32> = Vec::with_capacity(signature.iter().map(|s| s.2 as usize).sum());
            for (&i, &(_, start, _, _)) in order[planned[number].first..planned[number].first + planned[number].len].iter().zip(signature) {
                indices.extend(collected.specs[i].mesh.indices.iter().map(|index| index + start));
            }
            let block = store.insert_indices(&indices)?;
            let chunk = Chunk { signature: signature.clone(), block, count: indices.len() as u32, kind: planned[number].kind, level: planned[number].level, layer: planned[number].layer };
            if number < self.chunks.len() {
                self.chunks[number] = chunk;
            } else {
                self.chunks.push(chunk);
            }
            report.chunks_rebuilt += 1;
        }
        store.flush();
        report.chunks = self.chunk_count();
        report.triangles = self.chunks.iter().map(|c| c.count as usize / 3).sum();
        Ok(report)
    }
}

/// One item to bring up to date by itself (see [`DrawList::patch`]): a fill or a stroke of a shape at full opacity with a solid color.
pub struct ItemEdit<'a> {
    pub key: Key,
    pub world: Affine2,
    /// Premultiplied.
    pub color: [f32; 4],
    /// The stroke's width (0 for a fill).
    pub width: f32,
    pub mesh: &'a Mesh,
    pub made_from: (Stamp, i32),
}

/// What a [`DrawList::patch`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PatchReport {
    pub items: usize,
    pub vertices_uploaded: usize,
    /// Chunks whose index list was changed in place (the item's part of it) and chunks that were made again (its size changed).
    pub chunks_patched: usize,
    pub chunks_rebuilt: usize,
}

impl DrawList {
    /// Whether `key` is an item of the list.
    pub fn contains(&self, key: &Key) -> bool {
        self.placed.contains_key(key)
    }

    /// Brings some items up to date without a pass over the scene: their rows are written (the depth stays what it was, so the
    /// order of drawing does not change), and where an item's triangles were made again its vertices are replaced and the part
    /// of its chunk's index list that is its own is written again, or the chunk made again if the item now has more or fewer
    /// indices. Every key must be in the list (see [`DrawList::contains`]). `mesh_of` gives the triangles of any item of the list: asked with its key and what it was made from.
    pub fn patch<'c>(
        &mut self,
        edits: &[ItemEdit],
        table: &mut ShapeTable,
        store: &mut GeometryStore,
        mesh_of: impl Fn(Key, (Stamp, i32)) -> Option<&'c Mesh>,
    ) -> Result<PatchReport, AllocError> {
        let mut report = PatchReport::default();
        let mut moved: Vec<usize> = Vec::new();
        for (number, edit) in edits.iter().enumerate() {
            let placed = self.placed.get_mut(&edit.key).expect("the caller checked that the item is in the list");
            let depth = table.get(placed.row).depth();
            table.set(placed.row, RowData::solid(&edit.world, depth, edit.color, edit.width, 1.0));
            report.items += 1;
            if placed.made_from != edit.made_from {
                give_back(&mut store.vertices, &mut placed.vertices);
                placed.made_from = (Stamp::fresh(), i32::MIN); // nothing is made from this until the upload has worked
                placed.vertices = upload(store, edit.mesh, placed.row)?;
                placed.made_from = edit.made_from;
                report.vertices_uploaded += edit.mesh.vertices.len();
                moved.push(number);
            }
        }
        if !moved.is_empty() {
            self.patch_chunks(edits, &moved, store, &mesh_of, &mut report)?;
        }
        store.flush();
        Ok(report)
    }

    /// The index lists of the chunks that hold items whose vertices were replaced.
    fn patch_chunks<'c>(
        &mut self,
        edits: &[ItemEdit],
        moved: &[usize],
        store: &mut GeometryStore,
        mesh_of: &impl Fn(Key, (Stamp, i32)) -> Option<&'c Mesh>,
        report: &mut PatchReport,
    ) -> Result<(), AllocError> {
        let edit_of: FastMap<Key, usize> = moved.iter().map(|&number| (edits[number].key, number)).collect();
        // Which chunk, and where in it, each of them is: one look at the signatures.
        let mut by_chunk: FastMap<usize, Vec<usize>> = FastMap::default();
        for (number, chunk) in self.chunks.iter().enumerate() {
            for (at, signed) in chunk.signature.iter().enumerate() {
                if edit_of.contains_key(&signed.0) {
                    by_chunk.entry(number).or_default().push(at);
                }
            }
        }
        for (number, positions) in by_chunk {
            // What each of the changed items is now: where its vertices are, how many indices it has, what it was made from.
            let mut now = Vec::with_capacity(positions.len());
            let mut same_size = true;
            for &at in &positions {
                let signed = self.chunks[number].signature[at];
                let edit = &edits[edit_of[&signed.0]];
                let start = self.placed[&signed.0].vertices.start;
                let count = edit.mesh.indices.len() as u32;
                same_size &= count == signed.2;
                now.push((at, (signed.0, start, count, edit.made_from), edit));
            }
            let chunk = &mut self.chunks[number];
            if same_size {
                for (at, signed, edit) in now {
                    let offset: u32 = chunk.signature[..at].iter().map(|s| s.2).sum();
                    let data: Vec<u32> = edit.mesh.indices.iter().map(|index| index + signed.1).collect();
                    store.indices.overwrite(chunk.block.start + offset, bytemuck::cast_slice(&data));
                    chunk.signature[at] = signed;
                }
                report.chunks_patched += 1;
            } else {
                for (at, signed, _) in now {
                    chunk.signature[at] = signed;
                }
                let mut indices: Vec<u32> = Vec::with_capacity(chunk.signature.iter().map(|s| s.2 as usize).sum());
                for &(key, start, _, made_from) in &chunk.signature {
                    let mesh = mesh_of(key, made_from).expect("an item of the list has triangles");
                    indices.extend(mesh.indices.iter().map(|index| index + start));
                }
                give_back(&mut store.indices, &mut chunk.block);
                chunk.block = store.insert_indices(&indices)?;
                chunk.count = indices.len() as u32;
                report.chunks_rebuilt += 1;
            }
        }
        Ok(())
    }
}

/// Gives a block back to its buffer and leaves the owner with an empty one (nothing is freed twice, and a block that holds
/// nothing is not given back at all).
fn give_back(buffer: &mut crate::store::GrowableBuffer, block: &mut Block) {
    if block.reserved() > 0 {
        buffer.remove(*block);
    }
    *block = Block::EMPTY;
}

/// The rectangle that shows a layer: the unit square as two triangles (the row of the composite says where it is).
static QUAD: LazyLock<Mesh> = LazyLock::new(|| Mesh {
    vertices: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(|point| Vertex { point, normal: [0.0, 0.0], distance: 0.0, row: 0 }).to_vec(),
    indices: vec![0, 1, 2, 2, 1, 3],
});
/// What the quad was "made from": it never changes.
static QUAD_STAMP: LazyLock<Stamp> = LazyLock::new(Stamp::fresh);

fn upload(store: &mut GeometryStore, mesh: &Mesh, row: u32) -> Result<Block, AllocError> {
    let vertices: Vec<Vertex> = mesh.vertices.iter().map(|v| Vertex { row, ..*v }).collect();
    store.insert_vertices(&vertices)
}
