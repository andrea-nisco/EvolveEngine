//! The scene graph: a tree of groups and vector shapes.

use crate::animation::Clip;
use crate::boolean::{self, BoolOp};
use crate::game_camera::{CameraView, GameCamera};
use crate::geometry::{mean_scale, Affine2, Rect, Vec2};
use crate::path::Path;
use crate::style::{Color, FillRule, Paint, Stroke};
use crate::physics::Body;
use crate::revision::{Changes, Meta, NodeMut, Revs, SceneRevs, Stamp};
use crate::rig::{Bone, Ik, Skin};
use crate::symbol::{Instance, Symbol};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// A filled and/or stroked vector path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub path: Path,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub fill_rule: FillRule,
    /// If set, the outline follows bones; `path` is then the bind pose.
    #[serde(default)]
    pub skin: Option<Box<Skin>>,
}

impl Shape {
    pub fn new(path: Path) -> Self {
        Self {
            path,
            fill: Some(Paint::Solid(Color::rgb(86, 156, 214))),
            stroke: Some(Stroke::solid(Color::rgb(24, 40, 64), 2.0)),
            fill_rule: FillRule::NonZero,
            skin: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Children are drawn in order: the last child is on top.
    Group(Vec<Node>),
    Shape(Shape),
    /// A placed copy of a symbol (see [`crate::symbol`]).
    Instance(Instance),
    /// The frame the player sees (see [`crate::game_camera`]). It draws nothing.
    Camera(GameCamera),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    /// For groups: the top-most child acts as a clipping mask for the others.
    #[serde(default)]
    pub clip: bool,
    /// If set, this group is a bone (see [`crate::rig`]).
    #[serde(default)]
    pub bone: Option<Bone>,
    /// Makes the node take part in collision and physics (see [`crate::physics`]).
    #[serde(default)]
    pub body: Option<Body>,
    /// Script source with event handlers such as `on_update(me, dt)` (see [`crate::script`]).
    #[serde(default)]
    pub script: Option<String>,
    /// Transform relative to the parent node.
    pub transform: Affine2,
    pub kind: NodeKind,
    /// Which versions of this node's content exist; see [`crate::revision`]. Not saved, not compared.
    #[serde(skip)]
    pub revs: Meta<Revs>,
}

impl Node {
    /// For a clipping group: the content nodes, plus the mask child and its shape.
    /// `None` for ordinary nodes, or when the top child is not a shape (the clip is then ignored).
    pub fn clip_parts(&self) -> Option<(&[Node], &Node, &Shape)> {
        if !self.clip {
            return None;
        }
        match self.children().split_last()? {
            (mask @ Node { kind: NodeKind::Shape(shape), .. }, content) => Some((content, mask, shape)),
            _ => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match &self.kind {
            NodeKind::Group(children) => children,
            NodeKind::Shape(_) | NodeKind::Instance(_) | NodeKind::Camera(_) => &[],
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, NodeKind::Group(_))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Unique within the project; scripts switch scenes by this name.
    #[serde(default = "default_scene_name")]
    pub name: String,
    /// Top-level nodes in draw order (last is on top).
    pub nodes: Vec<Node>,
    /// Screen size and color of projects saved before cameras existed. Only read from files:
    /// loading turns them into a camera node (see [`Scene::upgrade_legacy_view`]).
    #[serde(default, rename = "size", skip_serializing_if = "Option::is_none")]
    pub(crate) legacy_size: Option<Vec2>,
    #[serde(default, rename = "background", skip_serializing_if = "Option::is_none")]
    pub(crate) legacy_background: Option<Color>,
    /// Master artwork referenced by [`NodeKind::Instance`] nodes.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Animation clips; each animates nodes of this scene.
    #[serde(default)]
    pub clips: Vec<Clip>,
    /// Inverse-kinematics constraints on bone chains.
    #[serde(default)]
    pub ik: Vec<Ik>,
    /// Source of ids for nodes and symbols.
    pub(crate) next_id: u64,
    /// Which versions of the scene exist; see [`crate::revision`]. Not saved, not compared.
    #[serde(skip)]
    pub(crate) revs: Meta<SceneRevs>,
    /// What hit tests remember between calls; see [`HitCache`]. Not saved, not compared.
    #[serde(skip)]
    pub(crate) hits: Meta<HitCache>,
}

/// Per node id: the geometry stamp a box was measured for, and the box.
type Boxes = Vec<Option<(Stamp, Option<Rect>)>>;

/// What [`Scene::hit_test`] remembers between calls: the box of every shape's control points, found by the shape's node id and
/// good for as long as the node's geometry stamp is the one it was measured for (see [`crate::revision`]). The box lets a hit test
/// skip the shapes the point is far from, without flattening their paths, which is nearly all of them. A copy of a scene starts with
/// nothing (undo makes copies, and a copy's stamps may mean another outline).
#[derive(Default)]
pub(crate) struct HitCache {
    boxes: Mutex<Boxes>,
}

impl Clone for HitCache {
    fn clone(&self) -> Self {
        HitCache::default()
    }
}

impl std::fmt::Debug for HitCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HitCache")
    }
}

/// Ids above this are measured every time instead of being remembered (the memory is a slot for each id up to the largest).
const MAX_CACHED_ID: u64 = 1 << 22;

/// The box of the control points of `shape` (the shape of `node`), from `cache` when its geometry stamp is the one it was measured for.
fn control_box(cache: &mut Boxes, node: &Node, shape: &Shape) -> Option<Rect> {
    if node.id.0 >= MAX_CACHED_ID {
        return shape.path.control_bounds();
    }
    let slot = node.id.0 as usize;
    if slot >= cache.len() {
        cache.resize(slot + 1, None);
    }
    match cache[slot] {
        Some((stamp, found)) if stamp == node.revs.geometry => {
            debug_assert!(found == shape.path.control_bounds(), "a change to the outline of a node was not recorded in its change stamps (see evolve_engine::revision)");
            found
        }
        _ => {
            let found = shape.path.control_bounds();
            cache[slot] = Some((node.revs.geometry, found));
            found
        }
    }
}

/// Whether a point `local` (in the shape's own space) can be on the shape at all, given the box of its control points: no
/// when it is farther from the box than the stroke reaches (half its width) and the tolerances (the flattened path is within the
/// tolerance of the curve) can make up for. Yes whenever that cannot be judged.
fn could_reach(extent: Option<Rect>, shape: &Shape, local: Vec2, tol: f32) -> bool {
    let Some(extent) = extent else { return true };
    let half_width = shape.stroke.as_ref().filter(|s| s.width.is_finite()).map_or(0.0, |s| (s.width * 0.5).max(0.0));
    let reach = half_width + 2.0 * tol;
    !reach.is_finite() || extent.expanded(reach).contains(local)
}

fn default_scene_name() -> String {
    "Scene 1".into()
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            name: default_scene_name(),
            nodes: Vec::new(),
            legacy_size: None,
            legacy_background: None,
            symbols: Vec::new(),
            clips: Vec::new(),
            ik: Vec::new(),
            next_id: 1,
            revs: Meta::default(),
            hits: Meta::default(),
        }
    }
}

// --- Tree lookup helpers -----------------------------------------------------------------------

fn find_in(nodes: &[Node], id: NodeId) -> Option<&Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_in(node.children(), id) {
            return Some(found);
        }
    }
    None
}

fn find_in_mut(nodes: &mut [Node], id: NodeId) -> Option<&mut Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let NodeKind::Group(children) = &mut node.kind {
            if let Some(found) = find_in_mut(children, id) {
                return Some(found);
            }
        }
    }
    None
}

/// Ids from the top-level ancestor down to `id` itself.
fn path_in(nodes: &[Node], id: NodeId) -> Option<Vec<NodeId>> {
    for node in nodes {
        if node.id == id {
            return Some(vec![id]);
        }
        if let Some(mut rest) = path_in(node.children(), id) {
            rest.insert(0, node.id);
            return Some(rest);
        }
    }
    None
}

fn collect_ids(nodes: &[Node], out: &mut Vec<NodeId>) {
    for node in nodes {
        out.push(node.id);
        collect_ids(node.children(), out);
    }
}

/// World-space bounds of `node`, whose own world transform is `world`.
pub(crate) fn node_bounds(scene: &Scene, node: &Node, world: Affine2) -> Option<Rect> {
    if let Some((_, mask, shape)) = node.clip_parts() {
        let mask_world = world * mask.transform;
        return shape.effective_path(scene, mask_world).bounds_under(&mask_world);
    }
    match &node.kind {
        NodeKind::Shape(shape) => shape.effective_path(scene, world).bounds_under(&world),
        NodeKind::Group(children) => children
            .iter()
            .filter_map(|child| node_bounds(scene, child, world * child.transform))
            .reduce(|a, b| a.union(&b)),
        NodeKind::Instance(instance) => {
            let root = &scene.symbol(instance.symbol)?.root;
            node_bounds(scene, root, world * root.transform)
        }
        NodeKind::Camera(camera) => Some(CameraView::new(camera, &world).bounds()),
    }
}

/// Returns true and fills `chain` (outermost first) if `p` hits something inside `nodes`.
fn hit_in(scene: &Scene, nodes: &[Node], parent_world: Affine2, p: Vec2, tolerance: f32, chain: &mut Vec<NodeId>, mut cache: Option<&mut Boxes>) -> bool {
    for node in nodes.iter().rev() {
        if !node.visible || node.locked {
            continue;
        }
        let world = parent_world * node.transform;
        if world.matrix2.determinant().abs() < 1e-9 {
            continue;
        }
        chain.push(node.id);
        let hit = match &node.kind {
            NodeKind::Group(_) if node.clip_parts().is_some() => {
                let (content, mask, shape) = node.clip_parts().expect("checked above");
                let mask_world = world * mask.transform;
                let local = mask_world.inverse().transform_point2(p);
                shape.effective_path(scene, mask_world).contains(local, shape.fill_rule, 0.1 * tolerance)
                    && hit_in(scene, content, world, p, tolerance, chain, cache.as_deref_mut())
            }
            NodeKind::Group(children) => hit_in(scene, children, world, p, tolerance, chain, cache.as_deref_mut()),
            NodeKind::Instance(instance) => scene.symbol(instance.symbol).is_some_and(|symbol| {
                // The master's inner nodes are not part of the tree, so keep only the instance in the chain.
                hit_in(scene, std::slice::from_ref(&symbol.root), world, p, tolerance, &mut Vec::new(), cache.as_deref_mut())
            }),
            NodeKind::Shape(shape) => {
                let local = world.inverse().transform_point2(p);
                let tol = tolerance / mean_scale(&world);
                // Most shapes are nowhere near the point: the box of the control points says so without flattening the path (a skinned
                // shape is bent at draw time, its outline is not the path).
                let apart = shape.skin.is_none() && cache.as_deref_mut().is_some_and(|boxes| !could_reach(control_box(boxes, node, shape), shape, local, tol));
                if apart {
                    false
                } else {
                    let path = shape.effective_path(scene, world);
                    let on_fill = shape.fill.is_some() && path.contains(local, shape.fill_rule, tol * 0.5);
                    let on_stroke =
                        shape.stroke.as_ref().is_some_and(|s| path.distance_to_outline(local, tol * 0.5) <= s.width * 0.5 + tol);
                    on_fill || on_stroke
                }
            }
            // Only the frame's border is hit, so the objects inside it stay selectable.
            NodeKind::Camera(camera) => CameraView::new(camera, &world).distance_to_border(p) <= tolerance,
        };
        if hit {
            return true;
        }
        chain.pop();
    }
    false
}

impl Scene {
    // --- Creation ------------------------------------------------------------------------------

    pub fn new_node(&mut self, name: impl Into<String>, kind: NodeKind) -> Node {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        Node {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: false,
            bone: None,
            body: None,
            script: None,
            transform: Affine2::IDENTITY,
            kind,
            revs: Meta::default(),
        }
    }

    /// Adds `node` on top of `parent`'s children (or on top of the scene).
    pub fn add(&mut self, node: Node, parent: Option<NodeId>) -> NodeId {
        let id = node.id;
        self.insert(node, parent, usize::MAX);
        id
    }

    /// Inserts `node` into `parent` at `index` (clamped). Falls back to the top level if `parent` is not a group.
    pub fn insert(&mut self, node: Node, parent: Option<NodeId>, index: usize) {
        let parent = parent.filter(|&p| self.find(p).is_some_and(Node::is_group));
        let siblings = match parent.and_then(|p| find_in_mut(&mut self.nodes, p)) {
            Some(Node { kind: NodeKind::Group(children), .. }) => children,
            _ => &mut self.nodes,
        };
        let index = index.min(siblings.len());
        siblings.insert(index, node);
        self.touch_node(parent, Changes::STRUCTURE);
    }

    // --- Queries -------------------------------------------------------------------------------

    pub fn find(&self, id: NodeId) -> Option<&Node> {
        find_in(&self.nodes, id)
    }

    /// A node for changing. Everything changed through it is recorded; see [`crate::revision`].
    pub fn find_mut(&mut self, id: NodeId) -> Option<NodeMut<'_>> {
        let Scene { nodes, revs, .. } = self;
        find_in_mut(nodes, id).map(|node| NodeMut::new(node, &mut revs.0))
    }

    /// A shape for changing, as a plain reference, so that a function can hand it around. The node's geometry and paint
    /// are stamped when this is called, so call it only when about to change the shape. `None` for other kinds of node.
    pub fn shape_mut(&mut self, id: NodeId) -> Option<&mut Shape> {
        let Scene { nodes, revs, .. } = self;
        let node = find_in_mut(nodes, id)?;
        if !matches!(node.kind, NodeKind::Shape(_)) {
            return None;
        }
        let kinds = Changes::GEOMETRY | Changes::PAINT;
        let stamp = node.revs.bump(kinds);
        revs.set(kinds, stamp, Some(id));
        match &mut node.kind {
            NodeKind::Shape(shape) => Some(shape),
            _ => None,
        }
    }

    /// What the scene looked like, as stamps, to be compared later with [`Scene::changed_since`]. Equal stamps mean
    /// equal content, in this scene and in copies of it.
    pub fn revision(&self) -> Revs {
        self.revs.0.revs()
    }

    /// The kinds of change made anywhere in the scene since `before` was taken from [`Scene::revision`]. Empty when
    /// nothing changed, in constant time.
    pub fn changed_since(&self, before: &Revs) -> Changes {
        self.revs.changed_since(before)
    }

    /// The name of this scene's journal of changes: a copy of the scene has another. Pass it along with a [`Scene::revision`]
    /// to be able to ask [`Scene::nodes_changed_since`] later.
    pub fn journal_epoch(&self) -> Stamp {
        self.revs.epoch()
    }

    /// Which nodes were changed since `before` (taken from [`Scene::revision`] of this same scene, the same
    /// [`Scene::journal_epoch`]), and what kinds of change each got. A change that was not to one node (the list of children, the
    /// symbols) comes as node 0. `None` when the journal does not reach back that far: look at everything then.
    pub fn nodes_changed_since(&self, before: &Revs) -> Option<Vec<(NodeId, Changes)>> {
        self.revs.nodes_changed_since(before)
    }

    /// True if everything that changed since `before` (taken from this scene's [`Scene::revision`] when its journal was named `epoch`)
    /// was done to cameras: nodes that draw nothing and have no children, so the picture of the scene is what it was. False when
    /// something else changed, or when the journal cannot say (another scene, a journal that does not reach back that far).
    pub fn only_cameras_changed_since(&self, before: &Revs, epoch: Stamp) -> bool {
        if self.journal_epoch() != epoch {
            return false;
        }
        let Some(changed) = self.nodes_changed_since(before) else { return false };
        changed.iter().all(|&(id, kinds)| {
            id.0 != 0 && !kinds.intersects(Changes::STRUCTURE) && self.find(id).is_some_and(|node| matches!(node.kind, NodeKind::Camera(_)) && node.children().is_empty())
        })
    }

    /// Records a change that was made to the scene as a whole (the list of nodes or symbols, say).
    pub(crate) fn touch(&mut self, kinds: Changes) {
        self.revs.bump(kinds, None);
    }

    /// Records a change to the node `id` itself, made by model code that holds the node without a [`NodeMut`].
    pub(crate) fn touch_node(&mut self, id: Option<NodeId>, kinds: Changes) {
        let stamp = self.revs.bump(kinds, id);
        if let Some(node) = id.and_then(|id| find_in_mut(&mut self.nodes, id)) {
            node.revs.set(kinds, stamp);
        }
    }

    /// True if the two scenes hold the same content, as `==` would say, but without looking at every node when their
    /// stamps already prove that the nodes and symbols are the same (which is almost always, between two frames).
    pub fn same_content(&self, other: &Scene) -> bool {
        if self.revs.revs() != other.revs.revs() {
            return self == other;
        }
        self.name == other.name
            && self.clips == other.clips
            && self.ik == other.ik
            && self.next_id == other.next_id
            && self.legacy_size == other.legacy_size
            && self.legacy_background == other.legacy_background
    }

    /// All node ids in document order (parents before children, bottom to top).
    /// Calls `visit` for every node, parents before their children, in document order (the order of [`Scene::all_ids`]).
    /// Per-frame code that looks at every node should use this and not `all_ids` followed by `find` for each id: that
    /// searches the tree once per node, and took 400 ms for a scene of 21,000 shapes.
    pub fn for_each_node<'a>(&'a self, mut visit: impl FnMut(&'a Node)) {
        fn walk<'a>(nodes: &'a [Node], visit: &mut impl FnMut(&'a Node)) {
            for node in nodes {
                visit(node);
                walk(node.children(), visit);
            }
        }
        walk(&self.nodes, &mut visit);
    }

    pub fn all_ids(&self) -> Vec<NodeId> {
        let mut ids = Vec::new();
        collect_ids(&self.nodes, &mut ids);
        ids
    }

    /// The first object (in document order) with this name.
    pub fn find_by_name(&self, name: &str) -> Option<NodeId> {
        self.all_ids().into_iter().find(|&id| self.find(id).is_some_and(|n| n.name == name))
    }

    /// Ancestors of `id` from the top level down, ending with `id`.
    pub fn ancestry(&self, id: NodeId) -> Option<Vec<NodeId>> {
        path_in(&self.nodes, id)
    }

    pub fn is_descendant(&self, ancestor: NodeId, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|chain| chain.len() > 1 && chain[..chain.len() - 1].contains(&ancestor))
    }

    /// Parent id (`None` for top level) and index among siblings.
    pub fn locate(&self, id: NodeId) -> Option<(Option<NodeId>, usize)> {
        let chain = self.ancestry(id)?;
        let parent = (chain.len() > 1).then(|| chain[chain.len() - 2]);
        let siblings = match parent {
            Some(p) => self.find(p)?.children(),
            None => &self.nodes[..],
        };
        Some((parent, siblings.iter().position(|n| n.id == id)?))
    }

    /// Transform from the node's local space to world space.
    pub fn world_transform(&self, id: NodeId) -> Option<Affine2> {
        let chain = self.ancestry(id)?;
        Some(chain.iter().filter_map(|&n| self.find(n)).fold(Affine2::IDENTITY, |acc, n| acc * n.transform))
    }

    /// World transform of the node's parent (identity for top-level nodes).
    pub fn parent_world(&self, id: NodeId) -> Affine2 {
        match self.locate(id) {
            Some((Some(parent), _)) => self.world_transform(parent).unwrap_or(Affine2::IDENTITY),
            _ => Affine2::IDENTITY,
        }
    }

    /// Axis-aligned world-space bounds of the geometry below `id`.
    pub fn world_bounds(&self, id: NodeId) -> Option<Rect> {
        node_bounds(self, self.find(id)?, self.world_transform(id)?)
    }

    /// Topmost visible, unlocked shape under `p` (world space). Returns the chain of
    /// ancestors, outermost first, ending with the shape.
    pub fn hit_test(&self, p: Vec2, tolerance: f32) -> Option<Vec<NodeId>> {
        let mut chain = Vec::new();
        let mut boxes = self.hits.boxes.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        hit_in(self, &self.nodes, Affine2::IDENTITY, p, tolerance, &mut chain, Some(&mut boxes)).then_some(chain)
    }

    /// [`Scene::hit_test`] without the boxes that let it skip far shapes: every shape is tested exactly. The reference for tests.
    #[cfg(test)]
    pub(crate) fn hit_test_exact(&self, p: Vec2, tolerance: f32) -> Option<Vec<NodeId>> {
        let mut chain = Vec::new();
        hit_in(self, &self.nodes, Affine2::IDENTITY, p, tolerance, &mut chain, None).then_some(chain)
    }

    /// Selected ids in document order, without nodes whose ancestor is also selected.
    /// One walk through the tree, so it stays fast with tens of thousands of selected objects.
    pub fn top_level_selection(&self, ids: &[NodeId]) -> Vec<NodeId> {
        fn walk(nodes: &[Node], selected: &HashSet<NodeId>, out: &mut Vec<NodeId>) {
            for node in nodes {
                if selected.contains(&node.id) {
                    out.push(node.id); // everything below it is covered by it
                } else {
                    walk(node.children(), selected, out);
                }
            }
        }
        let selected: HashSet<NodeId> = ids.iter().copied().collect();
        let mut out = Vec::new();
        walk(&self.nodes, &selected, &mut out);
        out
    }

    /// World-space bounds of the top-level objects among `ids`, in one walk through the tree.
    pub fn selection_bounds(&self, ids: &[NodeId]) -> Option<Rect> {
        fn walk(scene: &Scene, nodes: &[Node], parent: Affine2, selected: &HashSet<NodeId>, bounds: &mut Option<Rect>) {
            for node in nodes {
                let world = parent * node.transform;
                if selected.contains(&node.id) {
                    if let Some(found) = node_bounds(scene, node, world) {
                        *bounds = Some(bounds.map_or(found, |sum| sum.union(&found)));
                    }
                } else {
                    walk(scene, node.children(), world, selected, bounds);
                }
            }
        }
        let selected: HashSet<NodeId> = ids.iter().copied().collect();
        let mut bounds = None;
        walk(self, &self.nodes, Affine2::IDENTITY, &selected, &mut bounds);
        bounds
    }

    /// Drops the ids that are no longer in the scene (after a delete, say).
    pub fn retain_existing(&self, ids: &mut Vec<NodeId>) {
        if ids.len() <= 8 {
            ids.retain(|&id| self.find(id).is_some());
            return;
        }
        let existing: HashSet<NodeId> = self.all_ids().into_iter().collect();
        ids.retain(|id| existing.contains(id));
    }

    // --- Editing -------------------------------------------------------------------------------

    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        let (parent, index) = self.locate(id)?;
        let siblings = match parent {
            Some(p) => match &mut find_in_mut(&mut self.nodes, p)?.kind {
                NodeKind::Group(children) => children,
                NodeKind::Shape(_) | NodeKind::Instance(_) | NodeKind::Camera(_) => return None,
            },
            None => &mut self.nodes,
        };
        let node = siblings.remove(index);
        self.touch_node(parent, Changes::STRUCTURE);
        Some(node)
    }

    /// Removes the nodes `ids` name, with what is inside them, in one walk through the tree: the same scene as calling [`Scene::remove`]
    /// for each, but linear in the size of the scene instead of a search and a shift of the sibling list per node (4,000 of 21,200
    /// nodes took 1.3 s). Ids that are not in the scene are skipped, and a node inside another listed node goes with it. Returns how
    /// many nodes were taken out of their parents (not counting those that went with an outer one). Every parent that lost a child is
    /// stamped `STRUCTURE`, once.
    pub fn remove_all(&mut self, ids: &[NodeId]) -> usize {
        self.take_all(ids).len()
    }

    /// Like [`Scene::remove_all`], but hands the nodes taken out to the caller: those taken out of their parents, level by level (not
    /// in document order across levels; see `group`, which puts them in order).
    pub(crate) fn take_all(&mut self, ids: &[NodeId]) -> Vec<Node> {
        if ids.is_empty() {
            return Vec::new();
        }
        fn take_in(nodes: &mut Vec<Node>, doomed: &HashSet<NodeId>, parent: Option<NodeId>, changed: &mut Vec<Option<NodeId>>, taken: &mut Vec<Node>) {
            let before = taken.len();
            taken.extend(nodes.extract_if(.., |node| doomed.contains(&node.id)));
            if taken.len() > before {
                changed.push(parent);
            }
            for node in nodes.iter_mut() {
                let id = node.id;
                if let NodeKind::Group(children) = &mut node.kind {
                    take_in(children, doomed, Some(id), changed, taken);
                }
            }
        }
        let doomed: HashSet<NodeId> = ids.iter().copied().collect();
        let (mut changed, mut taken) = (Vec::new(), Vec::new());
        take_in(&mut self.nodes, &doomed, None, &mut changed, &mut taken);
        for parent in changed {
            self.touch_node(parent, Changes::STRUCTURE);
        }
        taken
    }

    /// The world transform of each of `ids` that is in the scene, found in one walk through the tree.
    fn worlds_of(&self, ids: &[NodeId]) -> std::collections::HashMap<NodeId, Affine2> {
        fn walk(nodes: &[Node], parent: Affine2, wanted: &HashSet<NodeId>, found: &mut std::collections::HashMap<NodeId, Affine2>) {
            for node in nodes {
                let world = parent * node.transform;
                if wanted.contains(&node.id) {
                    found.insert(node.id, world);
                }
                walk(node.children(), world, wanted, found);
            }
        }
        let wanted: HashSet<NodeId> = ids.iter().copied().collect();
        let mut found = std::collections::HashMap::new();
        walk(&self.nodes, Affine2::IDENTITY, &wanted, &mut found);
        found
    }

    /// Re-parents `id` under `new_parent` at `index` (counted after removal),
    /// keeping its on-screen position. Returns false if the move is not allowed.
    pub fn move_node(&mut self, id: NodeId, new_parent: Option<NodeId>, index: usize) -> bool {
        if let Some(p) = new_parent {
            let target_is_group = self.find(p).is_some_and(Node::is_group);
            if p == id || self.is_descendant(id, p) || !target_is_group {
                return false;
            }
        }
        let Some(world) = self.world_transform(id) else { return false };
        let Some(mut node) = self.remove(id) else { return false };
        let target_world = new_parent.and_then(|p| self.world_transform(p)).unwrap_or(Affine2::IDENTITY);
        node.transform = target_world.inverse() * world;
        node.revs.bump(Changes::TRANSFORM);
        self.touch(Changes::TRANSFORM);
        self.insert(node, new_parent, index);
        true
    }

    /// Moves a node one step towards the top (`up`) or bottom among its siblings.
    pub fn shift_order(&mut self, id: NodeId, up: bool) {
        let Some((parent, index)) = self.locate(id) else { return };
        let count = match parent {
            Some(p) => self.find(p).map_or(0, |n| n.children().len()),
            None => self.nodes.len(),
        };
        let target = if up { index + 1 } else { index.wrapping_sub(1) };
        if target < count {
            if let Some(node) = self.remove(id) {
                self.insert(node, parent, target);
            }
        }
    }

    pub fn group(&mut self, ids: &[NodeId]) -> Option<NodeId> {
        let ordered = self.top_level_selection(ids);
        let (parent, index) = self.locate(*ordered.first()?)?;
        let group = self.new_node("Group", NodeKind::Group(Vec::new()));
        let group_id = group.id;
        self.insert(group, parent, index);
        // Every node keeps its place on the screen: its world transform, found for all of them in one walk (moving them one at a time
        // searched the tree for each), becomes a transform relative to the group, which is where the first of them was.
        let worlds = self.worlds_of(&ordered);
        let group_world = self.world_transform(group_id).unwrap_or(Affine2::IDENTITY);
        let inverse = group_world.inverse();
        let position: std::collections::HashMap<NodeId, usize> = ordered.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let mut taken = self.take_all(&ordered);
        taken.sort_by_key(|node| position[&node.id]);
        for node in &mut taken {
            node.transform = inverse * worlds[&node.id];
            node.revs.bump(Changes::TRANSFORM);
        }
        self.touch(Changes::TRANSFORM);
        if let Some(Node { kind: NodeKind::Group(children), .. }) = find_in_mut(&mut self.nodes, group_id) {
            *children = taken;
        }
        self.touch_node(Some(group_id), Changes::STRUCTURE);
        Some(group_id)
    }

    /// [`Scene::group`] by moving the nodes one at a time: the reference for tests.
    #[cfg(test)]
    pub(crate) fn group_one_by_one(&mut self, ids: &[NodeId]) -> Option<NodeId> {
        let ordered = self.top_level_selection(ids);
        let (parent, index) = self.locate(*ordered.first()?)?;
        let group = self.new_node("Group", NodeKind::Group(Vec::new()));
        let group_id = group.id;
        self.insert(group, parent, index);
        for id in ordered {
            self.move_node(id, Some(group_id), usize::MAX);
        }
        Some(group_id)
    }

    /// Dissolves a group, keeping its children in place. Returns the freed children.
    pub fn ungroup(&mut self, id: NodeId) -> Vec<NodeId> {
        let Some((parent, index)) = self.locate(id) else { return Vec::new() };
        let Some(Node { kind: NodeKind::Group(children), transform, .. }) = self.remove(id) else {
            return Vec::new();
        };
        let mut freed = Vec::new();
        for (offset, mut child) in children.into_iter().enumerate() {
            child.transform = transform * child.transform;
            child.revs.bump(Changes::TRANSFORM);
            self.touch(Changes::TRANSFORM);
            freed.push(child.id);
            self.insert(child, parent, index + offset);
        }
        freed
    }

    /// Deep-copies a node (with fresh ids) and places the copy right above the original.
    pub fn duplicate(&mut self, id: NodeId) -> Option<NodeId> {
        let (parent, index) = self.locate(id)?;
        let copy = self.clone_with_new_ids(self.find(id)?.clone());
        let copy_id = copy.id;
        self.insert(copy, parent, index + 1);
        Some(copy_id)
    }

    pub(crate) fn clone_with_new_ids(&mut self, mut node: Node) -> Node {
        node.id = NodeId(self.next_id);
        self.next_id += 1;
        if let NodeKind::Group(children) = node.kind {
            let copies = children.into_iter().map(|child| self.clone_with_new_ids(child)).collect();
            node.kind = NodeKind::Group(copies);
        }
        node
    }

    /// Merges several shapes into one compound path that takes the lowest shape's style.
    pub fn combine(&mut self, ids: &[NodeId]) -> Option<NodeId> {
        let shapes = self.selected_shapes_in_order(ids);
        let (&target, others) = shapes.split_first()?;
        if others.is_empty() {
            return None;
        }
        let to_target_space = self.world_transform(target)?.inverse();
        let mut merged = Vec::new();
        for &id in others {
            let relative = to_target_space * self.world_transform(id)?;
            if let Some(Node { kind: NodeKind::Shape(shape), .. }) = self.find(id) {
                merged.extend(shape.path.transformed(&relative).subpaths);
            }
        }
        for &id in others {
            self.remove(id);
        }
        if let Some(mut node) = self.find_mut(target) {
            if let NodeKind::Shape(shape) = &mut node.kind {
                shape.path.subpaths.extend(merged);
                shape.fill_rule = FillRule::EvenOdd;
            }
        }
        Some(target)
    }

    /// Combines the selected shapes with a boolean operation. The lowest shape is the subject
    /// (and keeps its style); the shapes above it are applied in order.
    pub fn boolean(&mut self, ids: &[NodeId], op: BoolOp) -> Option<NodeId> {
        let shapes = self.selected_shapes_in_order(ids);
        let (&target, others) = shapes.split_first()?;
        if others.is_empty() {
            return None;
        }
        let to_target_space = self.world_transform(target)?.inverse();
        let Some(NodeKind::Shape(subject)) = self.find(target).map(|n| &n.kind) else { return None };
        let (mut result, mut rule) = (subject.path.clone(), subject.fill_rule);
        for &id in others {
            let relative = to_target_space * self.world_transform(id)?;
            let Some(NodeKind::Shape(other)) = self.find(id).map(|n| &n.kind) else { return None };
            result = boolean::combine(&result, rule, &other.path.transformed(&relative), other.fill_rule, op);
            rule = FillRule::EvenOdd; // results carry their holes as separate contours
        }
        for &id in others {
            self.remove(id);
        }
        if let Some(mut node) = self.find_mut(target) {
            if let NodeKind::Shape(shape) = &mut node.kind {
                shape.path = result;
                shape.fill_rule = FillRule::EvenOdd;
            }
        }
        Some(target)
    }

    /// Splits a compound path into one shape per subpath, each keeping the original style.
    /// Returns the ids of all resulting shapes (the original comes first).
    pub fn release_compound(&mut self, id: NodeId) -> Vec<NodeId> {
        let Some((parent, index)) = self.locate(id) else { return Vec::new() };
        let Some(original) = self.find(id).cloned() else { return Vec::new() };
        let NodeKind::Shape(shape) = &original.kind else { return Vec::new() };
        if shape.path.subpaths.len() < 2 {
            return vec![id];
        }
        let mut released = vec![id];
        for (offset, sub) in shape.path.subpaths.iter().enumerate().skip(1) {
            let mut copy = self.clone_with_new_ids(original.clone());
            if let NodeKind::Shape(s) = &mut copy.kind {
                s.path.subpaths = vec![sub.clone()];
            }
            copy.revs.bump(Changes::GEOMETRY);
            released.push(copy.id);
            self.insert(copy, parent, index + offset);
        }
        if let Some(mut node) = self.find_mut(id) {
            if let NodeKind::Shape(s) = &mut node.kind {
                s.path.subpaths.truncate(1);
            }
        }
        released
    }

    /// Moves a node's origin (the pivot of rotation and scale) to the center of its bounds without
    /// changing how it looks. Instances are already centered.
    pub fn center_origin(&mut self, id: NodeId) -> bool {
        let Some(world) = self.world_transform(id) else { return false };
        let Some(world_center) = self.world_bounds(id).map(|b| b.center()) else { return false };
        let to_local = world.inverse();
        let center = to_local.transform_point2(world_center);
        let shift = Affine2::from_translation(-center);
        let Some(mut node) = self.find_mut(id) else { return false };
        match &mut node.kind {
            // A skin is bound to the outline's current coordinates, so skinned shapes keep their origin.
            NodeKind::Shape(shape) if shape.skin.is_some() => return false,
            NodeKind::Shape(shape) => {
                shape.path = shape.path.transformed(&shift);
                // Gradients live in the same local space, so they move with the path.
                let paints = [shape.fill.as_mut(), shape.stroke.as_mut().map(|s| &mut s.paint)];
                for paint in paints.into_iter().flatten() {
                    if let Paint::Gradient(gradient) = paint {
                        gradient.transform = shift * gradient.transform;
                    }
                }
            }
            NodeKind::Group(children) => children.iter_mut().for_each(|c| {
                c.transform = shift * c.transform;
                c.revs.bump(Changes::TRANSFORM);
            }),
            NodeKind::Instance(_) | NodeKind::Camera(_) => return false,
        }
        node.transform *= Affine2::from_translation(center);
        true
    }

    /// Groups the selection and makes the top-most object a clipping mask for the rest.
    /// The top-most object must be a shape.
    pub fn make_clip_group(&mut self, ids: &[NodeId]) -> Option<NodeId> {
        let ordered = self.top_level_selection(ids);
        let top = self.find(*ordered.last()?)?;
        if ordered.len() < 2 || !matches!(top.kind, NodeKind::Shape(_)) {
            return None;
        }
        let group = self.group(ids)?;
        let mut node = self.find_mut(group)?;
        node.clip = true;
        node.name = "Clip Group".into();
        Some(group)
    }

    fn selected_shapes_in_order(&self, ids: &[NodeId]) -> Vec<NodeId> {
        self.top_level_selection(ids)
            .into_iter()
            .filter(|&id| matches!(self.find(id).map(|n| &n.kind), Some(NodeKind::Shape(_))))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(scene: &mut Scene, at: Vec2) -> NodeId {
        let mut node = scene.new_node("square", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)))));
        node.transform = Affine2::from_translation(at);
        scene.add(node, None)
    }

    /// The definition `top_level_selection` replaced, which took cubic time: a node is dropped
    /// when any other selected node is its ancestor.
    fn top_level_selection_by_definition(scene: &Scene, ids: &[NodeId]) -> Vec<NodeId> {
        scene
            .all_ids()
            .into_iter()
            .filter(|id| ids.contains(id))
            .filter(|&id| !ids.iter().any(|&other| other != id && scene.is_descendant(other, id)))
            .collect()
    }

    /// A random tree of groups and moved, scaled shapes, plus a random generator for picking from it.
    fn random_scene(round: usize, random: &mut impl FnMut(usize) -> usize) -> Scene {
        let mut scene = Scene::default();
        let mut groups: Vec<Option<NodeId>> = vec![None];
        for _ in 0..(10 + round * 3) {
            let parent = groups[random(groups.len())];
            let make_group = random(3) == 0;
            let kind = if make_group { NodeKind::Group(Vec::new()) } else { NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(5.0 + random(10) as f32)))) };
            let mut node = scene.new_node("n", kind);
            node.transform = Affine2::from_scale_angle_translation(Vec2::splat(0.5 + random(4) as f32 * 0.5), random(7) as f32 * 0.3, Vec2::new(random(200) as f32, random(200) as f32));
            let id = scene.add(node, parent);
            if make_group {
                groups.push(Some(id));
            }
        }
        scene
    }

    fn random_selection(all: &[NodeId], random: &mut impl FnMut(usize) -> usize) -> Vec<NodeId> {
        // In random order, with repeated ids and one id that is not in the scene.
        let mut ids: Vec<NodeId> = (0..random(all.len() + 1)).map(|_| all[random(all.len())]).collect();
        ids.push(NodeId(u64::MAX));
        ids
    }

    fn generator() -> impl FnMut(usize) -> usize {
        let mut seed = 99_u32;
        move |below| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as usize % below
        }
    }

    #[test]
    fn top_level_selection_matches_its_definition_on_random_trees() {
        let mut random = generator();
        for round in 0..30 {
            let scene = random_scene(round, &mut random);
            let all = scene.all_ids();
            for _ in 0..20 {
                let ids = random_selection(&all, &mut random);
                assert_eq!(scene.top_level_selection(&ids), top_level_selection_by_definition(&scene, &ids), "round {round}, selection {ids:?}");
            }
        }
    }

    #[test]
    fn visiting_every_node_goes_in_the_order_of_all_ids_on_random_trees() {
        let mut random = generator();
        for round in 0..30 {
            let scene = random_scene(round, &mut random);
            let mut visited = Vec::new();
            scene.for_each_node(|node| visited.push(node.id));
            assert_eq!(visited, scene.all_ids(), "round {round}");
        }
    }

    #[test]
    fn selection_bounds_and_pruning_match_the_one_by_one_versions_on_random_trees() {
        let mut random = generator();
        for round in 0..30 {
            let scene = random_scene(round, &mut random);
            let all = scene.all_ids();
            for _ in 0..20 {
                let ids = random_selection(&all, &mut random);
                let one_by_one = top_level_selection_by_definition(&scene, &ids).into_iter().filter_map(|id| scene.world_bounds(id)).reduce(|a, b| a.union(&b));
                assert_eq!(scene.selection_bounds(&ids), one_by_one, "round {round}, selection {ids:?}");

                let mut pruned = ids.clone();
                scene.retain_existing(&mut pruned);
                let expected: Vec<NodeId> = ids.iter().copied().filter(|&id| scene.find(id).is_some()).collect();
                assert_eq!(pruned, expected, "round {round}");
            }
        }
    }

    #[test]
    fn hit_test_picks_topmost() {
        let mut scene = Scene::default();
        let _bottom = square(&mut scene, Vec2::ZERO);
        let top = square(&mut scene, Vec2::new(5.0, 5.0));
        assert_eq!(scene.hit_test(Vec2::new(7.0, 7.0), 1.0), Some(vec![top]));
        assert_eq!(scene.hit_test(Vec2::new(100.0, 100.0), 1.0), None);
    }

    #[test]
    fn grouping_preserves_world_position() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::new(20.0, 0.0));
        let group = scene.group(&[a]).unwrap();
        scene.find_mut(group).unwrap().transform = Affine2::from_translation(Vec2::new(100.0, 0.0));
        scene.ungroup(group);
        let world = scene.world_transform(a).unwrap();
        assert_eq!(world.transform_point2(Vec2::ZERO), Vec2::new(120.0, 0.0));
    }

    #[test]
    fn cannot_move_group_into_itself() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let group = scene.group(&[a]).unwrap();
        let outer = scene.group(&[group]).unwrap();
        assert!(!scene.move_node(outer, Some(group), 0));
    }

    #[test]
    fn centering_the_origin_keeps_the_look() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::new(40.0, 20.0)); // covers 40..50 x 20..30
        let before = scene.world_bounds(a).unwrap();
        assert!(scene.center_origin(a));
        assert_eq!(scene.world_bounds(a).unwrap(), before);
        // The origin now sits at the middle of the shape.
        assert_eq!(scene.world_transform(a).unwrap().transform_point2(Vec2::ZERO), before.center());
    }

    #[test]
    fn boolean_union_replaces_shapes_with_one() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let b = square(&mut scene, Vec2::new(5.0, 0.0));
        assert_eq!(scene.boolean(&[a, b], BoolOp::Union), Some(a));
        assert_eq!(scene.nodes.len(), 1);
        assert!(scene.hit_test(Vec2::new(14.0, 5.0), 0.5).is_some());
    }

    #[test]
    fn release_compound_gives_one_shape_per_subpath() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let b = square(&mut scene, Vec2::new(50.0, 0.0));
        scene.combine(&[a, b]);
        assert_eq!(scene.release_compound(a).len(), 2);
        assert_eq!(scene.nodes.len(), 2);
    }

    #[test]
    fn clip_group_only_hits_inside_the_mask() {
        let mut scene = Scene::default();
        let content = square(&mut scene, Vec2::ZERO); // 0..10
        let mask = {
            let shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(4.0)));
            let node = scene.new_node("mask", NodeKind::Shape(shape));
            scene.add(node, None)
        };
        let group = scene.make_clip_group(&[content, mask]).unwrap();
        assert_eq!(scene.hit_test(Vec2::new(2.0, 2.0), 0.5).map(|c| c[0]), Some(group));
        assert_eq!(scene.hit_test(Vec2::new(8.0, 8.0), 0.5), None);
        assert_eq!(scene.world_bounds(group).unwrap().max, Vec2::splat(4.0));
    }

    #[test]
    fn combine_merges_subpaths() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let b = square(&mut scene, Vec2::new(50.0, 0.0));
        scene.combine(&[a, b]);
        assert_eq!(scene.nodes.len(), 1);
        match &scene.find(a).unwrap().kind {
            NodeKind::Shape(shape) => assert_eq!(shape.path.subpaths.len(), 2),
            _ => panic!("expected a shape"),
        }
    }
}
