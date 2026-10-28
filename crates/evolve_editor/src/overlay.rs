//! Editor-only drawing on top of the rendered scene: grid, selection, handles, tool previews.
//! These are drawn with egui at screen resolution, so they stay crisp at any zoom.

use crate::app::EditorApp;
use crate::draw_tools::drag_rect;
use crate::theme::{ACCENT, ACCENT_SOFT, GUIDE, HANDLE_FILL};
use crate::tools::{CanvasInput, Drag, Tool};
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape as UiShape, Stroke, StrokeKind};
use evolve_engine::camera::Camera;
use evolve_engine::game_camera::CameraView;
use evolve_engine::geometry::{Affine2, Rect as WorldRect, Vec2};
use evolve_engine::path::{Path, Polyline};
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use std::collections::HashSet;

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

    /// Many points at once: the camera's matrix is made once (`pt` makes it, with a sine and a cosine, for every point: 5 ms for the
    /// 230,000 points of the map's outlines).
    pub fn pts(&self, world: &[Vec2]) -> Vec<Pos2> {
        let matrix = self.camera.view_matrix();
        world
            .iter()
            .map(|&p| {
                let s = matrix.transform_point2(p);
                Pos2::new(self.origin.x + s.x, self.origin.y + s.y)
            })
            .collect()
    }

    /// The box in the world that a rectangle of the screen shows, with a little room around it (outlines are a pixel wide).
    pub fn visible_world(&self, clip: Rect) -> Option<WorldRect> {
        let to_world = |p: Pos2| self.camera.screen_to_world(Vec2::new(p.x - self.origin.x, p.y - self.origin.y));
        WorldRect::from_cloud([clip.left_top(), clip.right_top(), clip.right_bottom(), clip.left_bottom()].map(to_world)).map(|rect| rect.expanded(2.0 / self.camera.zoom))
    }
}

/// Points of an outline on screen closer than this to the one before are dropped.
const MIN_OUTLINE_STEP: f32 = 0.5;
/// Most points of selection outlines egui is asked to draw in a frame. A selection of thousands of objects (the whole map) has hundreds
/// of thousands, far more than the screen can show, and egui's triangles for them cost 12 to 16 ms a frame (2026-10-24): when there are
/// more, the step between points grows in proportion, up to `MAX_OUTLINE_STEP`.
const OUTLINE_POINT_BUDGET: usize = 30_000;
const MAX_OUTLINE_STEP: f32 = 4.0;

/// An outline through `points` (screen positions), drawn so that a sharp corner never shoots a spike across the view.
///
/// egui joins the corners of a *closed* line with miters of any length (epaint's `add_line_loop` does not cut sharp corners),
/// so where an outline turns back on itself, or where two points nearly coincide on screen and the direction between them
/// is noise, the join reached far outside the shape: selecting the provinces of a map drew long straight lines through the
/// whole view (2026-10-16). Points closer than half a pixel are dropped, and a closed outline is drawn as an open line that
/// comes back to its start, whose sharp corners egui does cut.
pub(crate) fn outline_shape(points: &[Pos2], closed: bool, stroke: Stroke) -> UiShape {
    outline_shape_with_step(points, closed, stroke, MIN_OUTLINE_STEP)
}

/// [`outline_shape`] with points less than `step` apart dropped.
fn outline_shape_with_step(points: &[Pos2], closed: bool, stroke: Stroke, step: f32) -> UiShape {
    let mut kept: Vec<Pos2> = Vec::with_capacity(points.len() + 1);
    for &p in points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()) {
        if kept.last().is_none_or(|last| last.distance(p) >= step) {
            kept.push(p);
        }
    }
    if closed {
        while kept.len() > 2 && kept[kept.len() - 1].distance(kept[0]) < step {
            kept.pop();
        }
        if kept.len() > 2 {
            kept.push(kept[0]);
        }
    }
    if kept.len() < 2 {
        return UiShape::Noop;
    }
    UiShape::line(kept, stroke)
}

/// Whether a shape lies wholly outside `visible` (a box in the world): the box of its anchors and handles, with its corners moved by
/// `world`, does not meet it. A shape bent by bones, or with a point that is not a number, is never counted out.
fn is_out_of_view(shape: &Shape, world: &Affine2, visible: &WorldRect) -> bool {
    if shape.skin.is_some() {
        return false;
    }
    let Some(local) = shape.path.control_bounds() else { return false };
    WorldRect::from_cloud(local.corners().map(|corner| world.transform_point2(corner))).is_some_and(|bounds| !bounds.intersects(visible))
}

/// The outline of a shape in the world: each subpath flattened in the shape's own coordinates (finely enough for the scale of `world`), then
/// its points moved. Moving the path first copied all of it, for every shape.
fn world_outline(path: &Path, world: &Affine2, tolerance: f32, out: &mut Vec<Polyline>) {
    let scale = (world.matrix2.x_axis.length_squared() + world.matrix2.y_axis.length_squared()).sqrt().max(1e-6);
    for subpath in &path.subpaths {
        let mut line = subpath.flatten(tolerance / scale);
        for point in &mut line.points {
            *point = world.transform_point2(*point);
        }
        out.push(line);
    }
}

fn collect_outlines(scene: &Scene, node: &Node, world: Affine2, tolerance: f32, visible: Option<&WorldRect>, out: &mut Vec<Polyline>) {
    match &node.kind {
        NodeKind::Shape(shape) => {
            if !visible.is_some_and(|visible| is_out_of_view(shape, &world, visible)) {
                world_outline(&shape.effective_path(scene, world), &world, tolerance, out);
            }
        }
        NodeKind::Group(children) => {
            for child in children {
                collect_outlines(scene, child, world * child.transform, tolerance, visible, out);
            }
        }
        NodeKind::Instance(instance) => {
            if let Some(symbol) = scene.symbol(instance.symbol) {
                collect_outlines(scene, &symbol.root, world * symbol.root.transform, tolerance, visible, out);
            }
        }
        NodeKind::Camera(camera) => out.extend(CameraView::new(camera, &world).outline().flatten(tolerance)),
    }
}

/// The egui shapes of `polylines` (world points, drawn through `screen`) inside `clip`: an outline that is wholly outside it is left out,
/// and when what is left has more than `OUTLINE_POINT_BUDGET` points the points are thinned out more than usual (the step grows up to `MAX_OUTLINE_STEP`), so that the cost of a
/// frame does not grow with the size of the selection.
pub(crate) fn outline_shapes(polylines: &[Polyline], screen: &Screen, clip: Rect, stroke: Stroke) -> Vec<UiShape> {
    let reach = clip.expand(stroke.width);
    let mut seen: Vec<(Vec<Pos2>, bool)> = Vec::new();
    for polyline in polylines {
        let points = screen.pts(&polyline.points);
        let bounds = points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).fold(Rect::NOTHING, |bounds, &p| bounds.union(Rect::from_min_max(p, p)));
        if reach.intersects(bounds) {
            seen.push((points, polyline.closed));
        }
    }
    let total: usize = seen.iter().map(|(points, _)| points.len()).sum();
    let step = (MIN_OUTLINE_STEP * total as f32 / OUTLINE_POINT_BUDGET as f32).clamp(MIN_OUTLINE_STEP, MAX_OUTLINE_STEP);
    seen.into_iter().map(|(points, closed)| outline_shape_with_step(&points, closed, stroke, step)).collect()
}

/// The outlines of the selected nodes, found in ONE walk of the scene with the world transform carried down. Looking each id up and asking
/// for its world transform searches the tree from the top every time, which made the outline of the 4,640 objects of the map (all of them
/// selected) take 70 ms a frame (2026-10-24). A selected node's outline holds everything below it, so the walk does not go on into it
/// (a group and its child both selected are outlined once).
/// With `visible` (a box in the world), shapes wholly outside it are not flattened at all.
pub(crate) fn selection_outlines(scene: &Scene, selection: &[NodeId], tolerance: f32, visible: Option<&WorldRect>) -> Vec<Polyline> {
    let selected: HashSet<NodeId> = selection.iter().copied().collect();
    let mut outlines = Vec::new();
    if !selected.is_empty() {
        collect_selected(scene, &scene.nodes, Affine2::IDENTITY, &selected, tolerance, visible, &mut outlines);
    }
    outlines
}

fn collect_selected(scene: &Scene, nodes: &[Node], parent_world: Affine2, selected: &HashSet<NodeId>, tolerance: f32, visible: Option<&WorldRect>, out: &mut Vec<Polyline>) {
    for node in nodes {
        let world = parent_world * node.transform;
        if selected.contains(&node.id) {
            collect_outlines(scene, node, world, tolerance, visible, out);
        } else if let NodeKind::Group(children) = &node.kind {
            collect_selected(scene, children, world, selected, tolerance, visible, out);
        }
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
        painter.extend(outline_shapes(polylines, screen, painter.clip_rect(), stroke));
    }

    fn paint_selection_outline(&self, painter: &Painter, screen: &Screen) {
        let visible = screen.visible_world(painter.clip_rect());
        let outlines = selection_outlines(&self.scene, &self.selection, 0.5 / self.camera.zoom, visible.as_ref());
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

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::epaint::{Mesh, TessellationOptions, Tessellator};
    use evolve_engine::path::{Anchor, SubPath};
    use std::time::Instant;

    /// The box of the triangles egui makes for `shape`.
    fn drawn_box(shape: UiShape) -> Rect {
        let mut tessellator = Tessellator::new(1.0, TessellationOptions::default(), [1, 1], Vec::new());
        let mut mesh = Mesh::default();
        tessellator.tessellate_shape(shape, &mut mesh);
        mesh.vertices.iter().fold(Rect::NOTHING, |r, v| r.union(Rect::from_min_max(v.pos, v.pos)))
    }

    #[test]
    fn an_outline_that_turns_back_or_has_points_on_top_of_each_other_stays_near_its_points() {
        let p = |x: f32, y: f32| Pos2::new(x, y);
        // A province-like outline with what real outlines have: an exact turn back (at 200, 10), a turn back that is almost
        // exact (at 300, 100), and points a tiny fraction of a pixel apart (at 10, 100) whose direction is noise.
        let points = [p(10.0, 10.0), p(200.0, 10.0), p(60.0, 10.0), p(60.0, 100.0), p(300.0, 100.02), p(61.0, 100.05), p(10.0, 100.0), p(10.0001, 100.0003), p(10.0, 99.9999), p(10.0, 50.0)];
        let points_box = Rect::from_points(&points);
        for closed in [true, false] {
            let drawn = drawn_box(outline_shape(&points, closed, Stroke::new(1.0, ACCENT)));
            assert!(points_box.expand(3.0).contains_rect(drawn), "closed {closed}: drawn over {drawn:?}, the points are in {points_box:?}");
            assert!(drawn.width() >= points_box.width() && drawn.height() >= points_box.height(), "closed {closed}: the whole outline is drawn");
        }
        // Nothing to draw: no points, one point, points all in one place, points that are not numbers.
        assert_eq!(drawn_box(outline_shape(&[], true, Stroke::new(1.0, ACCENT))), Rect::NOTHING);
        assert_eq!(drawn_box(outline_shape(&[p(5.0, 5.0), p(5.1, 5.1), p(5.0, 5.2)], true, Stroke::new(1.0, ACCENT))), Rect::NOTHING);
        let with_nan = [p(0.0, 0.0), p(f32::NAN, 3.0), p(50.0, 0.0), p(50.0, 50.0)];
        let drawn = drawn_box(outline_shape(&with_nan, true, Stroke::new(1.0, ACCENT)));
        assert!(Rect::from_min_max(p(0.0, 0.0), p(50.0, 50.0)).expand(3.0).contains_rect(drawn), "{drawn:?}");
    }

    fn square(scene: &mut Scene, name: &str, at: f32, shift: Vec2) -> Node {
        let corners = [(at, at), (at + 10.0, at), (at + 10.0, at + 10.0), (at, at + 10.0)].map(|(x, y)| Anchor::corner(Vec2::new(x, y)));
        let mut node = scene.new_node(name, NodeKind::Shape(Shape::new(Path { subpaths: vec![SubPath { anchors: corners.to_vec(), closed: true }] })));
        node.transform = Affine2::from_translation(shift);
        node
    }

    /// A (a shape), G (a moved and scaled group) holding B (a shape) and H (a group) holding C (a shape), and D (a shape); every level moves.
    fn a_scene_with_levels() -> (Scene, [NodeId; 6]) {
        let mut scene = Scene::with_camera(Vec2::new(500.0, 500.0));
        let a = square(&mut scene, "A", 0.0, Vec2::new(5.0, 0.0));
        let c = square(&mut scene, "C", 30.0, Vec2::new(2.0, 3.0));
        let mut h = scene.new_node("H", NodeKind::Group(vec![c]));
        h.transform = Affine2::from_translation(Vec2::new(7.0, 9.0)) * Affine2::from_scale(Vec2::splat(0.5));
        let b = square(&mut scene, "B", 10.0, Vec2::new(1.0, 1.0));
        let mut g = scene.new_node("G", NodeKind::Group(vec![b, h]));
        g.transform = Affine2::from_translation(Vec2::new(100.0, 50.0)) * Affine2::from_scale(Vec2::splat(2.0));
        let d = square(&mut scene, "D", 50.0, Vec2::ZERO);
        let ids = [a.id, g.id, g.children()[0].id, g.children()[1].id, g.children()[1].children()[0].id, d.id];
        for node in [a, g, d] {
            scene.add(node, None);
        }
        (scene, ids)
    }

    /// What the outlines were before: each id looked up, and its world transform asked for, one by one.
    fn one_by_one(scene: &Scene, selection: &[NodeId], tolerance: f32) -> Vec<Polyline> {
        let mut outlines = Vec::new();
        for &id in selection {
            if let (Some(node), Some(world)) = (scene.find(id), scene.world_transform(id)) {
                collect_outlines(scene, node, world, tolerance, None, &mut outlines);
            }
        }
        outlines
    }

    fn as_text(mut outlines: Vec<Polyline>) -> Vec<String> {
        let mut text: Vec<String> = outlines.drain(..).map(|line| format!("{:?} {}", line.points, line.closed)).collect();
        text.sort();
        text
    }

    #[test]
    fn the_outlines_of_a_selection_are_those_of_each_selected_node_at_its_place_in_the_scene() {
        let (scene, [a, g, b, h, c, d]) = a_scene_with_levels();
        for selection in [vec![c], vec![b], vec![a, d], vec![g], vec![h, a], vec![c, b, d, a], vec![d, a]] {
            let found = as_text(selection_outlines(&scene, &selection, 0.1, None));
            assert!(!found.is_empty(), "{selection:?} has an outline");
            assert_eq!(found, as_text(one_by_one(&scene, &selection, 0.1)), "{selection:?}");
        }
        // The level counts: C is moved by its own transform, H's and G's.
        let outline = &selection_outlines(&scene, &[c], 0.1, None)[0];
        let at = Affine2::from_translation(Vec2::new(100.0, 50.0)) * Affine2::from_scale(Vec2::splat(2.0)) * Affine2::from_translation(Vec2::new(7.0, 9.0)) * Affine2::from_scale(Vec2::splat(0.5)) * Affine2::from_translation(Vec2::new(2.0, 3.0));
        assert!(outline.points.iter().any(|p| p.distance(at.transform_point2(Vec2::new(30.0, 30.0))) < 1e-3), "{:?}", outline.points);
    }

    #[test]
    fn a_selected_group_is_outlined_once_even_when_what_is_in_it_is_selected_too() {
        let (scene, [a, g, b, h, c, _]) = a_scene_with_levels();
        let group_alone = as_text(selection_outlines(&scene, &[g], 0.1, None));
        assert_eq!(group_alone.len(), 2, "B and C");
        assert_eq!(as_text(selection_outlines(&scene, &[g, b, h, c], 0.1, None)), group_alone);
        assert_eq!(as_text(selection_outlines(&scene, &[b, g, g, c], 0.1, None)), group_alone, "in any order, and a node twice is one");
        assert_eq!(selection_outlines(&scene, &[a, a], 0.1, None).len(), 1);
    }

    #[test]
    fn shapes_wholly_outside_the_view_are_not_flattened_and_the_others_are_the_same_as_without_a_view() {
        let (scene, [a, g, b, _, c, d]) = a_scene_with_levels();
        let everything = [a, g, d];
        let all = as_text(selection_outlines(&scene, &everything, 0.1, None));
        assert_eq!(all.len(), 4, "A, B, C and D");
        let world = |min: (f32, f32), max: (f32, f32)| WorldRect { min: Vec2::new(min.0, min.1), max: Vec2::new(max.0, max.1) };
        // A is at 5..15 (x), B at 100 + 2 * (11..21) = 122..142, C near 100 + 2 * (7 + 0.5 * 32) = 146..156, D at 50..60.
        assert_eq!(as_text(selection_outlines(&scene, &everything, 0.1, Some(&world((-1000.0, -1000.0), (1000.0, 1000.0))))), all, "a view that shows all");
        assert_eq!(selection_outlines(&scene, &everything, 0.1, Some(&world((3000.0, 3000.0), (4000.0, 4000.0)))).len(), 0, "a view of somewhere else");
        let near_d = selection_outlines(&scene, &everything, 0.1, Some(&world((52.0, 52.0), (53.0, 53.0))));
        assert_eq!(near_d.len(), 1, "only D reaches there");
        assert_eq!(as_text(near_d), as_text(selection_outlines(&scene, &[d], 0.1, None)));
        // What is selected inside a group that is not selected is walked down to, with the same view.
        assert_eq!(selection_outlines(&scene, &[b, c], 0.1, None).len(), 2);
        assert!(selection_outlines(&scene, &[b, c], 0.1, Some(&world((3000.0, 3000.0), (4000.0, 4000.0)))).is_empty());
        // Touching counts: the view's edge on the shape's edge.
        assert_eq!(selection_outlines(&scene, &[d], 0.1, Some(&world((60.0, 60.0), (70.0, 70.0)))).len(), 1);
        // A point that is not a number is never counted out, whatever the view.
        let mut broken = Scene::with_camera(Vec2::new(10.0, 10.0));
        let mut node = square(&mut broken, "N", 0.0, Vec2::ZERO);
        if let NodeKind::Shape(shape) = &mut node.kind {
            shape.path.subpaths[0].anchors[1].pos.x = f32::NAN;
        }
        let id = node.id;
        broken.add(node, None);
        assert!(!selection_outlines(&broken, &[id], 0.1, Some(&world((3000.0, 3000.0), (4000.0, 4000.0)))).is_empty());
    }

    #[test]
    fn the_view_a_rectangle_of_the_screen_shows_is_the_box_of_its_corners_with_room_around() {
        let camera = Camera { center: Vec2::new(400.0, 300.0), zoom: 2.0, rotation: 0.0, viewport: Vec2::new(800.0, 600.0) };
        let screen = Screen { camera: &camera, origin: Pos2::new(10.0, 20.0) };
        let seen = screen.visible_world(Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(810.0, 620.0))).unwrap();
        // 800 x 600 screen points at twice the size show 400 x 300 units around (400, 300), and 2 pixels = 1 unit of room.
        assert!((seen.min - Vec2::new(199.0, 149.0)).abs().max_element() < 1e-3, "{seen:?}");
        assert!((seen.max - Vec2::new(601.0, 451.0)).abs().max_element() < 1e-3, "{seen:?}");
        let turned = Camera { rotation: std::f32::consts::FRAC_PI_2, ..camera.clone() };
        let screen = Screen { camera: &turned, origin: Pos2::new(10.0, 20.0) };
        let seen = screen.visible_world(Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(810.0, 620.0))).unwrap();
        assert!((seen.size() - Vec2::new(300.0 + 2.0, 400.0 + 2.0)).abs().max_element() < 1e-2, "a quarter turn swaps the sides: {seen:?}");
    }

    #[test]
    fn many_points_at_once_land_where_one_at_a_time_puts_them() {
        let camera = Camera { center: Vec2::new(33.0, -12.0), zoom: 3.7, rotation: 0.4, viewport: Vec2::new(800.0, 600.0) };
        let screen = Screen { camera: &camera, origin: Pos2::new(5.0, 7.0) };
        let points = [Vec2::new(0.0, 0.0), Vec2::new(100.0, -50.0), Vec2::new(-3.5, 8.25)];
        for (many, one) in screen.pts(&points).iter().zip(points.map(|p| screen.pt(p))) {
            assert!(many.distance(one) < 1e-3, "{many:?} against {one:?}");
        }
    }

    #[test]
    fn nothing_selected_or_nothing_there_has_no_outline() {
        let (scene, _) = a_scene_with_levels();
        assert!(selection_outlines(&scene, &[], 0.1, None).is_empty());
        assert!(selection_outlines(&scene, &[NodeId(987_654)], 0.1, None).is_empty());
        assert!(selection_outlines(&Scene::with_camera(Vec2::new(10.0, 10.0)), &[NodeId(987_654)], 0.1, None).is_empty());
    }

    fn camera_over(center: Vec2, zoom: f32) -> Camera {
        Camera { center, zoom, rotation: 0.0, viewport: Vec2::new(800.0, 600.0) }
    }

    fn points_of(shape: &UiShape) -> usize {
        match shape {
            UiShape::Path(path) => path.points.len(),
            UiShape::Noop => 0,
            other => panic!("not an outline: {other:?}"),
        }
    }

    fn polyline(points: &[(f32, f32)], closed: bool) -> Polyline {
        Polyline { points: points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(), closed }
    }

    #[test]
    fn outlines_wholly_outside_the_view_are_not_made_and_the_others_are_as_they_always_were() {
        let camera = camera_over(Vec2::new(400.0, 300.0), 1.0);
        let screen = Screen { camera: &camera, origin: Pos2::ZERO };
        let clip = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let stroke = Stroke::new(1.0, ACCENT);
        let inside = polyline(&[(100.0, 100.0), (300.0, 100.0), (300.0, 300.0)], true);
        let straddling = polyline(&[(-500.0, 100.0), (100.0, 100.0)], false);
        let outside = [polyline(&[(2000.0, 100.0), (2100.0, 100.0)], false), polyline(&[(100.0, -900.0), (200.0, -800.0), (150.0, -700.0)], true), polyline(&[(f32::NAN, 5.0), (3.0, f32::NAN)], false), polyline(&[], false)];
        let all: Vec<Polyline> = [vec![inside.clone(), straddling.clone()], outside.to_vec()].concat();
        let shapes = outline_shapes(&all, &screen, clip, stroke);
        assert_eq!(shapes.len(), 2, "only the two that can be seen");
        for (shape, line) in shapes.iter().zip([&inside, &straddling]) {
            let expected = outline_shape(&screen.pts(&line.points), line.closed, stroke);
            assert_eq!(points_of(shape), points_of(&expected));
        }
        assert!(outline_shapes(&outside, &screen, clip, stroke).is_empty());
        // Just touching the view counts as seen.
        let touching = polyline(&[(800.5, 10.0), (900.0, 10.0)], false);
        assert_eq!(outline_shapes(&[touching], &screen, clip, stroke).len(), 1);
    }

    #[test]
    fn a_selection_with_far_more_points_than_can_be_seen_is_thinned_and_a_small_one_is_not() {
        let camera = camera_over(Vec2::new(400.0, 300.0), 1.0);
        let screen = Screen { camera: &camera, origin: Pos2::ZERO };
        let clip = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let stroke = Stroke::new(1.0, ACCENT);
        // 400 wiggly lines of 1,000 points a pixel apart: 400,000 points.
        let lines: Vec<Polyline> = (0..400)
            .map(|k| polyline(&(0..1000).map(|i| (i as f32 * 0.75, 1.0 + (k as f32) * 1.4 + if i % 2 == 0 { 0.0 } else { 0.6 })).collect::<Vec<_>>(), false))
            .collect();
        let drawn: usize = outline_shapes(&lines, &screen, clip, stroke).iter().map(points_of).sum();
        assert!(drawn * 4 < 400_000, "{drawn} of 400,000 points are drawn: the step between points is at least four times as long (it stops growing at four pixels)");
        assert!(drawn >= 400 * 150, "but a drawing is left, a point about every four pixels of each line at most (150 of 1,000): {drawn} points");
        // A few lines keep the finest step.
        let few = &lines[..20];
        let finest: usize = few.iter().map(|l| points_of(&outline_shape(&screen.pts(&l.points), l.closed, stroke))).sum();
        let found: usize = outline_shapes(few, &screen, clip, stroke).iter().map(points_of).sum();
        assert_eq!(found, finest);
    }

    #[test]
    fn the_outlines_of_forty_thousand_selected_shapes_take_a_walk_of_the_scene_and_not_forty_thousand() {
        let mut scene = Scene::with_camera(Vec2::new(500.0, 500.0));
        let shapes: Vec<Node> = (0..40_000).map(|i| square(&mut scene, "S", (i % 400) as f32, Vec2::ZERO)).collect();
        let selection: Vec<NodeId> = shapes.iter().map(|n| n.id).collect();
        let group = scene.new_node("All", NodeKind::Group(shapes));
        scene.add(group, None);
        let started = Instant::now();
        let outlines = selection_outlines(&scene, &selection, 0.1, None);
        let took = started.elapsed();
        assert_eq!(outlines.len(), 40_000);
        // Looking each one up from the top is 40,000 searches of 40,000 nodes: seconds. One walk is a few milliseconds (a debug build 50).
        assert!(took.as_millis() < 1500, "{took:?}");
    }
}
