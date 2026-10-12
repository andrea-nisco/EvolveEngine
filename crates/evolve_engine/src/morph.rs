//! Shape morphing: blending one path into another, even with different numbers of anchors.
//!
//! The two paths are made compatible first: the one with fewer anchors gets extra anchors by
//! splitting its longest segments (which does not change its shape), and closed outlines are
//! rotated or reversed so that the anchors that are closest to each other get paired.

use crate::geometry::Vec2;
use crate::path::{Anchor, Path, SubPath};

/// Blends `a` into `b`: `t = 0` looks like `a`, `t = 1` looks like `b`.
/// Subpaths that exist on one side only grow out of (or shrink into) their own center.
pub fn morph(a: &Path, b: &Path, t: f32) -> Path {
    let count = a.subpaths.len().max(b.subpaths.len());
    let subpaths = (0..count)
        .filter_map(|i| match (a.subpaths.get(i), b.subpaths.get(i)) {
            (Some(x), Some(y)) => Some(morph_subpath(x, y, t)),
            (Some(x), None) => Some(morph_subpath(x, &collapsed(x), t)),
            (None, Some(y)) => Some(morph_subpath(&collapsed(y), y, t)),
            (None, None) => None,
        })
        .collect();
    Path { subpaths }
}

/// The same anchors, all squeezed into the subpath's center.
fn collapsed(sub: &SubPath) -> SubPath {
    let center = if sub.anchors.is_empty() {
        Vec2::ZERO
    } else {
        sub.anchors.iter().map(|a| a.pos).sum::<Vec2>() / sub.anchors.len() as f32
    };
    SubPath { anchors: sub.anchors.iter().map(|_| Anchor::corner(center)).collect(), closed: sub.closed }
}

fn morph_subpath(a: &SubPath, b: &SubPath, t: f32) -> SubPath {
    let closed = if t < 0.5 { a.closed } else { b.closed };
    let count = a.anchors.len().max(b.anchors.len());
    let a = with_anchor_count(a, count);
    let mut b = with_anchor_count(b, count);
    if a.closed && b.closed {
        b = best_alignment(&a, &b);
    }
    let anchors = a
        .anchors
        .iter()
        .zip(&b.anchors)
        .map(|(x, y)| Anchor {
            pos: x.pos.lerp(y.pos, t),
            handle_in: x.handle_in.lerp(y.handle_in, t),
            handle_out: x.handle_out.lerp(y.handle_out, t),
        })
        .collect();
    SubPath { anchors, closed }
}

/// Adds anchors (without changing the shape) until there are `count`.
fn with_anchor_count(sub: &SubPath, count: usize) -> SubPath {
    let mut sub = sub.clone();
    if sub.anchors.is_empty() {
        return sub;
    }
    while sub.anchors.len() < count {
        if sub.anchors.len() < 2 {
            sub.anchors.push(sub.anchors[0]);
            continue;
        }
        let longest = (0..sub.segment_count())
            .max_by(|&i, &j| segment_length(&sub, i).total_cmp(&segment_length(&sub, j)))
            .unwrap_or(0);
        sub.split_segment(longest, 0.5);
    }
    sub
}

/// Approximate length of a segment: the length of its control polygon.
fn segment_length(sub: &SubPath, index: usize) -> f32 {
    let s = sub.segment(index);
    s.p0.distance(s.c1) + s.c1.distance(s.c2) + s.c2.distance(s.p3)
}

fn reversed(sub: &SubPath) -> SubPath {
    let anchors = sub
        .anchors
        .iter()
        .rev()
        .map(|a| Anchor { pos: a.pos, handle_in: a.handle_out, handle_out: a.handle_in })
        .collect();
    SubPath { anchors, closed: sub.closed }
}

/// Rotates (and maybe reverses) the anchors of closed `b` so they sit closest to those of `a`.
fn best_alignment(a: &SubPath, b: &SubPath) -> SubPath {
    let n = a.anchors.len();
    let cost = |candidate: &SubPath, shift: usize| -> f32 {
        (0..n).map(|i| a.anchors[i].pos.distance_squared(candidate.anchors[(i + shift) % n].pos)).sum()
    };
    let flipped = reversed(b);
    let mut best = (f32::MAX, b, 0usize);
    for candidate in [b, &flipped] {
        for shift in 0..n {
            let c = cost(candidate, shift);
            if c < best.0 {
                best = (c, candidate, shift);
            }
        }
    }
    let (_, source, shift) = best;
    SubPath { anchors: (0..n).map(|i| source.anchors[(i + shift) % n]).collect(), closed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, size: f32) -> Path {
        Path::rect(Vec2::new(x, y), Vec2::new(x + size, y + size))
    }

    #[test]
    fn end_points_match_the_inputs() {
        let (a, b) = (square(0.0, 0.0, 10.0), square(100.0, 0.0, 20.0));
        assert_eq!(morph(&a, &b, 0.0).bounds(), a.bounds());
        let end = morph(&a, &b, 1.0).bounds().unwrap();
        let expected = b.bounds().unwrap();
        assert!(end.min.distance(expected.min) < 1e-3 && end.max.distance(expected.max) < 1e-3);
    }

    #[test]
    fn halfway_is_in_between() {
        let (a, b) = (square(0.0, 0.0, 10.0), square(100.0, 0.0, 10.0));
        let mid = morph(&a, &b, 0.5).bounds().unwrap();
        assert!((mid.min.x - 50.0).abs() < 1e-3);
    }

    #[test]
    fn different_anchor_counts_are_matched_without_changing_the_start_shape() {
        let a = square(0.0, 0.0, 10.0); // 4 anchors
        let b = Path::ellipse(Vec2::new(5.0, 5.0), Vec2::splat(5.0));
        let more = Path { subpaths: vec![with_anchor_count(&b.subpaths[0], 7)] };
        assert_eq!(more.subpaths[0].anchors.len(), 7);
        assert_eq!(more.bounds().map(|r| r.size()), b.bounds().map(|r| r.size()));
        assert_eq!(morph(&a, &b, 0.5).subpaths[0].anchors.len(), 4);
        assert_eq!(morph(&a, &more, 0.5).subpaths[0].anchors.len(), 7);
    }

    #[test]
    fn rotated_start_points_do_not_twist_the_shape() {
        let a = square(0.0, 0.0, 10.0);
        let mut b = a.clone();
        b.subpaths[0].anchors.rotate_left(2); // same square, different starting corner
        let mid = morph(&a, &b, 0.5).bounds().unwrap();
        assert!(mid.size().x > 9.9 && mid.size().y > 9.9, "the square collapsed: {mid:?}");
    }

    #[test]
    fn extra_subpaths_grow_from_their_center() {
        let a = square(0.0, 0.0, 10.0);
        let mut b = square(0.0, 0.0, 10.0);
        b.subpaths.extend(square(50.0, 50.0, 10.0).subpaths);
        assert_eq!(morph(&a, &b, 0.5).subpaths.len(), 2);
        let tiny = morph(&a, &b, 0.0);
        assert!(tiny.subpaths[1].anchors.iter().all(|p| p.pos == tiny.subpaths[1].anchors[0].pos));
    }
}
