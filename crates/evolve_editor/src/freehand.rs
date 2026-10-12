//! Turns a raw pointer trail into a smooth Bézier path.

use evolve_engine::geometry::{distance_to_segment, Vec2};
use evolve_engine::path::{Anchor, SubPath};

/// Ramer–Douglas–Peucker: drops points that deviate less than `tolerance` from the simplified line.
pub fn simplify(points: &[Vec2], tolerance: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let farthest = (first + 1..last)
            .map(|i| (i, distance_to_segment(points[i], points[first], points[last])))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = farthest.filter(|&(_, d)| d > tolerance) {
            keep[index] = true;
            stack.push((first, index));
            stack.push((index, last));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Open subpath through all `points`, with Catmull-Rom style tangents so the curve looks hand-drawn, not polygonal.
pub fn smooth_curve(points: &[Vec2]) -> SubPath {
    let last = points.len().saturating_sub(1);
    let anchors = (0..points.len())
        .map(|i| {
            let tangent = (points[(i + 1).min(last)] - points[i.saturating_sub(1)]) / 6.0;
            Anchor { pos: points[i], handle_in: -tangent, handle_out: tangent }
        })
        .collect();
    SubPath { anchors, closed: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let points = [Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        let simple = simplify(&points, 0.5);
        assert_eq!(simple, vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]);
    }

    #[test]
    fn smooth_curve_passes_through_every_point() {
        let points = [Vec2::ZERO, Vec2::new(10.0, 5.0), Vec2::new(20.0, 0.0)];
        let curve = smooth_curve(&points);
        assert_eq!(curve.anchors.iter().map(|a| a.pos).collect::<Vec<_>>(), points);
    }
}
