//! Copying objects between places and documents, and spawning copies while a game runs.

use crate::geometry::{Affine2, Vec2};
use crate::revision::Changes;
use crate::scene::{Node, NodeId, NodeKind, Scene};
use crate::symbol::{Symbol, SymbolId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Text that marks clipboard contents as copied objects (so they can travel through the system clipboard).
const MARKER: &str = "EVOLVE-CLIPBOARD:";

/// Objects that were copied, with the symbols they use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clipboard {
    /// Top-level objects with their transforms in world space.
    pub nodes: Vec<Node>,
    pub symbols: Vec<Symbol>,
}

impl Clipboard {
    /// How many copied shapes were skinned. Their skins can't be pasted (they refer to the original bones).
    pub fn skinned_shapes(&self) -> usize {
        fn count(node: &Node) -> usize {
            match &node.kind {
                NodeKind::Shape(shape) => usize::from(shape.skin.is_some()),
                NodeKind::Group(children) => children.iter().map(count).sum(),
                NodeKind::Instance(_) | NodeKind::Camera(_) => 0,
            }
        }
        self.nodes.iter().map(count).sum()
    }

    pub fn to_text(&self) -> Option<String> {
        serde_json::to_string(self).ok().map(|json| format!("{MARKER}{json}"))
    }

    pub fn from_text(text: &str) -> Option<Clipboard> {
        serde_json::from_str(text.strip_prefix(MARKER)?).ok()
    }
}

/// Symbols used by `node` (directly).
fn used_symbols(node: &Node, out: &mut Vec<SymbolId>) {
    match &node.kind {
        NodeKind::Instance(instance) => out.push(instance.symbol),
        NodeKind::Group(children) => children.iter().for_each(|c| used_symbols(c, out)),
        NodeKind::Shape(_) | NodeKind::Camera(_) => {}
    }
}

/// Points instances at their new symbols and drops skins (they refer to bones of the old document).
fn fix_pasted(node: &mut Node, symbols: &HashMap<SymbolId, SymbolId>) {
    match &mut node.kind {
        NodeKind::Instance(instance) => {
            if let Some(&new) = symbols.get(&instance.symbol) {
                instance.symbol = new;
                node.revs.bump(Changes::STRUCTURE);
            }
        }
        NodeKind::Group(children) => children.iter_mut().for_each(|c| fix_pasted(c, symbols)),
        NodeKind::Shape(shape) => {
            if shape.skin.take().is_some() {
                node.revs.bump(Changes::GEOMETRY);
            }
        }
        NodeKind::Camera(_) => {}
    }
}

impl Scene {
    /// Copies the given objects (parents of selected objects win over their children).
    pub fn copy_nodes(&self, ids: &[NodeId]) -> Option<Clipboard> {
        let mut nodes = Vec::new();
        for id in self.top_level_selection(ids) {
            let mut node = self.find(id)?.clone();
            node.transform = self.world_transform(id)?;
            node.revs.bump(Changes::TRANSFORM);
            nodes.push(node);
        }
        if nodes.is_empty() {
            return None;
        }
        // Collect the symbols they use, including symbols used inside those symbols.
        let mut wanted = Vec::new();
        nodes.iter().for_each(|n| used_symbols(n, &mut wanted));
        let mut symbols: Vec<Symbol> = Vec::new();
        while let Some(id) = wanted.pop() {
            if symbols.iter().any(|s| s.id == id) {
                continue;
            }
            if let Some(symbol) = self.symbol(id) {
                used_symbols(&symbol.root, &mut wanted);
                symbols.push(symbol.clone());
            }
        }
        Some(Clipboard { nodes, symbols })
    }

    /// Adds copies of the clipboard's objects on top of the scene, moved by `offset`.
    /// Symbols that already exist unchanged are reused; others are added.
    pub fn paste_nodes(&mut self, clipboard: &Clipboard, offset: Vec2) -> Vec<NodeId> {
        let mut mapping = HashMap::new();
        for symbol in &clipboard.symbols {
            if self.symbol(symbol.id).is_some_and(|existing| existing.root == symbol.root) {
                mapping.insert(symbol.id, symbol.id);
                continue;
            }
            let id = SymbolId(self.next_id);
            self.next_id += 1;
            mapping.insert(symbol.id, id);
            let root = self.clone_with_new_ids(symbol.root.clone());
            self.symbols.push(Symbol { id, name: symbol.name.clone(), root });
            self.touch(Changes::STRUCTURE);
        }
        // Symbols pasted with new ids may contain instances of other pasted symbols.
        let mut touched = false;
        for symbol in &mut self.symbols {
            if mapping.values().any(|&new| new == symbol.id) {
                fix_symbol_references(&mut symbol.root, &mapping);
                touched = true;
            }
        }
        if touched {
            self.touch(Changes::STRUCTURE);
        }

        let mut pasted = Vec::new();
        for node in &clipboard.nodes {
            let mut copy = self.clone_with_new_ids(node.clone());
            fix_pasted(&mut copy, &mapping);
            copy.transform = Affine2::from_translation(offset) * copy.transform;
            copy.revs.bump(Changes::TRANSFORM);
            pasted.push(self.add(copy, None));
        }
        pasted
    }

    /// Creates a copy of the object (or symbol instance) named `name` with its origin at `at`.
    /// An object named like that is copied as it is, including its physics body and script, and
    /// made visible, so a hidden object works as a template. Otherwise a symbol with that name is placed.
    pub fn spawn(&mut self, name: &str, at: Vec2) -> Option<NodeId> {
        let id = match self.find_by_name(name) {
            Some(template) => self.duplicate(template)?,
            None => {
                let symbol = self.symbols.iter().find(|s| s.name == name)?.id;
                self.place_instance(symbol, Vec2::ZERO)?
            }
        };
        let origin = self.world_transform(id)?.translation;
        self.translate_world(id, at - origin);
        if let Some(mut node) = self.find_mut(id) {
            node.set_visible(true);
        }
        Some(id)
    }
}

fn fix_symbol_references(node: &mut Node, mapping: &HashMap<SymbolId, SymbolId>) {
    match &mut node.kind {
        NodeKind::Instance(instance) => {
            if let Some(&new) = mapping.get(&instance.symbol) {
                instance.symbol = new;
                node.revs.bump(Changes::STRUCTURE);
            }
        }
        NodeKind::Group(children) => children.iter_mut().for_each(|c| fix_symbol_references(c, mapping)),
        NodeKind::Shape(_) | NodeKind::Camera(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path;
    use crate::scene::Shape;

    fn square(scene: &mut Scene, at: Vec2) -> NodeId {
        let mut node = scene.new_node("sq", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)))));
        node.transform = Affine2::from_translation(at);
        scene.add(node, None)
    }

    #[test]
    fn pasting_adds_independent_copies_at_an_offset() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::new(5.0, 5.0));
        let clipboard = scene.copy_nodes(&[a]).unwrap();
        let pasted = scene.paste_nodes(&clipboard, Vec2::new(20.0, 0.0));
        assert_eq!(scene.nodes.len(), 2);
        assert_ne!(pasted[0], a);
        assert_eq!(scene.world_bounds(pasted[0]).unwrap().min, Vec2::new(25.0, 5.0));
        assert_eq!(scene.world_bounds(a).unwrap().min, Vec2::new(5.0, 5.0), "the original stays");
    }

    #[test]
    fn copying_keeps_the_world_position_of_nested_objects() {
        let mut scene = Scene::default();
        let inner = square(&mut scene, Vec2::new(1.0, 1.0));
        let group = scene.group(&[inner]).unwrap();
        scene.find_mut(group).unwrap().transform = Affine2::from_translation(Vec2::new(100.0, 0.0));
        let clipboard = scene.copy_nodes(&[inner]).unwrap();
        let pasted = scene.paste_nodes(&clipboard, Vec2::ZERO);
        assert_eq!(scene.world_bounds(pasted[0]).unwrap().min, Vec2::new(101.0, 1.0));
    }

    #[test]
    fn symbols_travel_with_their_instances_and_are_reused_when_unchanged() {
        let mut source = Scene::default();
        let a = square(&mut source, Vec2::ZERO);
        let (symbol, instance) = source.make_symbol(&[a], "Box").unwrap();
        let clipboard = source.copy_nodes(&[instance]).unwrap();
        assert_eq!(clipboard.symbols.len(), 1);

        let mut other = Scene::default();
        let pasted = other.paste_nodes(&clipboard, Vec2::ZERO);
        assert_eq!(other.symbols.len(), 1, "the symbol came along");
        assert_eq!(other.symbol_usage(other.symbols[0].id), 1);
        assert!(other.world_bounds(pasted[0]).is_some());

        // Pasting into the document it came from reuses the symbol.
        let again = source.paste_nodes(&clipboard, Vec2::new(5.0, 0.0));
        assert_eq!(source.symbols.len(), 1, "same document: reused");
        assert_eq!(source.symbol_usage(symbol), 2);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn the_clipboard_survives_a_trip_through_text() {
        let mut scene = Scene::default();
        let a = square(&mut scene, Vec2::ZERO);
        let clipboard = scene.copy_nodes(&[a]).unwrap();
        let text = clipboard.to_text().unwrap();
        assert!(text.starts_with("EVOLVE-CLIPBOARD:"));
        assert_eq!(Clipboard::from_text(&text), Some(clipboard));
        assert_eq!(Clipboard::from_text("just some text"), None);
    }

    #[test]
    fn spawn_copies_a_hidden_template_or_places_a_symbol() {
        let mut scene = Scene::default();
        let template = square(&mut scene, Vec2::new(500.0, 500.0));
        {
            let mut node = scene.find_mut(template).unwrap();
            node.name = "Bullet".into();
            node.visible = false;
            node.script = Some("fn on_update(me, dt) {}".into());
        }
        let bullet = scene.spawn("Bullet", Vec2::new(30.0, 40.0)).unwrap();
        let node = scene.find(bullet).unwrap();
        assert!(node.visible && node.script.is_some());
        assert_eq!(scene.world_transform(bullet).unwrap().translation, Vec2::new(30.0, 40.0));
        assert!(!scene.find(template).unwrap().visible, "the template stays hidden");

        let (_, _) = scene.make_symbol(&[bullet], "Spark").unwrap();
        let spark = scene.spawn("Spark", Vec2::new(7.0, 8.0)).unwrap();
        assert_eq!(scene.world_transform(spark).unwrap().translation, Vec2::new(7.0, 8.0));
        assert!(scene.spawn("Nothing", Vec2::ZERO).is_none());
    }
}
