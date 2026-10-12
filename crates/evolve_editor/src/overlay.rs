//! Editor-only drawing on top of the rendered scene: grid, selection, handles, tool previews.
//! These are drawn with egui at screen resolution, so they stay crisp at any zoom.

use crate::app::EditorApp;
use crate::draw_tools::drag_rect;
use crate::theme::{ACCENT, ACCENT_SOFT, GUIDE, HANDLE_FILL};
use crate::tools::{CanvasInput, Drag, Tool};
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape as UiShape, Stroke, StrokeKind};
use evolve_engine::camera::Camera;
use evolve_engine::game_camera::CameraView;
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::path::{Path, Polyline};
use evolve_engine::scene::{Node, NodeKind, Scene};

const MAX_GRID_LINES: usize = 400;

/// World → egui screen conversion for one frame.
pub(crate) struct Screen<'a> {
    pub camera: &'a Camera,
    pub origin: Pos2,
}

impl Screen<'_> {
    pub fn pt(&self, world: Vec2) -> Pos2 {
        let s = self.camera.world_to_screen(world);
        Pos2::new(self.origin.x + s.x, self.origin.y + s.y)
    }

    pub fn pts(&self, world: &[Vec2]) -> Vec<Pos2> {
        world.iter().map(|&p| self.pt(p)).collect()
    }
}

fn collect_outlines(scene: &Scene, node: &Node, world: Affine2, tolerance: f32, out: &mut Vec<Polyline>) {
    match &node.kind {
        NodeKind::Shape(shape) => out.extend(shape.effective_path(scene, world).transformed(&world).flatten(tolerance)),
        NodeKind::Group(children) => {
            for child in children {
                collect_outlines(scene, child, world * child.transform, tolerance, out);
            }
        }
        NodeKind::Instance(instance) => {
            if let Some(symbol) = scene.symbol(instance.symbol) {
                collect_outlines(scene, &symbol.root, world * symbol.root.transform, tolerance, out);
            }
        }
        NodeKind::Camera(camera) => out.extend(CameraView::new(camera, &world).outline().flatten(tolerance)),
    }
}

fn square_handle(painter: &Painter, center: Pos2, selected: bool) {
    let rect = Rect::from_center_size(center, egui::vec2(8.0, 8.0));
    let fill = if selected { ACCENT } else { HANDLE_FILL };
    painter.rect_filled(rect, 1.0, fill);
    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, ACCENT), StrokeKind::Outside);
}

impl EditorApp {
    pub fn paint_overlays(&self, painter: &Painter, rect: Rect, input: &CanvasInput) {
        let painter = painter.with_clip_rect(rect);
        let screen = Screen { camera: &self.camera, origin: rect.min };

        if self.view.show_grid {
            self.paint_grid(&painter, &screen, rect);
        }
        self.paint_camera_frame(&painter, &screen);
        self.paint_selection_outline(&painter, &screen);

        match self.tool {
            Tool::Select => self.paint_transform_box(&painter, &screen),
            Tool::Node => self.paint_anchors(&painter, &screen),
            Tool::Pen => self.paint_pen(&painter, &screen, input),
            Tool::Weights => self.paint_weight_heatmap(&painter, &screen, input),
            Tool::Rectangle | Tool::Ellipse | Tool::Pencil | Tool::Bone => {}
        }
        self.paint_bones(&painter, &screen, input);
        self.paint_drag_preview(&painter, &screen, input);
        self.paint_guides(&painter, &screen);
    }

    /// Alignment lines shown while moving an object next to others.
    fn paint_guides(&self, painter: &Painter, screen: &Screen) {
        const REACH: f32 = 10_000.0; // screen points; the clip rect trims it to the viewport
        let reach = REACH / self.camera.zoom;
        let c = self.camera.center;
        for guide in &self.canvas.guides {
            let (a, b) = if guide.vertical {
                (Vec2::new(guide.at, c.y - reach), Vec2::new(guide.at, c.y + reach))
            } else {
                (Vec2::new(c.x - reach, guide.at), Vec2::new(c.x + reach, guide.at))
            };
            painter.line_segment([screen.pt(a), screen.pt(b)], Stroke::new(1.0, GUIDE));
        }
    }

    /// The frame of the game camera: what the player will see. Always shown, so the playable area is clear.
    fn paint_camera_frame(&self, painter: &Painter, screen: &Screen) {
        let camera = self.scene.main_camera().and_then(|id| self.scene.find(id));
        if camera.is_some_and(|n| !n.visible) {
            return;
        }
        let corners = screen.pts(&self.scene.camera_view().corners());
        let color = Color32::from_white_alpha(170);
        painter.add(UiShape::closed_line(corners.to_vec(), Stroke::new(1.5, color)));
        let name = camera.map_or("Default camera", |n| n.name.as_str());
        painter.text(corners[0] + egui::vec2(0.0, -4.0), egui::Align2::LEFT_BOTTOM, name, egui::FontId::proportional(12.0), color);
    }

    fn paint_grid(&self, painter: &Painter, screen: &Screen, rect: Rect) {
        let mut step = self.view.grid_size.max(1.0);
        while step * self.camera.zoom < 10.0 {
            step *= 2.0;
        }
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
            .map(|c| self.camera.screen_to_world(Vec2::new(c.x - rect.min.x, c.y - rect.min.y)));
        let lo = corners.iter().fold(Vec2::splat(f32::MAX), |a, &c| a.min(c));
        let hi = corners.iter().fold(Vec2::splat(f32::MIN), |a, &c| a.max(c));

        let stroke = Stroke::new(1.0, Color32::from_white_alpha(14));
        let (first_x, last_x) = ((lo.x / step).floor() as i64, (hi.x / step).ceil() as i64);
        let (first_y, last_y) = ((lo.y / step).floor() as i64, (hi.y / step).ceil() as i64);
        if ((last_x - first_x) + (last_y - first_y)) as usize > MAX_GRID_LINES {
            return; // zoomed out too far for a useful grid
        }
        for i in first_x..=last_x {
            let x = i as f32 * step;
            painter.line_segment([screen.pt(Vec2::new(x, lo.y)), screen.pt(Vec2::new(x, hi.y))], stroke);
        }
        for j in first_y..=last_y {
            let y = j as f32 * step;
            painter.line_segment([screen.pt(Vec2::new(lo.x, y)), screen.pt(Vec2::new(hi.x, y))], stroke);
        }
    }

    fn paint_polylines(&self, painter: &Painter, screen: &Screen, polylines: &[Polyline], stroke: Stroke) {
        for poly in polylines {
            let points = screen.pts(&poly.points);
            painter.add(if poly.closed { UiShape::closed_line(points, stroke) } else { UiShape::line(points, stroke) });
        }
    }

    fn paint_selection_outline(&self, painter: &Painter, screen: &Screen) {
        let tolerance = 0.5 / self.camera.zoom;
        let mut outlines = Vec::new();
        for &id in &self.selection {
            if let (Some(node), Some(world)) = (self.scene.find(id), self.scene.world_transform(id)) {
                collect_outlines(&self.scene, node, world, tolerance, &mut outlines);
            }
        }
        self.paint_polylines(painter, screen, &outlines, Stroke::new(1.0, ACCENT));
    }

    /// Bounding box with eight scale handles and a rotate handle.
    fn paint_transform_box(&self, painter: &Painter, screen: &Screen) {
        let Some(bounds) = self.selection_bounds() else { return };
        let corners = bounds.corners().map(|c| screen.pt(c));
        painter.add(UiShape::closed_line(corners.to_vec(), Stroke::new(1.0, ACCENT)));

        let (scale_handles, rotate_handle) = self.handle_screen_positions(&bounds);
        let offset = |v: Vec2| Pos2::new(screen.origin.x + v.x, screen.origin.y + v.y);
        let rotate_pos = offset(rotate_handle);
        painter.line_segment([offset(scale_handles[1]), rotate_pos], Stroke::new(1.0, ACCENT));
        painter.circle_filled(rotate_pos, 5.0, HANDLE_FILL);
        painter.circle_stroke(rotate_pos, 5.0, Stroke::new(1.0, ACCENT));
        // A camera's frame is resized in the Inspector, not by scaling.
        if self.selection_has_camera() {
            return;
        }
        for handle in scale_handles {
            square_handle(painter, offset(handle), false);
        }
    }

    /// Anchors of the edited shape, plus the handles of the selected anchor.
    fn paint_anchors(&self, painter: &Painter, screen: &Screen) {
        let Some(node) = self.edited_shape() else { return };
        let (Some(world), Some(Node { kind: NodeKind::Shape(shape), .. })) =
            (self.scene.world_transform(node), self.scene.find(node))
        else {
            return;
        };
        let to_screen = |local: Vec2| screen.pt(world.transform_point2(local));

        for (sub, subpath) in shape.path.subpaths.iter().enumerate() {
            for (index, anchor) in subpath.anchors.iter().enumerate() {
                let selected = self.canvas.anchor.is_some_and(|a| a.node == node && a.sub == sub && a.index == index);
                if selected {
                    for offset in [anchor.handle_in, anchor.handle_out].into_iter().filter(|o| *o != Vec2::ZERO) {
                        let tip = to_screen(anchor.pos + offset);
                        painter.line_segment([to_screen(anchor.pos), tip], Stroke::new(1.0, ACCENT));
                        painter.circle_filled(tip, 4.0, HANDLE_FILL);
                        painter.circle_stroke(tip, 4.0, Stroke::new(1.0, ACCENT));
                    }
                }
                square_handle(painter, to_screen(anchor.pos), selected);
            }
        }
    }

    fn paint_pen(&self, painter: &Painter, screen: &Screen, input: &CanvasInput) {
        let pen = &self.canvas.pen;
        if pen.is_empty() {
            return;
        }
        // Preview: the placed anchors plus a rubber band to the cursor.
        let mut preview = pen.clone();
        let dragging_handle = matches!(self.canvas.drag, Drag::PenHandle);
        if let (Some(pos), false) = (input.pos, dragging_handle) {
            preview.push(evolve_engine::path::Anchor::corner(self.snap(pos)));
        }
        let path = Path { subpaths: vec![evolve_engine::path::SubPath { anchors: preview, closed: false }] };
        self.paint_polylines(painter, screen, &path.flatten(0.5 / self.camera.zoom), Stroke::new(1.5, ACCENT));
        for (i, anchor) in pen.iter().enumerate() {
            square_handle(painter, screen.pt(anchor.pos), i == 0);
        }
    }

    fn paint_drag_preview(&self, painter: &Painter, screen: &Screen, input: &CanvasInput) {
        let Some(pos) = input.pos else { return };
        let stroke = Stroke::new(1.0, ACCENT);
        match &self.canvas.drag {
            Drag::Marquee { start } => {
                let area = evolve_engine::geometry::Rect::from_points(*start, pos);
                let corners = screen.pts(&area.corners());
                painter.add(UiShape::convex_polygon(corners, ACCENT_SOFT, stroke));
            }
            Drag::Shape { start } => {
                let rect = drag_rect(*start, self.snap(pos), input.shift, input.alt);
                let path = match self.tool {
                    Tool::Ellipse => Path::ellipse(rect.center(), rect.size() * 0.5),
                    _ => Path::rect(rect.min, rect.max),
                };
                self.paint_polylines(painter, screen, &path.flatten(0.5 / self.camera.zoom), Stroke::new(1.0, GUIDE));
            }
            Drag::Freehand { points } => {
                painter.add(UiShape::line(screen.pts(points), Stroke::new(1.5, ACCENT)));
            }
            _ => {}
        }
    }
}
