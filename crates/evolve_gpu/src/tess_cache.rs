//! Remembers the triangles of every shape and makes new ones only when they can have changed (plan D3 and D7, task 2.2).
//!
//! A shape is tessellated again only when
//! - its **geometry stamp** changed (Phase 1: the path, fill rule, skin, which paints it has, whether its stroke has a width, its
//!   join, cap or miter limit), or
//! - its stroke's **width** moved out of what the triangles allow ([`stroke_width_holds`]: the width itself is in the shape table and
//!   a small change costs nothing, plan C1), or
//! - its **scale bucket** changed and the shape is one whose triangles depend on scale (curves, round joins and caps).
//!
//! Moving, rotating, recoloring, fading or hiding-and-showing a shape, or moving a group it is in, never does. The cache is
//! brought up to date by walking the scene once ([`TessCache::update`]); entries of shapes that are gone or hidden are
//! dropped there.

use crate::cull::{control_extent, Cull, Extent};
use crate::tessellate::{
    bucket_holds, bucket_window, depends_on_scale, scale_bucket, stroke_width_holds, tessellate_mask, tessellate_shape, visible_stroke_width, ShapeMesh, TessLimits, TessWarning,
};
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::revision::Stamp;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use crate::fast_map::FastMap;
use web_time::Instant;

/// The triangles of one shape and what they were made for.
pub struct CachedShape {
    /// The geometry stamp the mesh was made for; for a skinned shape (see `posed`) a stamp of the mesh itself, new each time it is
    /// made again, because the shape's own stamp does not change when the bones move.
    pub geometry: Stamp,
    /// A stamp no other tessellation has: the triangles of this entry, which an item of the draw list remembers it was made from. A shape
    /// made again for a new bucket or a new stroke width has the same geometry stamp but other triangles.
    pub made: Stamp,
    pub bucket: i32,
    /// The width of the stroke these triangles were made for (0: no stroke triangles), see [`stroke_width_holds`].
    pub stroke_width: f32,
    pub depends_on_scale: bool,
    /// Made as a clip mask: only the fill silhouette, whatever the shape's paints are.
    pub mask: bool,
    pub mesh: ShapeMesh,
    /// The shape wants another zoom bucket than this mesh was made for, and has not got it yet (the frame's time for making shapes
    /// ran out): the mesh is still drawn, a little too coarse or too fine, until it is made again.
    pub stale: bool,
    /// For a skinned shape: the geometry stamp of its node and the path as the bones bent it, which is what the mesh was made from.
    /// The mesh is good for as long as both are the same (plan 6.1).
    posed: Option<(Stamp, Path)>,
    seen: u64,
}

impl CachedShape {
    /// What an item of the draw list remembers about the triangles it was made from.
    pub fn identity(&self) -> (Stamp, i32) {
        (self.made, self.bucket)
    }

    /// The path the mesh was made from when the shape is skinned: its outline as the bones bent it (in the shape's own space).
    pub fn posed_path(&self) -> Option<&Path> {
        self.posed.as_ref().map(|(_, path)| path)
    }

    /// The factors by which the scale `scale` (zoom times the shape's own largest scale) can change before this mesh is made again
    /// (see `tessellate::bucket_window`): `(0, 0)`, no limit, for a shape whose triangles do not depend on scale. Only right for
    /// the main entry of a node, which is what a frame without instances draws.
    pub fn scale_window(&self, scale: f32) -> (f32, f32) {
        if self.depends_on_scale {
            bucket_window(self.bucket, scale)
        } else {
            (0.0, 0.0)
        }
    }
}

/// What one [`TessCache::update`] did.
#[derive(Debug, Default, PartialEq)]
pub struct UpdateStats {
    /// Shapes tessellated in this update, because they were new or their geometry or scale changed.
    pub tessellated: Vec<NodeId>,
    /// Shapes whose triangles were still good.
    pub reused: usize,
    /// Entries dropped because the shape is gone, hidden or invisible.
    pub removed: usize,
    /// Shapes left out because they cannot reach the view (only with a cull).
    pub culled: usize,
    /// Skinned shapes that the cache cannot make: those inside a symbol instance (every instance would need its own pose) and clip
    /// masks (the feature gate sends scenes with them to the CPU renderer). Skinned shapes elsewhere are made like any other.
    pub skinned: usize,
    /// Shapes whose making was put off because this update's time for it ran out (see [`TessCache::set_budget`]): either they are
    /// drawn with triangles for another zoom bucket, or (outside the view, only kept ready) not at all yet.
    pub deferred: usize,
    pub warnings: Vec<(NodeId, TessWarning)>,
}

/// The box of a shape's control points, remembered for the geometry stamp it was measured for (for culling).
struct Measured {
    geometry: Stamp,
    extent: Option<Extent>,
    seen: u64,
}

pub struct TessCache {
    entries: FastMap<NodeId, CachedShape>,
    /// A node that is wanted at a second scale in one frame (the art of a symbol, under instances of different sizes): the
    /// triangles for that bucket. Empty for scenes without instances.
    extras: FastMap<(NodeId, i32, bool), CachedShape>,
    extents: FastMap<NodeId, Measured>,
    generation: u64,
    limits: TessLimits,
    /// The time one update may spend making shapes that are not needed at once (milliseconds), and what it has spent.
    budget_ms: Option<f64>,
    spent_ms: f64,
}

/// What one update walks with: the scene (for the symbols), the camera's zoom and the cull.
#[derive(Clone, Copy)]
struct Frame<'a> {
    scene: &'a Scene,
    zoom: f32,
    cull: Option<&'a Cull>,
    /// The walk covers one node and what is below it (see [`TessCache::update_node`]), not the scene: the instances it does not reach
    /// still use the entries it finds, so an entry that is good for the shape but not for this scale is left alone.
    partial: bool,
}

/// One shape (or clip mask) that `make` is asked about.
struct Wanted<'a> {
    node: &'a Node,
    shape: &'a Shape,
    world: Affine2,
    mask: bool,
    /// The shape's box when it was judged against the cull (`Some(None)`: not measured), `None` for a mask.
    extent: Option<Option<Extent>>,
    /// The node is in the art of a symbol, whose entries every instance shares.
    shared: bool,
}

/// How deep instances may be nested inside symbols before the walk stops (a symbol that contains itself would go on for ever).
pub const MAX_INSTANCE_DEPTH: usize = 12;

/// The largest factor by which a transform stretches: how big a unit of the shape's own space is, at most.
pub fn largest_scale(world: &Affine2) -> f32 {
    world.matrix2.x_axis.length().max(world.matrix2.y_axis.length())
}

impl TessCache {
    pub fn new(limits: TessLimits) -> TessCache {
        TessCache { entries: FastMap::default(), extras: FastMap::default(), extents: FastMap::default(), generation: 0, limits, budget_ms: None, spent_ms: 0.0 }
    }

    /// Limits the time one update spends making shapes (plan 5.4, GPU_SAFETY.md B3). A zoom that crosses a bucket makes every
    /// visible curve again, which took over 100 ms at once on the real map. With a budget, once an update has spent it:
    /// a shape that only needs another bucket keeps its old triangles for now, and a shape that is not in the view yet (only in the
    /// margin kept ready for a move) waits. A shape that is in the view and has no triangles at all is always made at once: a
    /// frame never has holes. Whatever waited is made in the next updates, so a view that has `deferred` shapes must be drawn again.
    /// `None` (the default) makes everything in every update.
    pub fn set_budget(&mut self, milliseconds: Option<f64>) {
        self.budget_ms = milliseconds;
    }

    /// The time one update may spend (see [`TessCache::set_budget`]).
    pub fn budget(&self) -> Option<f64> {
        self.budget_ms
    }

    pub fn len(&self) -> usize {
        self.entries.len() + self.extras.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.extras.is_empty()
    }

    /// The main entry of a node (its triangles for the scale it was last made for).
    pub fn get(&self, id: NodeId) -> Option<&CachedShape> {
        self.entries.get(&id)
    }

    /// The triangles of a node for a shape that is drawn at `scale` (zoom times the shape's own largest scale), as the last
    /// update chose them: the main entry when it is good for the scale, else the extra one of the scale's bucket.
    pub fn find(&self, id: NodeId, mask: bool, scale: f32) -> Option<&CachedShape> {
        let main = self.entries.get(&id)?;
        if main.mask == mask && (!main.depends_on_scale || bucket_holds(main.bucket, scale)) {
            return Some(main);
        }
        // Another size of the same art (an instance), else what is there until the right triangles are made.
        self.extras.get(&(id, scale_bucket(scale), mask)).or_else(|| Some(main).filter(|m| m.stale && m.mask == mask))
    }

    /// The entry of a node that holds the triangles an item was made from: its geometry stamp and bucket (a node can have one entry
    /// for each size it is drawn at). `None` if the cache no longer has them.
    pub fn entry_made_from(&self, id: NodeId, made_from: (Stamp, i32)) -> Option<&CachedShape> {
        self.entries
            .get(&id)
            .filter(|e| !e.mask && e.identity() == made_from)
            .or_else(|| self.extras.get(&(id, made_from.1, false)).filter(|e| e.made == made_from.0))
    }

    /// Whether the main entry of `node` is good as it is for the scale `scale` (zoom times the shape's own largest scale): it was made
    /// for the shape's current geometry, as a shape and not a mask, its zoom bucket still holds and so does its stroke width.
    pub fn holds(&self, node: &Node, scale: f32) -> bool {
        let width_holds = |e: &CachedShape| match &node.kind {
            NodeKind::Shape(shape) => stroke_width_holds(e.stroke_width, shape),
            _ => true,
        };
        self.entries.get(&node.id).is_some_and(|e| !e.mask && e.geometry == node.revs.geometry && (!e.depends_on_scale || bucket_holds(e.bucket, scale)) && width_holds(e))
    }

    /// Makes the triangles of one shape again, now, for the zoom it is seen at (one edit: an outline changed, a shape scaled into
    /// another bucket), and returns them. Ignores the budget: it is one shape that someone is working on.
    pub fn remake(&mut self, node: &Node, shape: &Shape, world: &Affine2, zoom: f32) -> &CachedShape {
        let bucket = scale_bucket(zoom * largest_scale(world));
        let mesh = tessellate_shape(shape, bucket, &self.limits);
        let made = CachedShape {
            geometry: node.revs.geometry,
            made: Stamp::fresh(),
            bucket,
            stroke_width: visible_stroke_width(shape),
            depends_on_scale: depends_on_scale(shape),
            mask: false,
            mesh,
            stale: false,
            posed: None,
            seen: self.generation,
        };
        self.entries.insert(node.id, made);
        &self.entries[&node.id]
    }

    /// The box of a shape's control points in its own space, as measured by the last update with a cull (`None` when it was not
    /// measured: callers keep such a shape).
    pub fn extent(&self, id: NodeId) -> Option<Extent> {
        self.extents.get(&id).and_then(|m| m.extent)
    }

    /// Bytes the cached triangles take on the card.
    pub fn gpu_bytes(&self) -> u64 {
        self.entries.values().chain(self.extras.values()).map(|e| e.mesh.gpu_bytes()).sum()
    }

    /// Brings the cache up to date with `scene` seen at camera zoom `zoom` (pixels per unit of scene space).
    pub fn update(&mut self, scene: &Scene, zoom: f32) -> UpdateStats {
        self.update_in(scene, zoom, None)
    }

    /// Like [`TessCache::update`], but with a `cull`, only the shapes that can reach the view are made and kept; the others
    /// are dropped (and made again when they come near).
    pub fn update_in(&mut self, scene: &Scene, zoom: f32, cull: Option<&Cull>) -> UpdateStats {
        self.generation += 1;
        self.spent_ms = 0.0;
        let mut stats = UpdateStats::default();
        self.walk(Frame { scene, zoom, cull, partial: false }, &scene.nodes, Affine2::IDENTITY, &mut stats, 0);
        let generation = self.generation;
        let before = self.len();
        self.entries.retain(|_, entry| entry.seen == generation);
        self.extras.retain(|_, entry| entry.seen == generation);
        self.extents.retain(|_, measured| measured.seen == generation);
        stats.removed = before - self.len();
        stats
    }

    /// Like [`TessCache::update_in`] for one node of the scene and what is below it, without a walk of the rest and without forgetting
    /// anything: entries of shapes elsewhere stay as they are, and so do those of shapes below the node that the cull leaves out
    /// (they are dropped by the next full update). `parent` is the world transform of the node's parent. For a node that moved: its
    /// shapes may need triangles for another zoom bucket, or may have come near enough to be made.
    pub fn update_node(&mut self, scene: &Scene, node: &Node, parent: Affine2, zoom: f32, cull: Option<&Cull>) -> UpdateStats {
        // (A new generation: an entry that the last update used would otherwise look like one that another visit has taken.)
        self.generation += 1;
        self.spent_ms = 0.0;
        let mut stats = UpdateStats::default();
        self.walk(Frame { scene, zoom, cull, partial: true }, std::slice::from_ref(node), parent, &mut stats, 0);
        stats
    }

    /// The box of the shape's control points, measured again only when its geometry stamp changed.
    fn extent_of(&mut self, node: &Node, shape: &Shape) -> Option<Extent> {
        let generation = self.generation;
        let stamp = node.revs.geometry;
        let measured = self.extents.entry(node.id).or_insert_with(|| Measured { geometry: stamp, extent: control_extent(&shape.path), seen: generation });
        if measured.geometry != stamp {
            *measured = Measured { geometry: stamp, extent: control_extent(&shape.path), seen: generation };
        }
        measured.seen = generation;
        measured.extent
    }

    fn walk(&mut self, frame: Frame, nodes: &[Node], parent: Affine2, stats: &mut UpdateStats, depth: usize) {
        for node in nodes {
            if !node.visible || node.opacity <= 0.0 {
                continue;
            }
            let world = parent * node.transform;
            match &node.kind {
                NodeKind::Group(children) => match node.clip_parts() {
                    Some((content, mask_node, mask_shape)) => {
                        // The mask counts as a silhouette only: its visibility, opacity, paints and stroke do not matter. A mask
                        // that cannot reach the view hides the whole group, so the content is not even made.
                        if mask_shape.skin.is_some() {
                            stats.skinned += 1;
                            continue;
                        }
                        let mask_world = world * mask_node.transform;
                        if let Some(cull) = frame.cull {
                            let extent = self.extent_of(mask_node, mask_shape);
                            if !cull.may_touch(extent, &mask_world, None) {
                                stats.culled += 1;
                                continue;
                            }
                        }
                        self.make(Wanted { node: mask_node, shape: mask_shape, world: mask_world, mask: true, extent: None, shared: depth > 0 }, frame, stats);
                        self.walk(frame, content, world, stats, depth);
                    }
                    None => self.walk(frame, children, world, stats, depth),
                },
                // A skinned shape inside an instance: the cache has one entry for the art of the symbol, and every instance would bend it
                // differently.
                NodeKind::Shape(shape) if shape.skin.is_some() && depth > 0 => stats.skinned += 1,
                NodeKind::Shape(shape) => {
                    let mut extent = None;
                    // (A skinned shape is not judged by its bind pose: it is always made, and the draw list judges its bent outline.)
                    if let (Some(cull), true) = (frame.cull, shape.skin.is_none()) {
                        extent = self.extent_of(node, shape);
                        if !cull.may_touch(extent, &world, shape.stroke.as_ref()) {
                            stats.culled += 1;
                            continue;
                        }
                    }
                    self.make(Wanted { node, shape, world, mask: false, extent: Some(extent), shared: depth > 0 }, frame, stats);
                }
                // The art of a symbol is made once, whatever number of instances there are: the nodes of the master are the same
                // for all of them (only where they are, and how big, differs).
                NodeKind::Instance(instance) => {
                    if let (Some(symbol), true) = (frame.scene.symbol(instance.symbol), depth < MAX_INSTANCE_DEPTH) {
                        self.walk(frame, std::slice::from_ref(&symbol.root), world, stats, depth + 1);
                    }
                }
                NodeKind::Camera(_) => {}
            }
        }
    }

    /// Makes the triangles of a shape (or of a clip mask: its fill silhouette only) unless the cache holds good ones.
    ///
    /// A node has one main entry. The art of a symbol is the same node for every instance, and instances can be at different
    /// scales, so the node can be wanted at two buckets in one frame: the first visit gets the main entry, and a visit that finds
    /// it taken (used in this frame) for another scale gets an extra one for its bucket.
    ///
    /// With the frame's time for making shapes spent (see [`TessCache::set_budget`]) a shape that only needs another bucket keeps its
    /// mesh for now, and one that has none and is not in the view waits.
    fn make(&mut self, wanted: Wanted, frame: Frame, stats: &mut UpdateStats) {
        let Wanted { node, shape, world, mask, extent, shared } = wanted;
        let world = &world;
        let scale = frame.zoom * largest_scale(world);
        let over = self.budget_ms.is_some_and(|budget| self.spent_ms >= budget);
        let bucket = scale_bucket(scale);
        let stamp = node.revs.geometry;
        let generation = self.generation;
        // A skinned shape is made from its outline as the bones bend it now (the CPU renderer's `effective_path`).
        let pose: Option<Path> = if mask { None } else { shape.skin.as_ref().map(|skin| skin.deform(frame.scene, &shape.path, *world).unwrap_or_else(|| shape.path.clone())) };
        // What an entry must have been made from to be the one wanted: the same geometry, and for a skinned shape the same pose.
        let same_source = |e: &CachedShape| match &pose {
            None => e.posed.is_none() && e.geometry == stamp,
            Some(path) => e.posed.as_ref().is_some_and(|(made_for, posed)| *made_for == stamp && posed == path),
        };
        // A shape made for the neighboring bucket is kept while the scale is only a little over the edge.
        let usable = |e: &CachedShape| e.mask == mask && same_source(e) && (!e.depends_on_scale || bucket_holds(e.bucket, scale)) && (mask || stroke_width_holds(e.stroke_width, shape));
        let mut extra = false;
        match self.entries.get_mut(&node.id) {
            Some(e) if usable(e) => {
                e.seen = generation;
                e.stale = false;
                stats.reused += 1;
                return;
            }
            // Used in this frame at another scale; or (a walk of one node) made for this shape and in use by instances elsewhere.
            Some(e) if e.seen == generation || (frame.partial && shared && e.mask == mask && same_source(e)) => extra = true,
            // Only the bucket differs, and there is no more time: the old triangles will do for now.
            Some(e) if over && !mask && !e.mask && same_source(e) => {
                e.seen = generation;
                e.stale = true;
                stats.deferred += 1;
                return;
            }
            _ => {}
        }
        if extra {
            if let Some(e) = self.extras.get_mut(&(node.id, bucket, mask)).filter(|e| same_source(e) && (mask || stroke_width_holds(e.stroke_width, shape))) {
                e.seen = generation;
                stats.reused += 1;
                return;
            }
        }
        // Nothing to show yet for this shape: only what is in the view has to be made now.
        if over && !mask && !extra {
            let in_view = match (frame.cull, extent) {
                (Some(cull), Some(extent)) => cull.touches_view(extent, world, shape.stroke.as_ref()),
                _ => true,
            };
            if !in_view {
                stats.deferred += 1;
                return;
            }
        }
        let started = Instant::now();
        let bent;
        let drawn: &Shape = match &pose {
            Some(path) => {
                bent = Shape { path: path.clone(), fill: shape.fill.clone(), stroke: shape.stroke.clone(), fill_rule: shape.fill_rule, skin: None };
                &bent
            }
            None => shape,
        };
        let mesh = if mask { tessellate_mask(drawn, bucket, &self.limits) } else { tessellate_shape(drawn, bucket, &self.limits) };
        self.spent_ms += started.elapsed().as_secs_f64() * 1000.0;
        stats.warnings.extend(mesh.warnings.iter().map(|w| (node.id, w.clone())));
        stats.tessellated.push(node.id);
        let made = CachedShape {
            geometry: if pose.is_some() { Stamp::fresh() } else { stamp },
            made: Stamp::fresh(),
            bucket,
            stroke_width: visible_stroke_width(drawn),
            depends_on_scale: depends_on_scale(drawn),
            mask,
            mesh,
            stale: false,
            posed: pose.map(|path| (stamp, path)),
            seen: generation,
        };
        if extra {
            self.extras.insert((node.id, bucket, mask), made);
        } else {
            self.entries.insert(node.id, made);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::path::{Anchor, Path, SubPath};
    use evolve_engine::scene::Shape;
    use evolve_engine::style::{Color, LineCap, LineJoin, Paint, Stroke};
    use evolve_engine::Vec2;

    /// 50 shapes: every second one is a circle (depends on scale), the others straight-sided polygons (do not).
    fn scene() -> (Scene, Vec<NodeId>) {
        let mut scene = Scene::default();
        let mut ids = Vec::new();
        for i in 0..50 {
            let at = Vec2::new(i as f32 * 30.0, 0.0);
            let path = if i % 2 == 0 {
                Path::ellipse(Vec2::ZERO, Vec2::splat(10.0))
            } else {
                Path { subpaths: vec![SubPath { anchors: [(0.0, 0.0), (20.0, 0.0), (10.0, 20.0)].map(|(x, y)| Anchor::corner(Vec2::new(x, y))).to_vec(), closed: true }] }
            };
            let mut shape = Shape::new(path);
            shape.fill = Some(Paint::Solid(Color::rgb(200, 30, 30)));
            shape.stroke = Some(Stroke { paint: Paint::Solid(Color::BLACK), width: 2.0, join: LineJoin::Miter, cap: LineCap::Butt, miter_limit: 4.0 });
            let mut node = scene.new_node(format!("shape {i}"), NodeKind::Shape(shape));
            node.transform = evolve_engine::Affine2::from_translation(at);
            ids.push(scene.add(node, None));
        }
        (scene, ids)
    }

    fn update(cache: &mut TessCache, scene: &Scene, zoom: f32) -> UpdateStats {
        cache.update(scene, zoom)
    }

    #[test]
    fn the_first_update_tessellates_everything_and_the_second_nothing() {
        let (scene, ids) = scene();
        let mut cache = TessCache::new(TessLimits::default());
        let first = update(&mut cache, &scene, 1.0);
        assert_eq!((first.tessellated.len(), first.reused, first.removed), (50, 0, 0));
        assert_eq!(cache.len(), 50);
        assert!(cache.get(ids[0]).unwrap().mesh.fill.is_some() && cache.gpu_bytes() > 0);
        let second = update(&mut cache, &scene, 1.0);
        assert_eq!((second.tessellated.len(), second.reused), (0, 50));
    }

    /// The test the plan asks for: editing one shape tessellates only that shape again.
    #[test]
    fn editing_the_geometry_of_one_shape_tessellates_only_that_shape() {
        let (mut scene, ids) = scene();
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);

        scene.find_mut(ids[7]).unwrap().path_mut().unwrap().subpaths[0].anchors[1].pos = Vec2::new(40.0, 0.0);
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!(stats.tessellated, [ids[7]], "only the edited shape");
        assert_eq!(stats.reused, 49);
        // And the new triangles are really the new shape.
        let wide = cache.get(ids[7]).unwrap().mesh.fill.as_ref().unwrap().vertices.iter().map(|v| v.point[0]).fold(0.0, f32::max);
        assert_eq!(wide, 40.0);

        // Other geometry edits: the stroke's width (stamps everything about that node), the fill rule.
        if let NodeKind::Shape(shape) = &mut scene.find_mut(ids[8]).unwrap().kind {
            shape.stroke.as_mut().unwrap().width = 3.5;
        }
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated, [ids[8]]);
        scene.shape_mut(ids[9]).unwrap().fill_rule = evolve_engine::style::FillRule::EvenOdd;
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated, [ids[9]]);
    }

    #[test]
    fn moving_recoloring_fading_or_moving_a_group_tessellates_nothing() {
        let (mut scene, ids) = scene();
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);
        scene.find_mut(ids[2]).unwrap().set_transform(evolve_engine::Affine2::from_translation(Vec2::new(5.0, 5.0)));
        scene.find_mut(ids[3]).unwrap().set_opacity(0.5);
        scene.find_mut(ids[4]).unwrap().set_solid_fill_color(Color::rgb(1, 2, 3));
        scene.find_mut(ids[5]).unwrap().set_solid_stroke_color(Color::rgb(3, 2, 1));
        *scene.find_mut(ids[6]).unwrap().name_mut() = "renamed".into();
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!((stats.tessellated.len(), stats.reused), (0, 50), "{stats:?}");

        // A group around some shapes: moving it changes their world matrices, not their triangles.
        let group = scene.group(&ids[10..20]).unwrap();
        let stats = update(&mut cache, &scene, 1.0);
        assert!(stats.tessellated.is_empty(), "grouping moves nodes in the tree, not their geometry: {stats:?}");
        scene.find_mut(group).unwrap().set_transform(evolve_engine::Affine2::from_translation(Vec2::new(100.0, 40.0)));
        assert!(update(&mut cache, &scene, 1.0).tessellated.is_empty());
    }

    #[test]
    fn a_zoom_inside_a_bucket_changes_nothing_and_crossing_a_bucket_redoes_only_the_shapes_that_depend_on_scale() {
        let (scene, ids) = scene();
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);
        assert!(update(&mut cache, &scene, 0.9).tessellated.is_empty(), "0.9 is in the same bucket as 1.0: [0.5, 1]");
        assert!(update(&mut cache, &scene, 0.6).tessellated.is_empty());
        let stats = update(&mut cache, &scene, 2.0);
        let circles: Vec<NodeId> = ids.iter().copied().step_by(2).collect();
        assert_eq!(stats.tessellated, circles, "the 25 circles, not the 25 triangles");
        assert_eq!(stats.reused, 25);
        // The finer bucket gave the circles more vertices.
        let (fine, coarse) = (cache.get(ids[0]).unwrap(), cache.get(ids[1]).unwrap());
        assert_eq!(fine.bucket, 1);
        assert!(fine.mesh.vertex_count() > 12 && coarse.mesh.vertex_count() <= 6 * 2 + 3 * 2);
        // And back out again: the circles are made coarser again (a cache of one bucket per shape).
        assert_eq!(update(&mut cache, &scene, 0.5).tessellated, circles);
        assert!(update(&mut cache, &scene, 0.5).tessellated.is_empty());
    }

    #[test]
    fn a_group_scale_counts_in_the_effective_scale() {
        let (mut scene, ids) = scene();
        let group = scene.group(&ids[0..4]).unwrap();
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);
        scene.find_mut(group).unwrap().set_transform(evolve_engine::Affine2::from_scale(Vec2::splat(8.0)));
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!(stats.tessellated, [ids[0], ids[2]], "the two circles inside the 8 times larger group, not the triangles");
        assert_eq!(cache.get(ids[0]).unwrap().bucket, 3);
    }

    #[test]
    fn removed_hidden_and_skinned_shapes_leave_the_cache_or_stay_out_of_it() {
        let (mut scene, ids) = scene();
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);
        scene.remove(ids[0]);
        scene.find_mut(ids[1]).unwrap().set_visible(false);
        scene.find_mut(ids[2]).unwrap().set_opacity(0.0);
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!((stats.removed, cache.len()), (3, 47));
        assert!(cache.get(ids[0]).is_none() && cache.get(ids[1]).is_none() && cache.get(ids[2]).is_none());
        // Showing a hidden shape again tessellates it again (its entry was dropped).
        scene.find_mut(ids[1]).unwrap().set_visible(true);
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated, [ids[1]]);

        // A skinned shape is made from its outline as the bones bend it (the entry is made again, for the pose).
        let upper = scene.add_bone(None, Vec2::new(0.0, 0.0), 0.0, 40.0);
        assert!(scene.bind_skin(ids[11], &[upper]));
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!((stats.skinned, stats.tessellated.contains(&ids[11])), (0, true));
        assert!(cache.get(ids[11]).unwrap().posed_path().is_some());
    }

    /// A skin on a shape: the entry holds the outline as bent, and is made again when the bones move and not otherwise.
    #[test]
    fn a_skinned_shape_is_made_from_the_bent_outline_and_again_only_when_the_pose_changes() {
        let (mut scene, ids) = scene();
        let bone = scene.add_bone(None, Vec2::new(0.0, 0.0), 0.0, 40.0);
        assert!(scene.bind_skin(ids[11], &[bone]));
        let mut cache = TessCache::new(TessLimits::default());
        update(&mut cache, &scene, 1.0);
        let first = cache.get(ids[11]).unwrap();
        let (mesh_stamp, bent_before) = (first.geometry, first.posed_path().unwrap().clone());
        assert!(update(&mut cache, &scene, 1.0).tessellated.is_empty(), "the same pose: the triangles are good");
        assert_eq!(cache.get(ids[11]).unwrap().geometry, mesh_stamp);
        scene.set_bone_world(bone, Vec2::new(0.0, 0.0), 0.8, None);
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated, [ids[11]], "the bone moved: this shape and no other is made again");
        let bent = cache.get(ids[11]).unwrap();
        assert_ne!(bent.geometry, mesh_stamp, "a new mesh has a new identity, so the draw list sends it");
        assert_ne!(bent.posed_path().unwrap(), &bent_before);
        // Unbinding the skin makes it a plain shape again.
        assert!(scene.unbind_skin(ids[11]));
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated, [ids[11]]);
        assert!(cache.get(ids[11]).unwrap().posed_path().is_none());
    }

    /// Inside the art of a symbol every instance would bend it differently, and the cache has one entry for it.
    #[test]
    fn a_skinned_shape_inside_an_instance_is_counted_and_left_alone() {
        let (mut scene, ids) = scene();
        let bone = scene.add_bone(None, Vec2::new(0.0, 0.0), 0.0, 40.0);
        assert!(scene.bind_skin(ids[3], &[bone]));
        scene.make_symbol(&[ids[3]], "Skinned").expect("a symbol");
        let mut cache = TessCache::new(TessLimits::default());
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!(stats.skinned, 1);
        assert!(cache.get(ids[3]).is_none());
    }

    #[test]
    fn warnings_of_the_tessellator_reach_the_caller() {
        let (mut scene, ids) = scene();
        scene.find_mut(ids[0]).unwrap().path_mut().unwrap().subpaths[0].anchors[0].pos = Vec2::new(f32::NAN, 0.0);
        let mut cache = TessCache::new(TessLimits::default());
        let stats = update(&mut cache, &scene, 1.0);
        assert_eq!(stats.warnings, [(ids[0], TessWarning::NotFinite)]);
        assert!(cache.get(ids[0]).unwrap().mesh.fill.is_none(), "the shape is in the cache, empty, so it is not retried on every frame");
        assert!(update(&mut cache, &scene, 1.0).tessellated.is_empty());
    }

    #[test]
    fn only_shapes_that_can_reach_the_view_are_made_and_a_moved_path_is_judged_again() {
        let (mut scene, ids) = scene(); // 50 shapes 30 units apart: x from 0 to 1470
        let cull = Cull::new(evolve_engine::Affine2::IDENTITY, (200, 100));
        let mut cache = TessCache::new(TessLimits::default());
        let stats = cache.update_in(&scene, 1.0, Some(&cull));
        // The view is 200 wide and the margin 114 each side: shapes up to about x = 314 are near.
        assert!(stats.culled > 35 && stats.tessellated.len() < 15, "{} culled, {} made", stats.culled, stats.tessellated.len());
        assert_eq!(stats.culled + stats.tessellated.len(), 50);
        assert!(cache.get(ids[1]).is_some() && cache.get(ids[40]).is_none());
        // Nothing changed: nothing is made again, and the far ones stay out.
        let again = cache.update_in(&scene, 1.0, Some(&cull));
        assert!(again.tessellated.is_empty() && again.culled == stats.culled);

        // The path of a far shape is edited so that it lies in the view: its extent must be measured again.
        {
            let mut node = scene.find_mut(ids[40]).unwrap();
            for anchor in node.path_mut().unwrap().subpaths[0].anchors.iter_mut() {
                anchor.pos -= Vec2::new(1200.0, 0.0);
            }
        }
        let moved = cache.update_in(&scene, 1.0, Some(&cull));
        assert_eq!(moved.tessellated, [ids[40]], "its new place is near the view");
        // And a near shape is edited to lie far away: it leaves the cache.
        {
            let mut node = scene.find_mut(ids[1]).unwrap();
            for anchor in node.path_mut().unwrap().subpaths[0].anchors.iter_mut() {
                anchor.pos += Vec2::new(5000.0, 0.0);
            }
        }
        let away = cache.update_in(&scene, 1.0, Some(&cull));
        assert!(away.tessellated.is_empty() && cache.get(ids[1]).is_none());

        // Without a cull, everything is made.
        let mut all = TessCache::new(TessLimits::default());
        assert_eq!(all.update_in(&scene, 1.0, None).tessellated.len(), 50);
    }

    #[test]
    fn a_zoom_that_sits_on_a_bucket_edge_does_not_make_every_circle_again_on_every_frame() {
        let (scene, ids) = scene();
        let circles = ids.iter().step_by(2).count();
        let mut cache = TessCache::new(TessLimits::default());
        assert_eq!(update(&mut cache, &scene, 1.0).tessellated.len(), 50);
        // A zoom wobbling by a few percent around 1.0 (the edge between buckets 0 and 1) and around 2.0: nothing is made.
        for zoom in [1.0, 1.02, 0.98, 1.05, 0.97, 1.0, 1.08, 0.95] {
            assert!(update(&mut cache, &scene, zoom).tessellated.is_empty(), "zoom {zoom} made shapes again");
        }
        // A real change of zoom still does it: twice as much is a finer bucket for every circle, and not for the polygons.
        assert_eq!(update(&mut cache, &scene, 2.4).tessellated.len(), circles);
        // And now the wobble around the new edge, from either side, is quiet.
        for zoom in [2.4, 2.1, 2.0, 1.95, 2.2, 2.0] {
            assert!(update(&mut cache, &scene, zoom).tessellated.is_empty(), "zoom {zoom} made shapes again");
        }
        // Far enough back it is made coarser again (10% is the most that the edge is stretched).
        assert_eq!(update(&mut cache, &scene, 0.6).tessellated.len(), circles);
    }
}
