//! Boolean operations on paths (union, difference, intersection, exclusion).
//!
//! Curves are flattened to fine polylines first, so results are made of short straight
//! segments (accurate to `FLATTEN_TOLERANCE`), not Bézier curves.

use crate::path::{Anchor, Path, SubPath};
use crate::style::FillRule;
use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

/// Maximum distance between a curve and its polyline approximation, in path units.
const FLATTEN_TOLERANCE: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    /// Subject minus clip.
    Difference,
    Intersection,
    Exclusion,
}

impl BoolOp {
    fn rule(self) -> OverlayRule {
        match self {
            BoolOp::Union => OverlayRule::Union,
            BoolOp::Difference => OverlayRule::Difference,
            BoolOp::Intersection => OverlayRule::Intersect,
            BoolOp::Exclusion => OverlayRule::Xor,
        }
    }
}

fn contours(path: &Path) -> Vec<Vec<[f32; 2]>> {
    path.flatten(FLATTEN_TOLERANCE)
        .into_iter()
        .map(|poly| {
            let mut points: Vec<[f32; 2]> = poly.points.iter().map(|p| [p.x, p.y]).collect();
            // Closed polylines repeat their first point; the overlay engine closes contours itself.
            if poly.closed && points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            points
        })
        .filter(|c| c.len() >= 3)
        .collect()
}

/// Applies `op` to two paths that share a coordinate space. Open subpaths count as closed.
pub fn combine(subject: &Path, subject_rule: FillRule, clip: &Path, clip_rule: FillRule, op: BoolOp) -> Path {
    let fill = |rule| match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    // The overlay engine takes one fill rule for both operands, so normalize the clip first
    // when the rules differ by resolving it on its own.
    let clip_contours = if clip_rule == subject_rule {
        contours(clip)
    } else {
        resolve(&contours(clip), clip_rule)
    };
    let result = contours(subject).overlay(&clip_contours, op.rule(), fill(subject_rule));

    let subpaths = result
        .into_iter()
        .flatten()
        .map(|contour| SubPath { anchors: contour.iter().map(|p| Anchor::corner((*p).into())).collect(), closed: true })
        .collect();
    Path { subpaths }
}

/// Re-expresses contours drawn with `rule` as an even-odd-safe shape set.
fn resolve(contours: &[Vec<[f32; 2]>], rule: FillRule) -> Vec<Vec<[f32; 2]>> {
    let empty: Vec<Vec<[f32; 2]>> = Vec::new();
    let fill = match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    contours.to_vec().overlay(&empty, OverlayRule::Union, fill).into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec2;

    fn square(x: f32, y: f32, size: f32) -> Path {
        Path::rect(Vec2::new(x, y), Vec2::new(x + size, y + size))
    }

    fn run(op: BoolOp) -> Path {
        combine(&square(0.0, 0.0, 10.0), FillRule::NonZero, &square(5.0, 5.0, 10.0), FillRule::NonZero, op)
    }

    #[test]
    fn union_covers_both_squares() {
        let path = run(BoolOp::Union);
        assert!(path.contains(Vec2::new(2.0, 2.0), FillRule::EvenOdd, 0.1));
        assert!(path.contains(Vec2::new(12.0, 12.0), FillRule::EvenOdd, 0.1));
        assert!(!path.contains(Vec2::new(12.0, 2.0), FillRule::EvenOdd, 0.1));
    }

    #[test]
    fn intersection_keeps_only_the_overlap() {
        let path = run(BoolOp::Intersection);
        assert!(path.contains(Vec2::new(7.0, 7.0), FillRule::EvenOdd, 0.1));
        assert!(!path.contains(Vec2::new(2.0, 2.0), FillRule::EvenOdd, 0.1));
    }

    #[test]
    fn difference_cuts_the_clip_out() {
        let path = run(BoolOp::Difference);
        assert!(path.contains(Vec2::new(2.0, 2.0), FillRule::EvenOdd, 0.1));
        assert!(!path.contains(Vec2::new(7.0, 7.0), FillRule::EvenOdd, 0.1));
    }

    #[test]
    fn exclusion_removes_the_overlap_only() {
        let path = run(BoolOp::Exclusion);
        assert!(path.contains(Vec2::new(2.0, 2.0), FillRule::EvenOdd, 0.1));
        assert!(path.contains(Vec2::new(12.0, 12.0), FillRule::EvenOdd, 0.1));
        assert!(!path.contains(Vec2::new(7.0, 7.0), FillRule::EvenOdd, 0.1));
    }
}
