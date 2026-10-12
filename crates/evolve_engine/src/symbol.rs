//! Symbols (prefabs): a master artwork stored once and placed many times as instances.
//!
//! Instances share the master's geometry. Each instance has its own transform and may override
//! the fill and stroke color of everything inside the master.

use crate::geometry::{Affine2, Rect, Vec2};
use crate::scene::{self, Node, NodeId, NodeKind, Scene, Shape};
use crate::style::{Color, Paint, Stroke};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u64);

/// The master artwork. Its root is a group centered on the origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub root: Node,
}

/// A placed copy of a symbol (the node's own transform positions it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub symbol: SymbolId,
    #[serde(default)]
    pub fill: Option<Color>,
    #[serde(default)]
    pub stroke: Option<Color>,
}

/// Color overrides in effect while drawing inside an instance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Overrides {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
}

impl Overrides {
    /// Overrides of an instance nested inside another: the outer instance wins.
    pub fn inside(self, instance: &Instance) -> Overrides {
        Overrides { fill: self.fill.or(instance.fill), stroke: self.stroke.or(instance.stroke) }
    }

    /// The fill to draw for `shape`. Shapes without a fill stay without one.
    pub fn fill_for<'a>(&self, shape: &'a Shape) -> Option<Cow<'a, Paint>> {
        match (&shape.fill, self.fill) {
            (Some(_), Some(color)) => Some(Cow::Owned(Paint::Solid(color))),
            (Some(paint), None) => Some(Cow::Borrowed(paint)),
            (None, _) => None,
        }
    }

    pub fn stroke_paint_for<'a>(&self, stroke: &'a Stroke) -> Cow<'a, Paint> {
        match self.stroke {
            Some(color) => Cow::Owned(Paint::Solid(color)),
            None => Cow::Borrowed(&stroke.paint),
        }
    }
}

/// True if `node` (or anything below it) contains an instance for which `test` holds.
fn any_instance(node: &Node, test: &mut dyn FnMut(&Instance) -> bool) -> bool {
    match &node.kind {
        NodeKind::Instance(instance) => test(instance),
        NodeKind::Group(children) => children.iter().any(|c| any_instance(c, test)),
        NodeKind::Shape(_) | NodeKind::Camera(_) => false,
    }
}

/// Bakes the overrides into a detached copy of the artwork.
fn apply_overrides(node: &mut Node, overrides: Overrides) {
    match &mut node.kind {
        NodeKind::Shape(shape) => {
            if let (Some(paint), Some(color)) = (&mut shape.fill, overrides.fill) {
                *paint = Paint::Solid(color);
            }
            if let (Some(stroke), Some(color)) = (&mut shape.stroke, overrides.stroke) {
                stroke.paint = Paint::Solid(color);
            }
        }
        NodeKind::Group(children) => children.iter_mut().for_each(|c| apply_overrides(c, overrides)),
        NodeKind::Instance(instance) => {
            instance.fill = overrides.fill.or(instance.fill);
            instance.stroke = overrides.stroke.or(instance.stroke);
        }
        NodeKind::Camera(_) => {}
    }
}

impl Scene {
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.id == id)
    }

    /// Bounds of the master artwork in its own space (centered on the origin).
    pub fn symbol_bounds(&self, id: SymbolId) -> Option<Rect> {
        let root = &self.symbol(id)?.root;
        scene::node_bounds(self, root, root.transform)
    }

    /// Number of instances of `id` in the scene and inside other symbols.
    pub fn symbol_usage(&self, id: SymbolId) -> usize {
        fn count(node: &Node, id: SymbolId) -> usize {
            match &node.kind {
                NodeKind::Instance(i) => usize::from(i.symbol == id),
                NodeKind::Group(children) => children.iter().map(|c| count(c, id)).sum(),
                NodeKind::Shape(_) | NodeKind::Camera(_) => 0,
            }
        }
        let in_scene: usize = self.nodes.iter().map(|n| count(n, id)).sum();
        let in_symbols: usize = self.symbols.iter().map(|s| count(&s.root, id)).sum();
        in_scene + in_symbols
    }

    /// True if symbol `outer` contains `inner`, directly or through other symbols.
    pub fn symbol_uses(&self, outer: SymbolId, inner: SymbolId) -> bool {
        let mut visited = vec![outer];
        let mut stack = vec![outer];
        while let Some(current) = stack.pop() {
            let Some(symbol) = self.symbol(current) else { continue };
            let mut used = Vec::new();
            any_instance(&symbol.root, &mut |i| {
                used.push(i.symbol);
                false
            });
            for next in used {
                if next == inner {
                    return true;
                }
                if !visited.contains(&next) {
                    visited.push(next);
                    stack.push(next);
                }
            }
        }
        false
    }

    /// True if any symbol (indirectly) contains itself, which would make drawing recurse forever.
    pub fn has_symbol_cycle(&self) -> bool {
        self.symbols.iter().any(|s| self.symbol_uses(s.id, s.id))
    }

    /// Creates a symbol from the selected nodes and replaces them with an instance of it.
    pub fn make_symbol(&mut self, ids: &[NodeId], name: &str) -> Option<(SymbolId, NodeId)> {
        let ordered = self.top_level_selection(ids);
        let (root, center) = self.bake_root(&ordered)?;
        let id = SymbolId(self.next_id);
        self.next_id += 1;
        self.symbols.push(Symbol { id, name: name.to_string(), root });
        let instance = self.replace_with_instance(&ordered, id, center)?;
        Some((id, instance))
    }

    /// Redefines a symbol from the selected nodes (which become an instance of it).
    /// Fails if the selection would make the symbol contain itself.
    pub fn update_symbol(&mut self, symbol: SymbolId, ids: &[NodeId]) -> Option<NodeId> {
        let ordered = self.top_level_selection(ids);
        let selected: Vec<&Node> = ordered.iter().filter_map(|&id| self.find(id)).collect();
        let cyclic = selected.iter().any(|n| {
            any_instance(n, &mut |i| i.symbol == symbol || self.symbol_uses(i.symbol, symbol))
        });
        if cyclic {
            return None;
        }
        let (root, center) = self.bake_root(&ordered)?;
        self.symbols.iter_mut().find(|s| s.id == symbol)?.root = root;
        self.replace_with_instance(&ordered, symbol, center)
    }

    /// Adds an instance with the master centered on `at`.
    pub fn place_instance(&mut self, symbol: SymbolId, at: Vec2) -> Option<NodeId> {
        let name = self.symbol(symbol)?.name.clone();
        let mut node = self.new_node(name, NodeKind::Instance(Instance { symbol, fill: None, stroke: None }));
        node.transform = Affine2::from_translation(at);
        Some(self.add(node, None))
    }

    /// A detached, editable copy of the master artwork (not yet in the scene).
    pub fn symbol_copy(&mut self, symbol: SymbolId) -> Option<Node> {
        let source = self.symbol(symbol)?;
        let (root, name) = (source.root.clone(), source.name.clone());
        let mut copy = self.clone_with_new_ids(root);
        copy.name = name;
        Some(copy)
    }

    /// Turns an instance into an ordinary group (with its overrides applied) so it can be edited freely.
    pub fn detach_instance(&mut self, id: NodeId) -> Option<NodeId> {
        let node = self.find(id)?;
        let NodeKind::Instance(instance) = &node.kind else { return None };
        let (symbol, overrides) = (instance.symbol, Overrides { fill: instance.fill, stroke: instance.stroke });
        let (name, transform, opacity) = (node.name.clone(), node.transform, node.opacity);
        let (parent, index) = self.locate(id)?;

        let mut copy = self.symbol_copy(symbol)?;
        apply_overrides(&mut copy, overrides);
        (copy.name, copy.transform, copy.opacity) = (name, transform, opacity);
        let copy_id = copy.id;
        self.remove(id);
        self.insert(copy, parent, index);
        Some(copy_id)
    }

    /// Removes an unused symbol. Returns false if it is still placed somewhere.
    pub fn delete_symbol(&mut self, id: SymbolId) -> bool {
        if self.symbol_usage(id) > 0 {
            return false;
        }
        self.symbols.retain(|s| s.id != id);
        true
    }

    /// Group of copies of `ordered` centered on their common center, which is returned.
    fn bake_root(&mut self, ordered: &[NodeId]) -> Option<(Node, Vec2)> {
        let bounds = ordered.iter().filter_map(|&id| self.world_bounds(id)).reduce(|a, b| a.union(&b))?;
        let center = bounds.center();
        let mut children = Vec::new();
        for &id in ordered {
            let mut node = self.find(id)?.clone();
            node.transform = Affine2::from_translation(-center) * self.world_transform(id)?;
            children.push(node);
        }
        Some((self.new_node("Symbol", NodeKind::Group(children)), center))
    }

    /// Puts an instance (centered on `center`, in world space) where the first of `ordered` was
    /// and removes `ordered`.
    fn replace_with_instance(&mut self, ordered: &[NodeId], symbol: SymbolId, center: Vec2) -> Option<NodeId> {
        let (parent, index) = self.locate(*ordered.first()?)?;
        let name = self.symbol(symbol)?.name.clone();
        let parent_world = parent.and_then(|p| self.world_transform(p)).unwrap_or(Affine2::IDENTITY);
        let mut node = self.new_node(name, NodeKind::Instance(Instance { symbol, fill: None, stroke: None }));
        node.transform = parent_world.inverse() * Affine2::from_translation(center);
        let id = node.id;
        self.insert(node, parent, index);
        for &old in ordered {
            self.remove(old);
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path;

    fn square(scene: &mut Scene, at: Vec2) -> NodeId {
        let mut node = scene.new_node("sq", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)))));
        node.transform = Affine2::from_translation(at);
        scene.add(node, None)
    }

    #[test]
    fn making_a_symbol_keeps_the_artwork_in_place() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::new(100.0, 50.0));
        let (symbol, instance) = scene.make_symbol(&[a], "Box").unwrap();
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.world_bounds(instance).unwrap().min, Vec2::new(100.0, 50.0));
        assert_eq!(scene.symbol_bounds(symbol).unwrap().center(), Vec2::ZERO);
    }

    #[test]
    fn instances_are_placed_independently_and_hit_testable() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let (symbol, _) = scene.make_symbol(&[a], "Box").unwrap();
        let copy = scene.place_instance(symbol, Vec2::new(200.0, 200.0)).unwrap();
        assert_eq!(scene.symbol_usage(symbol), 2);
        assert_eq!(scene.hit_test(Vec2::new(200.0, 200.0), 1.0), Some(vec![copy]));
    }

    #[test]
    fn detaching_gives_an_editable_group_with_overrides_applied() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let (symbol, instance) = scene.make_symbol(&[a], "Box").unwrap();
        if let Some(Node { kind: NodeKind::Instance(i), .. }) = scene.find_mut(instance) {
            i.fill = Some(Color::rgb(255, 0, 0));
        }
        let group = scene.detach_instance(instance).unwrap();
        assert_eq!(scene.symbol_usage(symbol), 0);
        let NodeKind::Shape(shape) = &scene.find(group).unwrap().children()[0].kind else { panic!("expected a shape") };
        assert_eq!(shape.fill, Some(Paint::Solid(Color::rgb(255, 0, 0))));
        assert!(scene.delete_symbol(symbol));
    }

    #[test]
    fn a_symbol_cannot_contain_itself() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let (symbol, instance) = scene.make_symbol(&[a], "Box").unwrap();
        assert!(!scene.delete_symbol(symbol), "still in use");
        assert!(scene.update_symbol(symbol, &[instance]).is_none());
        assert!(!scene.has_symbol_cycle());
    }
}
