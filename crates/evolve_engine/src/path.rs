//! Bézier paths made of anchor points with in/out control handles.

use crate::geometry::{distance_to_segment, Affine2, Rect, Vec2};
use crate::style::FillRule;
use serde::{Deserialize, Serialize};

/// An anchor with its two control handles, stored as offsets from `pos`.
/// Both handles at zero means a sharp corner between straight lines.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub pos: Vec2,
    pub handle_in: Vec2,
    pub handle_out: Vec2,
}

impl Anchor {
    pub fn corner(pos: Vec2) -> Self {
        Self { pos, handle_in: Vec2::ZERO, handle_out: Vec2::ZERO }
    }

    /// Smooth anchor: the handles are mirrored around `pos`.
    pub fn smooth(pos: Vec2, handle_out: Vec2) -> Self {
        Self { pos, handle_in: -handle_out, handle_out }
    }
}

/// One cubic Bézier segment.
#[derive(Clone, Copy, Debug)]
pub struct Cubic {
    pub p0: Vec2,
    pub c1: Vec2,
    pub c2: Vec2,
    pub p3: Vec2,
}

impl Cubic {
    pub fn point(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        self.p0 * (u * u * u) + self.c1 * (3.0 * u * u * t) + self.c2 * (3.0 * u * t * t) + self.p3 * (t * t * t)
    }

    /// De Casteljau subdivision at `t`.
    pub fn split(&self, t: f32) -> (Cubic, Cubic) {
        let p01 = self.p0.lerp(self.c1, t);
        let p12 = self.c1.lerp(self.c2, t);
        let p23 = self.c2.lerp(self.p3, t);
        let p012 = p01.lerp(p12, t);
        let p123 = p12.lerp(p23, t);
        let mid = p012.lerp(p123, t);
        (
            Cubic { p0: self.p0, c1: p01, c2: p012, p3: mid },
            Cubic { p0: mid, c1: p123, c2: p23, p3: self.p3 },
        )
    }

    pub fn is_line(&self) -> bool {
        self.c1 == self.p0 && self.c2 == self.p3
    }

    /// Appends points approximating the curve to `out` (excluding `p0`),
    /// so that the maximum deviation stays below `tolerance`.
    pub fn flatten(&self, tolerance: f32, out: &mut Vec<Vec2>) {
        // The error of n uniform steps is bounded by 6*dd / (8*n^2).
        let dd = (self.p0 - 2.0 * self.c1 + self.c2).length().max((self.c1 - 2.0 * self.c2 + self.p3).length());
        let n = ((0.75 * dd / tolerance.max(1e-6)).sqrt().ceil() as usize).clamp(1, 256);
        for i in 1..=n {
            out.push(self.point(i as f32 / n as f32));
        }
    }

    /// Closest point on the curve to `p`: returns `(t, distance)`.
    pub fn nearest(&self, p: Vec2) -> (f32, f32) {
        const STEPS: usize = 32;
        let dist = |t: f32| self.point(t).distance(p);
        let (mut best_t, mut best_d) = (0.0, f32::MAX);
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let d = dist(t);
            if d < best_d {
                (best_t, best_d) = (t, d);
            }
        }
        // Refine with a ternary search inside the winning sample interval.
        let step = 1.0 / STEPS as f32;
        let (mut lo, mut hi) = ((best_t - step).max(0.0), (best_t + step).min(1.0));
        for _ in 0..12 {
            let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if dist(a) < dist(b) {
                hi = b;
            } else {
                lo = a;
            }
        }
        let t = (lo + hi) * 0.5;
        (t, dist(t))
    }
}

/// A connected chain of anchors, open or closed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubPath {
    pub anchors: Vec<Anchor>,
    pub closed: bool,
}

impl SubPath {
    pub fn segment_count(&self) -> usize {
        let n = self.anchors.len();
        if self.closed {
            n
        } else {
            n.saturating_sub(1)
        }
    }

    /// The curve between anchor `i` and the next one.
    pub fn segment(&self, i: usize) -> Cubic {
        let a = &self.anchors[i];
        let b = &self.anchors[(i + 1) % self.anchors.len()];
        Cubic { p0: a.pos, c1: a.pos + a.handle_out, c2: b.pos + b.handle_in, p3: b.pos }
    }

    /// Splits segment `i` at `t`, inserting a new anchor that keeps the shape unchanged.
    pub fn split_segment(&mut self, i: usize, t: f32) {
        let segment = self.segment(i);
        if segment.is_line() {
            // A straight edge stays straight: the new anchor is a plain corner without handles.
            self.anchors.insert(i + 1, Anchor::corner(segment.p0.lerp(segment.p3, t)));
            return;
        }
        let (left, right) = segment.split(t);
        let next = (i + 1) % self.anchors.len();
        self.anchors[i].handle_out = left.c1 - left.p0;
        self.anchors[next].handle_in = right.c2 - right.p3;
        let anchor = Anchor { pos: left.p3, handle_in: left.c2 - left.p3, handle_out: right.c1 - right.p0 };
        self.anchors.insert(i + 1, anchor);
    }

    /// Cuts the subpath at anchor `index`. A closed subpath becomes one open subpath that starts and
    /// ends at the cut; an open one becomes two. Returns `None` if there is nothing to cut
    /// (the anchor is an end point of an open subpath).
    pub fn split_at_anchor(&self, index: usize) -> Option<Vec<SubPath>> {
        let n = self.anchors.len();
        if index >= n {
            return None;
        }
        let mut start = self.anchors[index];
        start.handle_in = Vec2::ZERO;
        let mut end = self.anchors[index];
        end.handle_out = Vec2::ZERO;

        if self.closed {
            let mut anchors = vec![start];
            anchors.extend((1..n).map(|k| self.anchors[(index + k) % n]));
            anchors.push(end);
            return Some(vec![SubPath { anchors, closed: false }]);
        }
        if index == 0 || index + 1 == n {
            return None;
        }
        let mut first: Vec<Anchor> = self.anchors[..index].to_vec();
        first.push(end);
        let mut second = vec![start];
        second.extend_from_slice(&self.anchors[index + 1..]);
        Some(vec![SubPath { anchors: first, closed: false }, SubPath { anchors: second, closed: false }])
    }

    pub fn flatten(&self, tolerance: f32) -> Polyline {
        let mut points: Vec<Vec2> = self.anchors.first().map(|a| a.pos).into_iter().collect();
        for i in 0..self.segment_count() {
            let seg = self.segment(i);
            if seg.is_line() {
                points.push(seg.p3);
            } else {
                seg.flatten(tolerance, &mut points);
            }
        }
        Polyline { points, closed: self.closed }
    }
}

/// A flattened subpath. When closed, the last point equals the first.
#[derive(Clone, Debug)]
pub struct Polyline {
    pub points: Vec<Vec2>,
    pub closed: bool,
}

impl Polyline {
    /// Edges of the polyline, including the implicit edge back to the start.
    pub fn edges(&self) -> impl Iterator<Item = (Vec2, Vec2)> + '_ {
        let n = self.points.len();
        (0..n).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }
}

/// A compound path: one or more subpaths filled together.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Path {
    pub subpaths: Vec<SubPath>,
}

impl Path {
    pub fn rect(min: Vec2, max: Vec2) -> Self {
        let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        Self {
            subpaths: vec![SubPath { anchors: corners.iter().map(|&c| Anchor::corner(c)).collect(), closed: true }],
        }
    }

    pub fn ellipse(center: Vec2, radii: Vec2) -> Self {
        // Control-point distance that makes four cubic arcs best approximate a circle.
        const KAPPA: f32 = 0.552_284_7;
        let (kx, ky) = (radii.x * KAPPA, radii.y * KAPPA);
        let anchor = |offset: Vec2, handle_out: Vec2| Anchor::smooth(center + offset, handle_out);
        Self {
            subpaths: vec![SubPath {
                anchors: vec![
                    anchor(Vec2::new(radii.x, 0.0), Vec2::new(0.0, ky)),
                    anchor(Vec2::new(0.0, radii.y), Vec2::new(-kx, 0.0)),
                    anchor(Vec2::new(-radii.x, 0.0), Vec2::new(0.0, -ky)),
                    anchor(Vec2::new(0.0, -radii.y), Vec2::new(kx, 0.0)),
                ],
                closed: true,
            }],
        }
    }

    pub fn transformed(&self, m: &Affine2) -> Path {
        let mut path = self.clone();
        for anchor in path.subpaths.iter_mut().flat_map(|s| s.anchors.iter_mut()) {
            anchor.pos = m.transform_point2(anchor.pos);
            anchor.handle_in = m.transform_vector2(anchor.handle_in);
            anchor.handle_out = m.transform_vector2(anchor.handle_out);
        }
        path
    }

    pub fn flatten(&self, tolerance: f32) -> Vec<Polyline> {
        self.subpaths.iter().map(|s| s.flatten(tolerance)).collect()
    }

    pub fn bounds(&self) -> Option<Rect> {
        let polylines = self.flatten(0.05);
        Rect::from_cloud(polylines.iter().flat_map(|p| p.points.iter().copied()))
    }

    /// The bounds of this path moved by `m`, the same as `self.transformed(m).bounds()`. A path of straight lines only (the usual one on
    /// a map: no handles) has the box of its moved anchors, found with no copy of the path and no flattening; with the selection of a
    /// whole map this is the difference between 4 ms and 1 ms.
    pub fn bounds_under(&self, m: &Affine2) -> Option<Rect> {
        let straight = self.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.handle_in == Vec2::ZERO && a.handle_out == Vec2::ZERO);
        if !straight {
            return self.transformed(m).bounds();
        }
        Rect::from_cloud(self.subpaths.iter().flat_map(|s| &s.anchors).map(|a| m.transform_point2(a.pos)))
    }

    /// The box of the anchors and their handles. A curve lies inside the hull of its control points, so the whole path is inside
    /// this box, which costs a pass over the points and no flattening. `None` for a path with no points, or with a point that is not a
    /// number (callers then take the exact route).
    pub fn control_bounds(&self) -> Option<Rect> {
        let mut found: Option<Rect> = None;
        for anchor in self.subpaths.iter().flat_map(|s| s.anchors.iter()) {
            for p in [anchor.pos, anchor.pos + anchor.handle_in, anchor.pos + anchor.handle_out] {
                if !p.is_finite() {
                    return None;
                }
                found = Some(match found {
                    Some(r) => Rect { min: r.min.min(p), max: r.max.max(p) },
                    None => Rect { min: p, max: p },
                });
            }
        }
        found
    }

    /// Point-in-shape test using the given fill rule. Open subpaths count as closed.
    pub fn contains(&self, p: Vec2, rule: FillRule, tolerance: f32) -> bool {
        let (mut winding, mut crossings) = (0i32, 0u32);
        for poly in self.flatten(tolerance) {
            for (a, b) in poly.edges() {
                let side = (b - a).perp_dot(p - a);
                if a.y <= p.y && b.y > p.y && side > 0.0 {
                    winding += 1;
                    crossings += 1;
                } else if a.y > p.y && b.y <= p.y && side < 0.0 {
                    winding -= 1;
                    crossings += 1;
                }
            }
        }
        match rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => crossings % 2 == 1,
        }
    }

    /// Distance from `p` to the path outline.
    pub fn distance_to_outline(&self, p: Vec2, tolerance: f32) -> f32 {
        self.flatten(tolerance)
            .iter()
            .flat_map(|poly| {
                let edges = if poly.closed { poly.points.len() } else { poly.points.len().saturating_sub(1) };
                poly.edges().take(edges).collect::<Vec<_>>()
            })
            .map(|(a, b)| distance_to_segment(p, a, b))
            .fold(f32::MAX, f32::min)
    }

    /// Closest segment to `p`: returns `(subpath index, segment index, t, distance)`.
    pub fn nearest_segment(&self, p: Vec2) -> Option<(usize, usize, f32, f32)> {
        let mut best: Option<(usize, usize, f32, f32)> = None;
        for (si, sub) in self.subpaths.iter().enumerate() {
            for i in 0..sub.segment_count() {
                let (t, d) = sub.segment(i).nearest(p);
                if best.is_none_or(|b| d < b.3) {
                    best = Some((si, i, t, d));
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipse_contains_center_but_not_outside() {
        let path = Path::ellipse(Vec2::ZERO, Vec2::new(10.0, 5.0));
        assert!(path.contains(Vec2::ZERO, FillRule::NonZero, 0.1));
        assert!(!path.contains(Vec2::new(11.0, 0.0), FillRule::NonZero, 0.1));
    }

    #[test]
    fn even_odd_makes_a_hole() {
        let mut path = Path::rect(Vec2::splat(-10.0), Vec2::splat(10.0));
        path.subpaths.extend(Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)).subpaths);
        assert!(!path.contains(Vec2::ZERO, FillRule::EvenOdd, 0.1));
        assert!(path.contains(Vec2::new(7.0, 0.0), FillRule::EvenOdd, 0.1));
    }

    #[test]
    fn splitting_a_straight_edge_adds_a_handle_free_corner() {
        let mut path = Path::rect(Vec2::ZERO, Vec2::splat(10.0));
        path.subpaths[0].split_segment(0, 0.5);
        let anchors = &path.subpaths[0].anchors;
        assert_eq!(anchors.len(), 5);
        assert_eq!(anchors[1].pos, Vec2::new(5.0, 0.0));
        assert!(anchors.iter().all(|a| a.handle_in == Vec2::ZERO && a.handle_out == Vec2::ZERO), "every edge is still a plain line");
    }

    #[test]
    fn cutting_a_closed_path_opens_it_at_the_anchor() {
        let path = Path::rect(Vec2::ZERO, Vec2::splat(10.0));
        let parts = path.subpaths[0].split_at_anchor(1).unwrap();
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].closed);
        assert_eq!(parts[0].anchors.len(), 5);
        assert_eq!(parts[0].anchors[0].pos, parts[0].anchors[4].pos);
    }

    #[test]
    fn cutting_an_open_path_makes_two_and_ignores_end_points() {
        let line = SubPath { anchors: (0..4).map(|i| Anchor::corner(Vec2::new(i as f32, 0.0))).collect(), closed: false };
        let parts = line.split_at_anchor(2).unwrap();
        assert_eq!((parts[0].anchors.len(), parts[1].anchors.len()), (3, 2));
        assert!(line.split_at_anchor(0).is_none());
        assert!(line.split_at_anchor(3).is_none());
    }

    #[test]
    fn the_bounds_of_a_moved_path_are_those_of_the_moved_path() {
        let polyline = Path { subpaths: vec![SubPath { anchors: [(3.0, 1.0), (40.0, -7.0), (12.0, 30.0), (-5.0, 9.0)].map(|(x, y)| Anchor::corner(Vec2::new(x, y))).to_vec(), closed: false }] };
        let mut two_parts = Path::rect(Vec2::new(-4.0, -2.0), Vec2::new(10.0, 6.0));
        two_parts.subpaths.extend(Path::rect(Vec2::new(100.0, 50.0), Vec2::new(130.0, 80.0)).subpaths);
        let lone_point = Path { subpaths: vec![SubPath { anchors: vec![Anchor::corner(Vec2::new(7.0, -3.0))], closed: false }] };
        let one_curve = Path::ellipse(Vec2::new(5.0, 5.0), Vec2::new(10.0, 4.0));
        // A handle on only one side of an edge still bends it: out of the first corner, and into the second.
        let mut bent_out = Path::rect(Vec2::ZERO, Vec2::splat(10.0));
        bent_out.subpaths[0].anchors[1].handle_out = Vec2::new(8.0, 0.0);
        let mut bent_in = Path::rect(Vec2::ZERO, Vec2::splat(10.0));
        bent_in.subpaths[0].anchors[2].handle_in = Vec2::new(8.0, 0.0);
        let moves = [
            Affine2::IDENTITY,
            Affine2::from_translation(Vec2::new(30.0, -20.0)),
            Affine2::from_angle(0.7) * Affine2::from_scale(Vec2::new(2.0, 0.5)),
            Affine2::from_translation(Vec2::new(-8.0, 2.0)) * Affine2::from_angle(-2.1) * Affine2::from_scale(Vec2::splat(1.5)),
        ];
        for (name, path) in [("polyline", &polyline), ("two parts", &two_parts), ("lone point", &lone_point), ("ellipse", &one_curve), ("a handle out", &bent_out), ("a handle in", &bent_in), ("nothing", &Path::default())] {
            for m in &moves {
                assert_eq!(path.bounds_under(m), path.transformed(m).bounds(), "{name} under {m:?}");
            }
        }
        assert!(Path::default().bounds_under(&Affine2::IDENTITY).is_none());
    }

    #[test]
    fn splitting_keeps_the_shape() {
        let mut path = Path::ellipse(Vec2::ZERO, Vec2::splat(10.0));
        let before = path.subpaths[0].segment(0).point(0.5);
        path.subpaths[0].split_segment(0, 0.5);
        assert_eq!(path.subpaths[0].anchors.len(), 5);
        assert!(path.subpaths[0].anchors[1].pos.distance(before) < 1e-4);
    }
}
