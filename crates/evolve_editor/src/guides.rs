//! Smart guides: snapping a moving box to the edges and centers of other objects.

use evolve_engine::geometry::Rect;

/// An alignment line shown while dragging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    /// A vertical line sits at x = `at`, a horizontal one at y = `at`.
    pub vertical: bool,
    pub at: f32,
}

/// The closest alignment on one axis: how far to shift, and where the guide goes.
fn best_alignment(mine: [f32; 3], theirs: impl Iterator<Item = [f32; 3]>, threshold: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for candidate in theirs.flat_map(|t| t.into_iter()) {
        for m in mine {
            let offset = candidate - m;
            if offset.abs() <= threshold && best.is_none_or(|(o, _)| offset.abs() < o.abs()) {
                best = Some((offset, candidate));
            }
        }
    }
    best
}

fn x_lines(r: &Rect) -> [f32; 3] {
    [r.min.x, r.center().x, r.max.x]
}

fn y_lines(r: &Rect) -> [f32; 3] {
    [r.min.y, r.center().y, r.max.y]
}

/// Finds the shift (per axis, if any) that aligns `moved` with one of `targets` within `threshold`,
/// plus the guides to draw.
pub fn snap_to_guides(moved: &Rect, targets: &[Rect], threshold: f32) -> (Option<f32>, Option<f32>, Vec<Guide>) {
    let x = best_alignment(x_lines(moved), targets.iter().map(x_lines), threshold);
    let y = best_alignment(y_lines(moved), targets.iter().map(y_lines), threshold);
    let guides = x
        .map(|(_, at)| Guide { vertical: true, at })
        .into_iter()
        .chain(y.map(|(_, at)| Guide { vertical: false, at }))
        .collect();
    (x.map(|(o, _)| o), y.map(|(o, _)| o), guides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::geometry::Vec2;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { min: Vec2::new(x, y), max: Vec2::new(x + w, y + h) }
    }

    #[test]
    fn edges_snap_when_close() {
        let target = rect(100.0, 0.0, 50.0, 50.0);
        let moved = rect(53.0, 200.0, 50.0, 50.0); // right edge at 103, 3 away from the target's left edge
        let (x, y, guides) = snap_to_guides(&moved, &[target], 5.0);
        assert_eq!(x, Some(-3.0));
        assert_eq!(y, None);
        assert_eq!(guides, vec![Guide { vertical: true, at: 100.0 }]);
    }

    #[test]
    fn centers_align_and_far_objects_are_ignored() {
        let target = rect(0.0, 0.0, 100.0, 100.0);
        let moved = rect(20.0, 201.0, 60.0, 60.0); // center x = 50 matches; center y = 231
        let (x, y, _) = snap_to_guides(&moved, &[target], 5.0);
        assert_eq!(x, Some(0.0));
        assert_eq!(y, None);
    }
}
