//! A vector path in the draw list: straight segments and Bezier curves, no engine types.

use crate::geometry::Point;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    MoveTo(Point),
    LineTo(Point),
    /// Quadratic curve: control point, end point.
    QuadTo(Point, Point),
    /// Cubic curve: two control points, end point.
    CubicTo(Point, Point, Point),
    /// Closes the current contour.
    Close,
}

/// A sequence of contours. Fill rule: non-zero, as for glyphs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiPath {
    pub cmds: Vec<PathCmd>,
}

impl UiPath {
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn move_to(&mut self, p: Point) {
        self.cmds.push(PathCmd::MoveTo(p));
    }

    pub fn line_to(&mut self, p: Point) {
        self.cmds.push(PathCmd::LineTo(p));
    }

    pub fn quad_to(&mut self, control: Point, to: Point) {
        self.cmds.push(PathCmd::QuadTo(control, to));
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, to: Point) {
        self.cmds.push(PathCmd::CubicTo(c1, c2, to));
    }

    pub fn close(&mut self) {
        self.cmds.push(PathCmd::Close);
    }
}
