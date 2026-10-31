//! Draw-list geometry as engine paths.

use evolve_engine::geometry::Vec2;
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_ui_core::{PathCmd, Point, Rect, UiPath};

/// Control-point distance that makes a cubic arc best approximate a quarter circle.
const KAPPA: f32 = 0.552_284_7;

fn vec(p: Point) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// A rectangle with a radius per corner (top-left, top-right, bottom-right, bottom-left). A radius is cut to what fits:
/// no more than half of the shorter side, and neighbouring radii never overlap.
pub(crate) fn rounded_rect(rect: Rect, radii: [f32; 4]) -> Path {
    let (x, y, r, b) = (rect.x, rect.y, rect.right(), rect.bottom());
    let (w, h) = (rect.w.max(0.0), rect.h.max(0.0));
    let [mut tl, mut tr, mut br, mut bl] = radii.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    // Two neighbours on one side share that side: scale all radii down together when a side is too short for them.
    let scale = [(tl + tr, w), (bl + br, w), (tl + bl, h), (tr + br, h)]
        .into_iter()
        .filter(|(sum, _)| *sum > 0.0)
        .map(|(sum, side)| side / sum)
        .fold(1.0f32, f32::min);
    if scale < 1.0 {
        [tl, tr, br, bl] = [tl, tr, br, bl].map(|v| v * scale);
    }

    let mut anchors: Vec<Anchor> = Vec::with_capacity(8);
    // For each corner: the point where the arc starts on the incoming edge, the corner, and where it ends on the outgoing edge.
    let mut corner = |radius: f32, entry: Vec2, corner: Vec2, exit: Vec2| {
        if radius < 1e-4 {
            anchors.push(Anchor::corner(corner));
        } else {
            anchors.push(Anchor { pos: entry, handle_in: Vec2::ZERO, handle_out: (corner - entry) * KAPPA });
            anchors.push(Anchor { pos: exit, handle_in: (corner - exit) * KAPPA, handle_out: Vec2::ZERO });
        }
    };
    corner(tl, Vec2::new(x, y + tl), Vec2::new(x, y), Vec2::new(x + tl, y));
    corner(tr, Vec2::new(r - tr, y), Vec2::new(r, y), Vec2::new(r, y + tr));
    corner(br, Vec2::new(r, b - br), Vec2::new(r, b), Vec2::new(r - br, b));
    corner(bl, Vec2::new(x + bl, b), Vec2::new(x, b), Vec2::new(x, b - bl));
    Path { subpaths: vec![SubPath { anchors, closed: true }] }
}

/// The outline of a draw-list path as an engine path. Quadratic curves are raised to cubic ones, which is exact.
pub(crate) fn convert_path(path: &UiPath) -> Path {
    let mut subpaths: Vec<SubPath> = Vec::new();
    let mut current: Vec<Anchor> = Vec::new();

    let finish = |current: &mut Vec<Anchor>, subpaths: &mut Vec<SubPath>, closed: bool| {
        let mut anchors = std::mem::take(current);
        if closed && anchors.len() > 1 {
            // A contour that comes back to its start with a last curve: the last anchor is the first one again.
            let (first, last) = (anchors[0], anchors[anchors.len() - 1]);
            if first.pos == last.pos {
                anchors[0].handle_in = last.handle_in;
                anchors.pop();
            }
        }
        if anchors.len() > 1 {
            subpaths.push(SubPath { anchors, closed });
        }
    };
    // Sets the outgoing handle of the anchor we are leaving, then adds the next one with its incoming handle.
    let curve_to = |current: &mut Vec<Anchor>, c1: Vec2, c2: Vec2, to: Vec2| {
        if let Some(last) = current.last_mut() {
            last.handle_out = c1 - last.pos;
        }
        current.push(Anchor { pos: to, handle_in: c2 - to, handle_out: Vec2::ZERO });
    };

    for cmd in &path.cmds {
        match *cmd {
            PathCmd::MoveTo(p) => {
                finish(&mut current, &mut subpaths, false);
                current.push(Anchor::corner(vec(p)));
            }
            PathCmd::LineTo(p) => current.push(Anchor::corner(vec(p))),
            PathCmd::QuadTo(c, p) => {
                if let Some(from) = current.last().map(|a| a.pos) {
                    let (c, to) = (vec(c), vec(p));
                    curve_to(&mut current, from + (c - from) * (2.0 / 3.0), to + (c - to) * (2.0 / 3.0), to);
                }
            }
            PathCmd::CubicTo(c1, c2, p) => curve_to(&mut current, vec(c1), vec(c2), vec(p)),
            PathCmd::Close => finish(&mut current, &mut subpaths, true),
        }
    }
    finish(&mut current, &mut subpaths, false);
    Path { subpaths }
}
