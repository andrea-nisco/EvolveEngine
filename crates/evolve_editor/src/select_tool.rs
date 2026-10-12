//! Select tool: picking, box selection and the move / scale / rotate handles.

use crate::app::EditorApp;
use crate::guides::{snap_to_guides, Guide};
use crate::tools::{CanvasInput, Drag, TransformKind};
use eframe::egui::CursorIcon;
use evolve_engine::geometry::{rotate_around, scale_around, Affine2, Rect, Vec2};
use evolve_engine::scene::{NodeId, NodeKind};

/// Normalized positions of the eight scale handles, clockwise from the top-left corner.
pub const HANDLE_NORMS: [Vec2; 8] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(0.5, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(1.0, 0.5),
    Vec2::new(1.0, 1.0),
    Vec2::new(0.5, 1.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(0.0, 0.5),
];

const HANDLE_PICK_RADIUS: f32 = 7.0;
const ROTATE_HANDLE_DISTANCE: f32 = 26.0;

impl EditorApp {
    /// World-space bounds of everything selected.
    pub fn selection_bounds(&self) -> Option<Rect> {
        self.scene
            .top_level_selection(&self.selection)
            .into_iter()
            .filter_map(|id| self.scene.world_bounds(id))
            .reduce(|a, b| a.union(&b))
    }

    /// True if a camera is selected (cameras move and turn, but are resized in the Inspector).
    pub fn selection_has_camera(&self) -> bool {
        self.selection.iter().any(|&id| matches!(self.scene.find(id).map(|n| &n.kind), Some(NodeKind::Camera(_))))
    }

    /// Screen positions of the eight scale handles and the rotate handle.
    pub fn handle_screen_positions(&self, bounds: &Rect) -> ([Vec2; 8], Vec2) {
        let to_screen = |norm: Vec2| self.camera.world_to_screen(bounds.min + bounds.size() * norm);
        let scale_handles = HANDLE_NORMS.map(to_screen);
        let (top_middle, center) = (scale_handles[1], to_screen(Vec2::splat(0.5)));
        let outward = (top_middle - center).try_normalize().unwrap_or(Vec2::NEG_Y);
        (scale_handles, top_middle + outward * ROTATE_HANDLE_DISTANCE)
    }

    fn handle_at(&self, screen: Vec2) -> Option<TransformKind> {
        let bounds = self.selection_bounds()?;
        let (scale_handles, rotate_handle) = self.handle_screen_positions(&bounds);
        if rotate_handle.distance(screen) <= HANDLE_PICK_RADIUS {
            return Some(TransformKind::Rotate);
        }
        if self.selection_has_camera() {
            return None;
        }
        scale_handles.iter().position(|h| h.distance(screen) <= HANDLE_PICK_RADIUS).map(TransformKind::Scale)
    }

    pub fn handle_cursor(&self, screen: Vec2) -> CursorIcon {
        match self.handle_at(screen) {
            Some(TransformKind::Rotate) => CursorIcon::Grab,
            Some(TransformKind::Scale(i)) => match i {
                0 | 4 => CursorIcon::ResizeNwSe,
                2 | 6 => CursorIcon::ResizeNeSw,
                1 | 5 => CursorIcon::ResizeVertical,
                _ => CursorIcon::ResizeHorizontal,
            },
            _ => CursorIcon::Default,
        }
    }

    /// Sets each item's world transform to `delta * start`, keeping parent transforms in mind.
    pub fn apply_world_delta(&mut self, items: &[(NodeId, Affine2)], delta: Affine2) {
        for &(id, start_world) in items {
            let parent_inverse = self.scene.parent_world(id).inverse();
            if let Some(node) = self.scene.find_mut(id) {
                node.transform = parent_inverse * delta * start_world;
            }
        }
    }

    pub fn select_tool(&mut self, input: &CanvasInput) {
        if input.drag_started {
            self.begin_select_drag(input);
        }
        if input.dragging {
            self.update_select_drag(input);
        }
        if input.drag_stopped {
            self.end_select_drag(input);
        }
        if input.clicked {
            self.click_select(input);
        }
    }

    fn begin_select_drag(&mut self, input: &CanvasInput) {
        let kind = self.handle_at(input.screen_origin).or_else(|| {
            let chain = self.scene.hit_test(input.origin, input.tolerance)?;
            if !chain.iter().any(|id| self.selection.contains(id)) {
                if !input.shift {
                    self.selection.clear();
                }
                self.selection.push(chain[0]);
            }
            Some(TransformKind::Move)
        });

        let Some(kind) = kind else {
            if !input.shift {
                self.selection.clear();
            }
            self.canvas.drag = Drag::Marquee { start: input.origin };
            return;
        };
        let Some(bounds) = self.selection_bounds() else { return };
        let items = self
            .scene
            .top_level_selection(&self.selection)
            .into_iter()
            .filter_map(|id| Some((id, self.scene.world_transform(id)?)))
            .collect();
        let mut targets: Vec<Rect> = self
            .scene
            .nodes
            .iter()
            .filter(|n| n.visible && !self.selection.contains(&n.id))
            .filter_map(|n| self.scene.world_bounds(n.id))
            .collect();
        targets.push(self.scene.camera_view().bounds());
        self.canvas.drag = Drag::Transform { kind, start: input.origin, bounds, items, targets };
    }

    fn update_select_drag(&mut self, input: &CanvasInput) {
        let Some(pos) = input.pos else { return };
        match std::mem::take(&mut self.canvas.drag) {
            Drag::Transform { kind, start, bounds, items, targets } => {
                let (delta, guides) = self.transform_delta(kind, start, &bounds, &targets, pos, input);
                self.apply_world_delta(&items, delta);
                self.canvas.guides = guides;
                self.canvas.drag = Drag::Transform { kind, start, bounds, items, targets };
            }
            other => self.canvas.drag = other,
        }
    }

    fn end_select_drag(&mut self, input: &CanvasInput) {
        if let (Drag::Marquee { start }, Some(pos)) = (&self.canvas.drag, input.pos) {
            let area = Rect::from_points(*start, pos);
            let hits = self
                .scene
                .nodes
                .iter()
                .filter(|n| n.visible && !n.locked && !matches!(n.kind, NodeKind::Camera(_))) // the frame covers the scene: pick it by its border
                .filter(|n| self.scene.world_bounds(n.id).is_some_and(|b| b.intersects(&area)))
                .map(|n| n.id)
                .collect::<Vec<_>>();
            for id in hits {
                if !self.selection.contains(&id) {
                    self.selection.push(id);
                }
            }
        }
        self.canvas.drag = Drag::None;
        self.canvas.guides.clear();
    }

    fn click_select(&mut self, input: &CanvasInput) {
        let Some(pos) = input.pos else { return };
        // Bones are drawn on top of the artwork, so they are picked first.
        if let Some(bone) = self.scene.pick_bone(pos, input.tolerance) {
            match (input.shift, self.selection.iter().position(|&id| id == bone)) {
                (false, _) => self.selection = vec![bone],
                (true, Some(i)) => {
                    self.selection.remove(i);
                }
                (true, None) => self.selection.push(bone),
            }
            return;
        }
        let Some(chain) = self.scene.hit_test(pos, input.tolerance) else {
            if !input.shift {
                self.selection.clear();
            }
            return;
        };
        // A plain click picks the outermost group; a double click digs down to the shape.
        let target = if input.double_clicked { chain[chain.len() - 1] } else { chain[0] };
        if input.shift {
            match self.selection.iter().position(|&id| id == target) {
                Some(i) => {
                    self.selection.remove(i);
                }
                None => self.selection.push(target),
            }
        } else {
            self.selection = vec![target];
        }
    }

    /// The transform to apply to the selection as it was when the drag began,
    /// plus the smart guides that are active.
    fn transform_delta(
        &self,
        kind: TransformKind,
        start: Vec2,
        bounds: &Rect,
        targets: &[Rect],
        pos: Vec2,
        input: &CanvasInput,
    ) -> (Affine2, Vec<Guide>) {
        let (center, size) = (bounds.center(), bounds.size());
        let transform = match kind {
            TransformKind::Move => {
                let mut delta = pos - start;
                if input.shift {
                    if delta.x.abs() > delta.y.abs() { delta.y = 0.0 } else { delta.x = 0.0 }
                }
                // Snap the box's top-left corner, not the pointer, so objects land on the grid.
                let grid_delta = self.snap(bounds.min + delta) - bounds.min;
                if !self.view.smart_guides {
                    return (Affine2::from_translation(grid_delta), Vec::new());
                }
                // Alignment with other objects wins over the grid, axis by axis.
                let moved = Rect { min: bounds.min + delta, max: bounds.max + delta };
                let (x, y, guides) = snap_to_guides(&moved, targets, 6.0 / self.camera.zoom);
                let delta = Vec2::new(x.map_or(grid_delta.x, |o| delta.x + o), y.map_or(grid_delta.y, |o| delta.y + o));
                return (Affine2::from_translation(delta), guides);
            }
            TransformKind::Scale(handle) => {
                let norm = HANDLE_NORMS[handle];
                let grabbed = bounds.min + size * norm;
                let pivot = if input.alt { center } else { bounds.min + size * (Vec2::ONE - norm) };
                let target = self.snap(pos);
                let factor = |axis_norm: f32, grabbed: f32, pivot: f32, target: f32| {
                    let span = grabbed - pivot;
                    if axis_norm == 0.5 || span.abs() < 1e-4 {
                        return 1.0;
                    }
                    let f = (target - pivot) / span;
                    if f.abs() < 1e-3 { 1e-3f32.copysign(f) } else { f }
                };
                let mut scale = Vec2::new(
                    factor(norm.x, grabbed.x, pivot.x, target.x),
                    factor(norm.y, grabbed.y, pivot.y, target.y),
                );
                let is_corner = norm.x != 0.5 && norm.y != 0.5;
                if input.shift && is_corner {
                    let uniform = if (scale.x - 1.0).abs() > (scale.y - 1.0).abs() { scale.x } else { scale.y };
                    scale = Vec2::splat(uniform);
                }
                scale_around(scale, pivot)
            }
            TransformKind::Rotate => {
                let mut angle = (pos - center).to_angle() - (start - center).to_angle();
                if input.shift {
                    let step = 15f32.to_radians();
                    angle = (angle / step).round() * step;
                }
                rotate_around(angle, center)
            }
        };
        (transform, Vec::new())
    }
}
