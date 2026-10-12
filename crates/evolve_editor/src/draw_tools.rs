//! Drawing tools: Pen (Bézier), Rectangle, Ellipse and Pencil (freehand).

use crate::app::EditorApp;
use crate::freehand;
use crate::tools::{CanvasInput, Drag};
use evolve_engine::geometry::{Rect, Vec2};
use evolve_engine::path::{Anchor, Path, SubPath};

/// Distance (screen points) within which a click on the first pen point closes the path.
const CLOSE_RADIUS: f32 = 8.0;

/// Rectangle spanned by a drag from `a` to `b`. Shift keeps it square, Alt grows it from the center.
pub fn drag_rect(a: Vec2, b: Vec2, square: bool, from_center: bool) -> Rect {
    let mut delta = b - a;
    if square {
        let side = delta.x.abs().max(delta.y.abs());
        delta = Vec2::new(side.copysign(delta.x), side.copysign(delta.y));
    }
    if from_center {
        Rect::from_points(a - delta, a + delta)
    } else {
        Rect::from_points(a, a + delta)
    }
}

/// Snaps `p` to 15° steps around `from`.
fn constrain_angle(from: Vec2, p: Vec2) -> Vec2 {
    let delta = p - from;
    let step = 15f32.to_radians();
    let angle = (delta.to_angle() / step).round() * step;
    from + Vec2::from_angle(angle) * delta.length()
}

impl EditorApp {
    // --- Pen -----------------------------------------------------------------------------------

    pub fn pen_tool(&mut self, input: &CanvasInput) {
        let Some(pos) = input.pos else { return };
        let pos = self.pen_point(pos, input);

        if input.double_clicked {
            // The first click of the double click already placed a point.
            self.finish_pen(false);
        } else if input.drag_started {
            let origin = self.pen_point(input.origin, input);
            if self.pen_closes_at(input.screen_origin) {
                self.finish_pen(true);
            } else {
                self.canvas.pen.push(Anchor::corner(origin));
                self.canvas.drag = Drag::PenHandle;
            }
        } else if input.clicked {
            if self.pen_closes_at(input.screen) {
                self.finish_pen(true);
            } else {
                self.canvas.pen.push(Anchor::corner(pos));
            }
        }

        if input.dragging && matches!(self.canvas.drag, Drag::PenHandle) {
            if let Some(last) = self.canvas.pen.last_mut() {
                last.handle_out = pos - last.pos;
                last.handle_in = -last.handle_out;
            }
        }
        if input.drag_stopped {
            self.canvas.drag = Drag::None;
        }
    }

    fn pen_point(&self, p: Vec2, input: &CanvasInput) -> Vec2 {
        let p = self.snap(p);
        match (input.shift, self.canvas.pen.last()) {
            (true, Some(last)) => constrain_angle(last.pos, p),
            _ => p,
        }
    }

    fn pen_closes_at(&self, screen: Vec2) -> bool {
        self.canvas.pen.len() >= 2
            && self.canvas.pen.first().is_some_and(|a| self.camera.world_to_screen(a.pos).distance(screen) <= CLOSE_RADIUS)
    }

    /// Turns the points placed so far into a shape.
    pub fn finish_pen(&mut self, closed: bool) {
        let mut anchors = std::mem::take(&mut self.canvas.pen);
        // A double click leaves a duplicate point at the end.
        if anchors.len() >= 2 && anchors[anchors.len() - 1].pos == anchors[anchors.len() - 2].pos {
            anchors.pop();
        }
        self.canvas.drag = Drag::None;
        if anchors.len() >= 2 {
            self.add_shape_node(Path { subpaths: vec![SubPath { anchors, closed }] }, "Path");
        }
    }

    pub fn cancel_pen(&mut self) {
        self.canvas.pen.clear();
    }

    // --- Rectangle / Ellipse -------------------------------------------------------------------

    pub fn shape_tool(&mut self, input: &CanvasInput, ellipse: bool) {
        if input.drag_started {
            self.canvas.drag = Drag::Shape { start: self.snap(input.origin) };
        }
        if !input.drag_stopped {
            return;
        }
        let drag = std::mem::take(&mut self.canvas.drag);
        let (Drag::Shape { start }, Some(pos)) = (drag, input.pos) else { return };
        let rect = drag_rect(start, self.snap(pos), input.shift, input.alt);
        if rect.size().min_element() * self.camera.zoom < 2.0 {
            return; // an accidental click, not a drag
        }
        if ellipse {
            self.add_shape_node(Path::ellipse(rect.center(), rect.size() * 0.5), "Ellipse");
        } else {
            self.add_shape_node(Path::rect(rect.min, rect.max), "Rectangle");
        }
    }

    // --- Pencil --------------------------------------------------------------------------------

    pub fn pencil_tool(&mut self, input: &CanvasInput) {
        if input.drag_started {
            self.canvas.drag = Drag::Freehand { points: vec![input.origin] };
        }
        if let (Drag::Freehand { points }, Some(pos)) = (&mut self.canvas.drag, input.pos) {
            let min_step = 1.5 / self.camera.zoom;
            if points.last().is_none_or(|last| last.distance(pos) >= min_step) {
                points.push(pos);
            }
        }
        if !input.drag_stopped {
            return;
        }
        if let Drag::Freehand { points } = std::mem::take(&mut self.canvas.drag) {
            let simplified = freehand::simplify(&points, 2.0 / self.camera.zoom);
            if simplified.len() >= 2 {
                let path = Path { subpaths: vec![freehand::smooth_curve(&simplified)] };
                self.add_shape_node(path, "Drawing");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_makes_the_rect_square() {
        let rect = drag_rect(Vec2::ZERO, Vec2::new(10.0, -4.0), true, false);
        assert_eq!(rect.size(), Vec2::new(10.0, 10.0));
        assert_eq!(rect.min, Vec2::new(0.0, -10.0));
    }

    #[test]
    fn alt_grows_the_rect_from_its_center() {
        let rect = drag_rect(Vec2::ZERO, Vec2::new(3.0, 2.0), false, true);
        assert_eq!(rect.center(), Vec2::ZERO);
        assert_eq!(rect.size(), Vec2::new(6.0, 4.0));
    }
}
