//! Node tool: edit anchors and Bézier handles of a single shape.

use crate::app::EditorApp;
use crate::tools::{AnchorPart, AnchorRef, CanvasInput, Drag, Mirror};
use evolve_engine::geometry::{mean_scale, Vec2};
use evolve_engine::path::Anchor;
use evolve_engine::scene::{NodeId, NodeKind, Shape};

const PICK_RADIUS: f32 = 7.0;

fn are_opposite(a: Vec2, b: Vec2) -> bool {
    a.perp_dot(b).abs() <= 1e-3 * a.length() * b.length() && a.dot(b) < 0.0
}

/// Where the opposite handle should go when `dragged` moves, if it should follow at all.
fn mirrored(dragged: Vec2, mirror: Mirror) -> Option<Vec2> {
    match mirror {
        Mirror::Off => None,
        Mirror::KeepLength(len) => Some(-dragged.normalize_or_zero() * len),
        Mirror::EqualLength => Some(-dragged),
    }
}

impl EditorApp {
    /// The one selected shape that the node tool is editing.
    pub fn edited_shape(&self) -> Option<NodeId> {
        let [id] = self.selection[..] else { return None };
        matches!(self.scene.find(id)?.kind, NodeKind::Shape(_)).then_some(id)
    }

    fn shape_mut(&mut self, id: NodeId) -> Option<&mut Shape> {
        match &mut self.scene.find_mut(id)?.kind {
            NodeKind::Shape(shape) => Some(shape),
            NodeKind::Group(_) | NodeKind::Instance(_) | NodeKind::Camera(_) => None,
        }
    }

    fn anchor_at(&self, target: AnchorRef) -> Option<&Anchor> {
        match &self.scene.find(target.node)?.kind {
            NodeKind::Shape(shape) => shape.path.subpaths.get(target.sub)?.anchors.get(target.index),
            NodeKind::Group(_) | NodeKind::Instance(_) | NodeKind::Camera(_) => None,
        }
    }

    pub fn anchor_exists(&self, target: AnchorRef) -> bool {
        self.anchor_at(target).is_some()
    }

    /// The anchor, or a handle of the selected anchor, under a screen position.
    pub fn pick_anchor_part(&self, screen: Vec2) -> Option<(AnchorRef, AnchorPart)> {
        let node = self.edited_shape()?;
        let world = self.scene.world_transform(node)?;
        let to_screen = |local: Vec2| self.camera.world_to_screen(world.transform_point2(local));
        let NodeKind::Shape(shape) = &self.scene.find(node)?.kind else { return None };

        if let Some(selected) = self.canvas.anchor.filter(|a| a.node == node) {
            if let Some(a) = self.anchor_at(selected) {
                for (part, offset) in [(AnchorPart::HandleOut, a.handle_out), (AnchorPart::HandleIn, a.handle_in)] {
                    if offset != Vec2::ZERO && to_screen(a.pos + offset).distance(screen) <= PICK_RADIUS {
                        return Some((selected, part));
                    }
                }
            }
        }
        for (sub, subpath) in shape.path.subpaths.iter().enumerate() {
            for (index, a) in subpath.anchors.iter().enumerate() {
                if to_screen(a.pos).distance(screen) <= PICK_RADIUS {
                    return Some((AnchorRef { node, sub, index }, AnchorPart::Anchor));
                }
            }
        }
        None
    }

    pub fn node_tool(&mut self, input: &CanvasInput) {
        if input.drag_started {
            self.begin_anchor_drag(input);
        }
        if input.dragging {
            self.update_anchor_drag(input);
        }
        if input.drag_stopped {
            self.canvas.drag = Drag::None;
        }
        if input.double_clicked {
            self.node_double_click(input);
        } else if input.clicked {
            self.node_click(input);
        }
    }

    fn begin_anchor_drag(&mut self, input: &CanvasInput) {
        if let Some((target, part)) = self.pick_anchor_part(input.screen_origin) {
            self.canvas.anchor = Some(target);
            let Some(&a) = self.anchor_at(target) else { return };
            let mirror = match part {
                AnchorPart::Anchor => Mirror::Off,
                _ if a.handle_in == Vec2::ZERO && a.handle_out == Vec2::ZERO => Mirror::EqualLength,
                AnchorPart::HandleOut if are_opposite(a.handle_in, a.handle_out) => Mirror::KeepLength(a.handle_in.length()),
                AnchorPart::HandleIn if are_opposite(a.handle_in, a.handle_out) => Mirror::KeepLength(a.handle_out.length()),
                _ => Mirror::Off,
            };
            self.canvas.drag = Drag::Anchor { target, part, mirror };
        } else if let Some(chain) = self.scene.hit_test(input.origin, input.tolerance) {
            self.selection = vec![chain[chain.len() - 1]];
            self.canvas.anchor = None;
        }
    }

    fn update_anchor_drag(&mut self, input: &CanvasInput) {
        let Drag::Anchor { target, part, mirror } = self.canvas.drag.clone() else { return };
        let Some(pos) = input.pos else { return };
        let Some(world) = self.scene.world_transform(target.node) else { return };
        let mirror = if input.alt { Mirror::Off } else { mirror };
        let world_pos = if part == AnchorPart::Anchor { self.snap(pos) } else { pos };
        let local = world.inverse().transform_point2(world_pos);

        let Some(shape) = self.shape_mut(target.node) else { return };
        let Some(a) = shape.path.subpaths.get_mut(target.sub).and_then(|s| s.anchors.get_mut(target.index)) else { return };
        match part {
            AnchorPart::Anchor => a.pos = local,
            AnchorPart::HandleOut => {
                a.handle_out = local - a.pos;
                a.handle_in = mirrored(a.handle_out, mirror).unwrap_or(a.handle_in);
            }
            AnchorPart::HandleIn => {
                a.handle_in = local - a.pos;
                a.handle_out = mirrored(a.handle_in, mirror).unwrap_or(a.handle_out);
            }
        }
    }

    fn node_click(&mut self, input: &CanvasInput) {
        if let Some((target, _)) = self.pick_anchor_part(input.screen) {
            self.canvas.anchor = Some(target);
            return;
        }
        self.canvas.anchor = None;
        match input.pos.and_then(|p| self.scene.hit_test(p, input.tolerance)) {
            Some(chain) => self.selection = vec![chain[chain.len() - 1]],
            None if !input.shift => self.selection.clear(),
            None => {}
        }
    }

    fn node_double_click(&mut self, input: &CanvasInput) {
        if let Some((target, AnchorPart::Anchor)) = self.pick_anchor_part(input.screen) {
            self.toggle_smooth(target);
            return;
        }
        let (Some(node), Some(pos)) = (self.edited_shape(), input.pos) else { return };
        let Some(world) = self.scene.world_transform(node) else { return };
        let local = world.inverse().transform_point2(pos);
        let scale = mean_scale(&world);
        let Some(shape) = self.shape_mut(node) else { return };
        if let Some((sub, segment, t, distance)) = shape.path.nearest_segment(local) {
            if distance * scale <= input.tolerance {
                shape.path.subpaths[sub].split_segment(segment, t);
                self.canvas.anchor = Some(AnchorRef { node, sub, index: segment + 1 });
            }
        }
    }

    /// Corner anchors become smooth (handles along the neighbours' direction); smooth ones become corners.
    fn toggle_smooth(&mut self, target: AnchorRef) {
        let Some(shape) = self.shape_mut(target.node) else { return };
        let Some(sub) = shape.path.subpaths.get_mut(target.sub) else { return };
        let (n, i) = (sub.anchors.len(), target.index);
        let prev = (i > 0).then(|| sub.anchors[i - 1].pos).or_else(|| sub.closed.then(|| sub.anchors[n - 1].pos));
        let next = (i + 1 < n).then(|| sub.anchors[i + 1].pos).or_else(|| sub.closed.then(|| sub.anchors[0].pos));
        let a = &mut sub.anchors[i];

        if a.handle_in != Vec2::ZERO || a.handle_out != Vec2::ZERO {
            a.handle_in = Vec2::ZERO;
            a.handle_out = Vec2::ZERO;
            return;
        }
        let direction = match (prev, next) {
            (Some(p), Some(q)) => q - p,
            (Some(p), None) => a.pos - p,
            (None, Some(q)) => q - a.pos,
            (None, None) => return,
        };
        let reach = [prev, next].into_iter().flatten().map(|q| q.distance(a.pos)).fold(f32::MAX, f32::min) / 3.0;
        *a = Anchor::smooth(a.pos, direction.normalize_or_zero() * reach);
    }

    /// Cuts the path open at the selected anchor.
    pub fn split_selected_anchor(&mut self) {
        let Some(target) = self.canvas.anchor else {
            self.status = "Select an anchor with the Node tool first".into();
            return;
        };
        let cut = self.shape_mut(target.node).and_then(|shape| {
            let parts = shape.path.subpaths.get(target.sub)?.split_at_anchor(target.index)?;
            shape.path.subpaths.splice(target.sub..=target.sub, parts);
            Some(())
        });
        match cut {
            Some(()) => self.canvas.anchor = None,
            None => self.status = "An open path can't be cut at its end points".into(),
        }
    }

    /// Removes the selected anchor. Returns false if there was none.
    pub fn delete_selected_anchor(&mut self) -> bool {
        let Some(target) = self.canvas.anchor else { return false };
        self.canvas.anchor = None;
        let Some(shape) = self.shape_mut(target.node) else { return false };
        let Some(sub) = shape.path.subpaths.get_mut(target.sub) else { return false };
        sub.anchors.remove(target.index);
        if sub.anchors.len() < 2 {
            shape.path.subpaths.remove(target.sub);
        }
        if shape.path.subpaths.is_empty() {
            self.scene.remove(target.node);
            self.selection.clear();
        }
        true
    }
}
