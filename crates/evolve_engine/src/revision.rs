//! Change tracking: knowing cheaply what changed in a scene since a renderer, the undo history or a picking index last
//! looked at it. The plan and its reasons are in `GPU_RENDERER_PLAN.md` (Phase 1, decision D7).
//!
//! **Stamps.** A [`Stamp`] names one version of some content. Every change takes a new stamp from a counter shared by
//! the whole process, so two different versions never carry the same stamp, not even in two copies of a scene that
//! were changed separately. Copies of a scene carry the same stamps as the original until one of them changes. So:
//! *the same stamp means the same content*, and a cache can be keyed by it. Undo needs no special care: restoring an
//! old snapshot restores its old stamps.
//!
//! **Kinds of change.** What changed decides what a renderer must redo, so each node keeps one stamp per kind
//! ([`Revs`]). The scene keeps the same five stamps for "anything in the scene of this kind changed", so a quiet frame
//! is known in O(1) and a frame where only transforms changed can skip re-building geometry.
//!
//! | kind | covers | a renderer redoes |
//! |---|---|---|
//! | [`Changes::GEOMETRY`] | a shape's path, fill rule, skin, the presence of fill and stroke, whether the stroke has a width (above 0), its join, cap and miter limit, a camera's size | tessellation |
//! | [`Changes::TRANSFORM`] | the node's own transform | its row (and its descendants' world matrices) |
//! | [`Changes::PAINT`] | fill and stroke paints, the width of a stroke that has one, opacity, visibility, an instance's color overrides, a camera's background | its row |
//! | [`Changes::STRUCTURE`] | which children a group has and their order, the clip flag, the kind of node, an instance's symbol, the list of symbols | the draw order and chunks |
//! | [`Changes::DATA`] | name, locked, script, body, bone settings: nothing that is drawn | nothing |
//!
//! **How writes are tracked.** [`Scene::find_mut`](crate::scene::Scene::find_mut) returns a [`NodeMut`]. Reading
//! through it is free. Writing a field directly (`node.name = ...`) goes through `DerefMut`, which assumes the worst
//! and stamps every kind. That is always correct and costs one tessellation of that one node. Code that runs every
//! frame on many nodes (physics, scripts, animation, dragging) uses the precise methods instead (`transform_mut`,
//! `set_opacity`, ...), which stamp only what they change.
//!
//! A skinned shape is drawn bent by bones that are other nodes, so its picture also depends on their transform
//! stamps; renderers treat skinned shapes as dynamic.
//!
//! **Which nodes changed.** The scene's stamps say that something of some kind changed, not what. A renderer that has drawn a
//! scene and finds one node moved should not walk 21,000 nodes to find it, so the scene also keeps a short journal: every
//! stamp it hands out is written down with the node that got it ([`SceneRevs::nodes_changed_since`]). A change that is not
//! to one node (the list of children, the symbols) is written down as node 0, "unknown", and the reader then looks at
//! everything. The journal is bounded, and a copy of a scene starts with an empty one and a name of its own
//! ([`SceneRevs::epoch`]): a reader that was told what changed in one scene must never be told it of another (undo restores a
//! copy).

use crate::geometry::Affine2;
use crate::path::Path;
use crate::physics::Body;
use crate::rig::Bone;
use crate::scene::{Node, NodeId, NodeKind};
use crate::style::{Color, Paint};
use std::ops::{BitOr, Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAMP: AtomicU64 = AtomicU64::new(1);

/// One version of some content. See the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Stamp(u64);

impl Stamp {
    /// A stamp no other version has had or will have in this process.
    pub fn fresh() -> Stamp {
        Stamp(NEXT_STAMP.fetch_add(1, Ordering::Relaxed))
    }
}

/// A set of kinds of change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes(u8);

impl Changes {
    pub const NONE: Changes = Changes(0);
    pub const GEOMETRY: Changes = Changes(1);
    pub const TRANSFORM: Changes = Changes(2);
    pub const PAINT: Changes = Changes(4);
    pub const STRUCTURE: Changes = Changes(8);
    pub const DATA: Changes = Changes(16);
    pub const ALL: Changes = Changes(31);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True if every kind in `other` is in `self`.
    pub fn contains(self, other: Changes) -> bool {
        self.0 & other.0 == other.0
    }

    /// True if `self` and `other` have a kind in common.
    pub fn intersects(self, other: Changes) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Changes {
    type Output = Changes;

    fn bitor(self, other: Changes) -> Changes {
        Changes(self.0 | other.0)
    }
}

/// The five stamps of a node, or of a whole scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revs {
    pub geometry: Stamp,
    pub transform: Stamp,
    pub paint: Stamp,
    pub structure: Stamp,
    pub data: Stamp,
}

impl Default for Revs {
    /// New content gets a stamp of its own. (Also what a node or scene read from a file starts with.)
    fn default() -> Self {
        let born = Stamp::fresh();
        Self { geometry: born, transform: born, paint: born, structure: born, data: born }
    }
}

impl Revs {
    /// The newest of the five stamps: when the last change was made (stamps only go up).
    pub fn latest(&self) -> Stamp {
        self.geometry.max(self.transform).max(self.paint).max(self.structure).max(self.data)
    }

    /// Gives every kind in `kinds` a fresh stamp and returns it.
    pub fn bump(&mut self, kinds: Changes) -> Stamp {
        let stamp = Stamp::fresh();
        self.set(kinds, stamp);
        stamp
    }

    pub(crate) fn set(&mut self, kinds: Changes, stamp: Stamp) {
        for (kind, slot) in [
            (Changes::GEOMETRY, &mut self.geometry),
            (Changes::TRANSFORM, &mut self.transform),
            (Changes::PAINT, &mut self.paint),
            (Changes::STRUCTURE, &mut self.structure),
            (Changes::DATA, &mut self.data),
        ] {
            if kinds.contains(kind) {
                *slot = stamp;
            }
        }
    }

    /// The kinds whose stamp differs from `before`: what changed between two looks at the same content.
    pub fn changed_since(&self, before: &Revs) -> Changes {
        let mut changes = Changes::NONE;
        for (kind, now, then) in [
            (Changes::GEOMETRY, self.geometry, before.geometry),
            (Changes::TRANSFORM, self.transform, before.transform),
            (Changes::PAINT, self.paint, before.paint),
            (Changes::STRUCTURE, self.structure, before.structure),
            (Changes::DATA, self.data, before.data),
        ] {
            if now != then {
                changes = changes | kind;
            }
        }
        changes
    }
}

/// Bookkeeping about content rather than content: serialization skips it and two values are always equal whatever it
/// holds, so that comparing nodes or scenes (as the undo history does) compares only what they contain.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meta<T>(pub T);

impl<T> PartialEq for Meta<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Deref for Meta<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Meta<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Entries the journal keeps; when it is full the older half is dropped.
const JOURNAL_ENTRIES: usize = 1 << 15;

/// The stamps of a scene and the journal of which nodes got them (see the module documentation).
#[derive(Debug)]
pub struct SceneRevs {
    revs: Revs,
    /// Names this scene's journal: another scene (a copy) has another.
    epoch: Stamp,
    /// Changes at or before this stamp are not in the journal (never written, or dropped).
    floor: Stamp,
    /// (stamp, node id or 0 for "not one node", kinds), with stamps going up.
    entries: Vec<(Stamp, u64, Changes)>,
}

impl Default for SceneRevs {
    fn default() -> Self {
        SceneRevs { revs: Revs::default(), epoch: Stamp::fresh(), floor: Stamp(0), entries: Vec::new() }
    }
}

impl Clone for SceneRevs {
    /// The copy has the same stamps and the same content, but its history from here on is its own: an empty journal, and a name
    /// of its own, so that nobody takes it for the scene it was copied from.
    fn clone(&self) -> Self {
        SceneRevs { revs: self.revs, epoch: Stamp::fresh(), floor: self.revs.latest(), entries: Vec::new() }
    }
}

impl SceneRevs {
    /// The five stamps.
    pub fn revs(&self) -> Revs {
        self.revs
    }

    /// The name of this scene's journal.
    pub fn epoch(&self) -> Stamp {
        self.epoch
    }

    /// Gives every kind in `kinds` a fresh stamp, written down for `node` (`None`: not one node), and returns it.
    pub(crate) fn bump(&mut self, kinds: Changes, node: Option<NodeId>) -> Stamp {
        let stamp = Stamp::fresh();
        self.set(kinds, stamp, node);
        stamp
    }

    pub(crate) fn set(&mut self, kinds: Changes, stamp: Stamp, node: Option<NodeId>) {
        self.revs.set(kinds, stamp);
        if self.entries.len() >= JOURNAL_ENTRIES {
            let dropped: Vec<_> = self.entries.drain(..JOURNAL_ENTRIES / 2).collect();
            if let Some(last) = dropped.last() {
                self.floor = last.0;
            }
        }
        self.entries.push((stamp, node.map_or(0, |id| id.0), kinds));
    }

    /// The kinds of change since `before` (see [`Revs::changed_since`]).
    pub fn changed_since(&self, before: &Revs) -> Changes {
        self.revs.changed_since(before)
    }

    /// The nodes that were changed since the scene looked as `before` (a value of `revision()` taken earlier from *this* scene,
    /// see [`SceneRevs::epoch`]), with the kinds of change each got, by node id. A change that was not to one node is in the list as
    /// node 0. `None` when the journal does not reach back to `before`.
    pub fn nodes_changed_since(&self, before: &Revs) -> Option<Vec<(NodeId, Changes)>> {
        let since = before.latest();
        if since < self.floor {
            return None;
        }
        let first = self.entries.partition_point(|entry| entry.0 <= since);
        let mut found: Vec<(NodeId, Changes)> = Vec::new();
        let mut seen: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        for &(_, id, kinds) in &self.entries[first..] {
            match seen.get(&id) {
                Some(&at) => found[at].1 = found[at].1 | kinds,
                None => {
                    seen.insert(id, found.len());
                    found.push((NodeId(id), kinds));
                }
            }
        }
        Some(found)
    }
}

/// A node borrowed for changing, from [`Scene::find_mut`](crate::scene::Scene::find_mut). It records what is changed
/// in the node and in the scene it lives in. See the module documentation.
pub struct NodeMut<'a> {
    node: &'a mut Node,
    scene: &'a mut SceneRevs,
}

impl<'a> NodeMut<'a> {
    pub(crate) fn new(node: &'a mut Node, scene: &'a mut SceneRevs) -> Self {
        Self { node, scene }
    }

    fn touch(&mut self, kinds: Changes) {
        let stamp = self.node.revs.bump(kinds);
        self.scene.set(kinds, stamp, Some(self.node.id));
    }

    // --- Precise changes: they stamp only what they change --------------------------------------

    /// The node's transform, for changing. Stamps `TRANSFORM`.
    pub fn transform_mut(&mut self) -> &mut Affine2 {
        self.touch(Changes::TRANSFORM);
        &mut self.node.transform
    }

    pub fn set_transform(&mut self, transform: Affine2) {
        *self.transform_mut() = transform;
    }

    /// Stamps `PAINT`.
    pub fn opacity_mut(&mut self) -> &mut f32 {
        self.touch(Changes::PAINT);
        &mut self.node.opacity
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        *self.opacity_mut() = opacity;
    }

    /// Stamps `PAINT`.
    pub fn visible_mut(&mut self) -> &mut bool {
        self.touch(Changes::PAINT);
        &mut self.node.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        *self.visible_mut() = visible;
    }

    /// Stamps `DATA`.
    pub fn name_mut(&mut self) -> &mut String {
        self.touch(Changes::DATA);
        &mut self.node.name
    }

    /// Stamps `DATA`.
    pub fn locked_mut(&mut self) -> &mut bool {
        self.touch(Changes::DATA);
        &mut self.node.locked
    }

    /// Stamps `DATA`.
    pub fn set_body(&mut self, body: Option<Body>) {
        self.touch(Changes::DATA);
        self.node.body = body;
    }

    /// Stamps `DATA`.
    pub fn set_script(&mut self, script: Option<String>) {
        self.touch(Changes::DATA);
        self.node.script = script;
    }

    /// Whether a group clips its content to its top child. Stamps `STRUCTURE`.
    pub fn set_clip(&mut self, clip: bool) {
        self.touch(Changes::STRUCTURE);
        self.node.clip = clip;
    }

    /// Gives a camera's frame this size, if the node is a camera and the size is not the one it has. Stamps `GEOMETRY`, and only when
    /// the size changes: a script that sets the same size every frame leaves the scene as it was.
    pub fn set_camera_size(&mut self, size: crate::geometry::Vec2) {
        if !matches!(&self.node.kind, NodeKind::Camera(camera) if camera.size != size) {
            return;
        }
        self.touch(Changes::GEOMETRY);
        if let NodeKind::Camera(camera) = &mut self.node.kind {
            camera.size = size;
        }
    }

    /// A shape's path, for changing. Stamps `GEOMETRY`. `None` for other kinds of node.
    pub fn path_mut(&mut self) -> Option<&mut Path> {
        if !matches!(self.node.kind, NodeKind::Shape(_)) {
            return None;
        }
        self.touch(Changes::GEOMETRY);
        match &mut self.node.kind {
            NodeKind::Shape(shape) => Some(&mut shape.path),
            _ => None,
        }
    }

    /// A bone's settings, for changing (a bone's length shapes the skins bound to it). Stamps `GEOMETRY` and `DATA`.
    pub fn bone_mut(&mut self) -> Option<&mut Bone> {
        self.node.bone?;
        self.touch(Changes::GEOMETRY | Changes::DATA);
        self.node.bone.as_mut()
    }

    /// Makes a shape's fill the solid `color`, if it has a solid fill. Returns whether it did. Stamps `PAINT`.
    pub fn set_solid_fill_color(&mut self, color: Color) -> bool {
        if !matches!(&self.node.kind, NodeKind::Shape(shape) if matches!(shape.fill, Some(Paint::Solid(_)))) {
            return false;
        }
        self.touch(Changes::PAINT);
        if let NodeKind::Shape(shape) = &mut self.node.kind {
            shape.fill = Some(Paint::Solid(color));
        }
        true
    }

    /// Gives a shape's stroke this width, if it has a stroke and the width is a number that is not negative; returns whether it did. Stamps
    /// `PAINT` only: the width is in the shape's row for the renderer to read, and the triangles of a stroke do not depend on it (see
    /// `evolve_gpu::tess_cache` for the exceptions the renderer takes care of). A width that makes the stroke appear (from 0) or vanish
    /// (to 0) also stamps `GEOMETRY`, because there are then other triangles. Setting the width the stroke has changes nothing.
    pub fn set_stroke_width(&mut self, width: f32) -> bool {
        let Some(old) = (match &self.node.kind {
            NodeKind::Shape(shape) => shape.stroke.as_ref().map(|stroke| stroke.width),
            _ => None,
        }) else {
            return false;
        };
        if !width.is_finite() || width < 0.0 {
            return false;
        }
        if old == width {
            return true;
        }
        let visible = |w: f32| w.is_finite() && w > 0.0;
        self.touch(if visible(old) == visible(width) { Changes::PAINT } else { Changes::PAINT | Changes::GEOMETRY });
        if let NodeKind::Shape(shape) = &mut self.node.kind {
            if let Some(stroke) = shape.stroke.as_mut() {
                stroke.width = width;
            }
        }
        true
    }

    /// Makes a shape's stroke paint the solid `color`, if it has a stroke with a solid paint. Stamps `PAINT`.
    pub fn set_solid_stroke_color(&mut self, color: Color) -> bool {
        let solid = |node: &Node| matches!(&node.kind, NodeKind::Shape(shape) if shape.stroke.as_ref().is_some_and(|s| matches!(s.paint, Paint::Solid(_))));
        if !solid(self.node) {
            return false;
        }
        self.touch(Changes::PAINT);
        if let NodeKind::Shape(shape) = &mut self.node.kind {
            if let Some(stroke) = shape.stroke.as_mut() {
                stroke.paint = Paint::Solid(color);
            }
        }
        true
    }
}

impl Deref for NodeMut<'_> {
    type Target = Node;

    fn deref(&self) -> &Node {
        self.node
    }
}

/// Writing through this assumes the worst and stamps every kind; use the precise methods for frequent changes.
impl DerefMut for NodeMut<'_> {
    fn deref_mut(&mut self) -> &mut Node {
        self.touch(Changes::ALL);
        self.node
    }
}
