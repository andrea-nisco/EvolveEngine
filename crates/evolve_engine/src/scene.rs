//! The scene graph: a tree of groups and vector shapes.

use crate::animation::Clip;
use crate::boolean::{self, BoolOp};
use crate::game_camera::{CameraView, GameCamera};
use crate::geometry::{mean_scale, Affine2, Rect, Vec2};
use crate::path::Path;
use crate::style::{Color, FillRule, Paint, Stroke};
use crate::physics::Body;
use crate::rig::{Bone, Ik, Skin};
use crate::symbol::{Instance, Symbol};
use serde::{Deserialize, Serialize};

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
        return shape.effective_path(scene, mask_world).transformed(&mask_world).bounds();
    }
    match &node.kind {
        NodeKind::Shape(shape) => shape.effective_path(scene, world).transformed(&world).bounds(),
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
fn hit_in(scene: &Scene, nodes: &[Node], parent_world: Affine2, p: Vec2, tolerance: f32, chain: &mut Vec<NodeId>) -> bool {
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
                    && hit_in(scene, content, world, p, tolerance, chain)
            }
            NodeKind::Group(children) => hit_in(scene, children, world, p, tolerance, chain),
            NodeKind::Instance(instance) => scene.symbol(instance.symbol).is_some_and(|symbol| {
                // The master's inner nodes are not part of the tree, so keep only the instance in the chain.
                hit_in(scene, std::slice::from_ref(&symbol.root), world, p, tolerance, &mut Vec::new())
            }),
            NodeKind::Shape(shape) => {
                let local = world.inverse().transform_point2(p);
                let tol = tolerance / mean_scale(&world);
                let path = shape.effective_path(scene, world);
                let on_fill = shape.fill.is_some() && path.contains(local, shape.fill_rule, tol * 0.5);
                let on_stroke =
                    shape.stroke.as_ref().is_some_and(|s| path.distance_to_outline(local, tol * 0.5) <= s.width * 0.5 + tol);
                on_fill || on_stroke
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
        let siblings = match parent.and_then(|p| find_in_mut(&mut self.nodes, p)) {
            Some(Node { kind: NodeKind::Group(children), .. }) => children,
            _ => &mut self.nodes,
        };
        let index = index.min(siblings.len());
        siblings.insert(index, node);
    }

    // --- Queries -------------------------------------------------------------------------------

    pub fn find(&self, id: NodeId) -> Option<&Node> {
        find_in(&self.nodes, id)
    }

    pub fn find_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        find_in_mut(&mut self.nodes, id)
    }

    /// All node ids in document order (parents before children, bottom to top).
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
        hit_in(self, &self.nodes, Affine2::IDENTITY, p, tolerance, &mut chain).then_some(chain)
    }

    /// Selected ids in document order, without nodes whose ancestor is also selected.
    pub fn top_level_selection(&self, ids: &[NodeId]) -> Vec<NodeId> {
        self.all_ids()
            .into_iter()
            .filter(|id| ids.contains(id))
            .filter(|&id| !ids.iter().any(|&other| other != id && self.is_descendant(other, id)))
            .collect()
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
        Some(siblings.remove(index))
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
        if let Some(Node { kind: NodeKind::Shape(shape), .. }) = self.find_mut(target) {
            shape.path.subpaths.extend(merged);
            shape.fill_rule = FillRule::EvenOdd;
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
        if let Some(Node { kind: NodeKind::Shape(shape), .. }) = self.find_mut(target) {
            shape.path = result;
            shape.fill_rule = FillRule::EvenOdd;
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
            released.push(copy.id);
            self.insert(copy, parent, index + offset);
        }
        if let Some(Node { kind: NodeKind::Shape(s), .. }) = self.find_mut(id) {
            s.path.subpaths.truncate(1);
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
        let Some(node) = self.find_mut(id) else { return false };
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
            NodeKind::Group(children) => children.iter_mut().for_each(|c| c.transform = shift * c.transform),
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
        let node = self.find_mut(group)?;
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
