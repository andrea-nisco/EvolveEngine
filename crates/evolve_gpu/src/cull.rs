//! Which shapes are worth making and drawing for a view (plan 5.7, pulled forward after the first hands-on try).
//!
//! A shape whose pixels all lie outside the view, with a margin, is neither tessellated nor drawn: zoomed in far on a map of
//! 21,000 shapes only the few hundred near the screen are made, instead of all of them at the fine zoom bucket (which went
//! over the geometry budget and sent the view back to the CPU renderer). The test is deliberately generous, so that it never
//! hides a shape that touches the view:
//! - the shape's extent is the box of its **control points** (a Bézier curve lies inside the hull of its control points),
//! - a stroke grows the box by half its width times the largest a miter, a square cap or a round join can reach, and
//! - strokes and fills get a couple of pixels for anti-aliasing and the one-pixel minimum of thin lines.
//!
//! The picture with culling is the picture without it (a test draws both and compares the bytes). Anything the test cannot
//! judge (a path without points, a transform that is not a finite number) is kept.

use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::path::Path;
use evolve_engine::style::{LineJoin, Stroke};

/// A box in the shape's own space: `(min, max)`.
pub type Extent = (Vec2, Vec2);

/// Pixels added around every shape for anti-aliasing and the minimum line width.
pub const SLACK_PX: f32 = 3.0;
/// The most a stroke's reach is multiplied by: miter limits above this are treated as this.
const MAX_REACH_FACTOR: f32 = 50.0;

/// The part of the world a view shows, and a margin around it.
#[derive(Clone, Copy, Debug)]
pub struct Cull {
    /// World to pixels.
    view: Affine2,
    min: Vec2,
    max: Vec2,
    /// The size of the view in pixels.
    size: (f32, f32),
}

impl Cull {
    /// For a view of `size` pixels. The margin (a quarter of the larger side, and a little more) means a slow pan makes new
    /// shapes only now and then, not on every frame.
    pub fn new(view: Affine2, size: (u32, u32)) -> Cull {
        let (width, height) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let margin = 0.25 * width.max(height) + 64.0;
        Cull { view, min: Vec2::splat(-margin), max: Vec2::new(width + margin, height + margin), size: (width, height) }
    }

    /// Whether a shape can reach the view itself, not only the margin around it (what is seen now, against what is only kept ready
    /// for a move).
    pub fn touches_view(&self, extent: Option<Extent>, world: &Affine2, stroke: Option<&Stroke>) -> bool {
        let Some(extent) = extent else { return true };
        let Some((min, max)) = pixel_box(extent, &(self.view * *world), stroke, SLACK_PX) else { return true };
        max.x >= 0.0 && min.x <= self.size.0 && max.y >= 0.0 && min.y <= self.size.1
    }

    /// The part of the pixel plane that this cull keeps shapes for: the view and its margin, as (min, max).
    pub fn region(&self) -> (Vec2, Vec2) {
        (self.min, self.max)
    }

    /// Whether a shape with this extent (in its own space), drawn with `world` and this stroke, can reach the view.
    pub fn may_touch(&self, extent: Option<Extent>, world: &Affine2, stroke: Option<&Stroke>) -> bool {
        let Some(extent) = extent else { return true };
        let Some((min, max)) = pixel_box(extent, &(self.view * *world), stroke, SLACK_PX) else { return true };
        max.x >= self.min.x && min.x <= self.max.x && max.y >= self.min.y && min.y <= self.max.y
    }
}

/// The box in pixels that a shape with this extent (its own space) can paint, drawn with `to_pixels` and this stroke: the
/// extent's corners moved, grown by the stroke's reach and by `slack` pixels. `None` when it cannot be judged (a transform or
/// a width that is not a number): callers keep such a shape.
pub fn pixel_box(extent: Extent, to_pixels: &Affine2, stroke: Option<&Stroke>, slack: f32) -> Option<(Vec2, Vec2)> {
    let (lo, hi) = extent;
    let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)].map(|c| to_pixels.transform_point2(c));
    if corners.iter().any(|c| !c.is_finite()) {
        return None;
    }
    let (mut min, mut max) = (corners[0], corners[0]);
    for c in &corners[1..] {
        min = min.min(*c);
        max = max.max(*c);
    }
    let scale = to_pixels.matrix2.x_axis.length().max(to_pixels.matrix2.y_axis.length());
    let reach = stroke.filter(|s| s.width > 0.0 && s.width.is_finite()).map_or(0.0, |s| {
        let factor = match s.join {
            LineJoin::Miter => s.miter_limit.clamp(1.0, MAX_REACH_FACTOR),
            LineJoin::Round | LineJoin::Bevel => 1.0,
        };
        0.5 * s.width * scale * factor.max(std::f32::consts::SQRT_2)
    });
    if !reach.is_finite() {
        return None;
    }
    let reach = reach + slack;
    Some((min - Vec2::splat(reach), max + Vec2::splat(reach)))
}

/// The box of a path's anchors and handles, in the path's own space.
pub fn control_extent(path: &Path) -> Option<Extent> {
    let mut found: Option<Extent> = None;
    for sub in &path.subpaths {
        for anchor in &sub.anchors {
            for p in [anchor.pos, anchor.pos + anchor.handle_in, anchor.pos + anchor.handle_out] {
                found = Some(match found {
                    Some((lo, hi)) => (lo.min(p), hi.max(p)),
                    None => (p, p),
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::path::{Anchor, SubPath};

    fn square(at: Vec2, side: f32) -> Option<Extent> {
        control_extent(&Path::rect(at, at + Vec2::splat(side)))
    }

    fn view(zoom: f32) -> Cull {
        Cull::new(Affine2::from_scale(Vec2::splat(zoom)), (800, 600))
    }

    #[test]
    fn a_shape_in_the_view_is_kept_and_one_far_away_is_not() {
        let cull = view(1.0);
        assert!(cull.may_touch(square(Vec2::new(100.0, 100.0), 50.0), &Affine2::IDENTITY, None));
        assert!(!cull.may_touch(square(Vec2::new(5000.0, 100.0), 50.0), &Affine2::IDENTITY, None));
        assert!(!cull.may_touch(square(Vec2::new(-5000.0, -5000.0), 50.0), &Affine2::IDENTITY, None));
        assert!(!cull.may_touch(square(Vec2::new(100.0, 4000.0), 50.0), &Affine2::IDENTITY, None));
    }

    #[test]
    fn the_margin_keeps_shapes_just_outside_and_the_zoom_decides_what_is_far() {
        // 800 x 600 view: margin = 0.25 * 800 + 64 = 264 pixels.
        let just_outside = square(Vec2::new(800.0 + 200.0, 100.0), 10.0);
        assert!(view(1.0).may_touch(just_outside, &Affine2::IDENTITY, None));
        assert!(!view(1.0).may_touch(square(Vec2::new(800.0 + 300.0, 100.0), 10.0), &Affine2::IDENTITY, None));
        // The same place at 4x: 1000 units are 4000 pixels away.
        assert!(!view(4.0).may_touch(just_outside, &Affine2::IDENTITY, None));
        // A shape larger than the view that covers it entirely is in.
        assert!(view(1.0).may_touch(square(Vec2::new(-5000.0, -5000.0), 20_000.0), &Affine2::IDENTITY, None));
    }

    #[test]
    fn a_wide_stroke_reaches_further_than_the_shape() {
        let far = square(Vec2::new(800.0 + 400.0, 100.0), 10.0);
        let mut stroke = Stroke::solid(evolve_engine::style::Color::rgb(0, 0, 0), 4.0);
        assert!(!view(1.0).may_touch(far, &Affine2::IDENTITY, Some(&stroke)));
        stroke.width = 400.0;
        assert!(view(1.0).may_touch(far, &Affine2::IDENTITY, Some(&stroke)), "half of 400 reaches 200 and a miter more: 400 + 264 > 400");
        stroke.join = LineJoin::Round;
        stroke.width = 160.0; // reach 80 + slack: not enough to cover the 136 pixels between the margin and the shape
        assert!(!view(1.0).may_touch(far, &Affine2::IDENTITY, Some(&stroke)));
        stroke.join = LineJoin::Miter;
        stroke.miter_limit = 4.0;
        assert!(view(1.0).may_touch(far, &Affine2::IDENTITY, Some(&stroke)), "the same width with a miter join can reach 4 times as far");
    }

    #[test]
    fn the_shapes_own_transform_and_a_rotated_view_are_taken_into_account() {
        let extent = square(Vec2::ZERO, 10.0);
        assert!(view(1.0).may_touch(extent, &Affine2::IDENTITY, None), "at the origin: in");
        let moved = Affine2::from_translation(Vec2::new(9000.0, 0.0));
        assert!(!view(1.0).may_touch(extent, &moved, None));
        let scaled_up = Affine2::from_scale(Vec2::splat(2000.0));
        assert!(view(1.0).may_touch(extent, &scaled_up, None), "a huge scale makes it cover the view");
        let rotated = Cull::new(Affine2::from_angle(std::f32::consts::FRAC_PI_2) * Affine2::from_translation(Vec2::new(0.0, -1000.0)), (800, 600));
        // A quarter turn: what was far to the right is now far down, and the box test is done on the pixel box.
        assert!(!rotated.may_touch(square(Vec2::new(5000.0, 0.0), 10.0), &Affine2::IDENTITY, None));
    }

    #[test]
    fn what_cannot_be_judged_is_kept() {
        let cull = view(1.0);
        assert!(cull.may_touch(None, &Affine2::IDENTITY, None), "no points");
        let nan = Affine2::from_translation(Vec2::new(f32::NAN, 0.0));
        assert!(cull.may_touch(square(Vec2::ZERO, 10.0), &nan, None));
        let mut stroke = Stroke::solid(evolve_engine::style::Color::rgb(0, 0, 0), f32::INFINITY);
        assert!(!cull.may_touch(square(Vec2::new(9000.0, 0.0), 10.0), &Affine2::IDENTITY, Some(&stroke)), "an infinite width is no stroke at all, as for the draw list");
        stroke.width = 0.0;
        assert!(!cull.may_touch(square(Vec2::new(9000.0, 0.0), 10.0), &Affine2::IDENTITY, Some(&stroke)), "no stroke when the width is 0");
    }

    #[test]
    fn the_extent_holds_the_handles_of_curves_too() {
        let path = Path {
            subpaths: vec![SubPath {
                anchors: vec![
                    Anchor { pos: Vec2::new(0.0, 0.0), handle_in: Vec2::ZERO, handle_out: Vec2::new(0.0, -50.0) },
                    Anchor { pos: Vec2::new(10.0, 0.0), handle_in: Vec2::new(0.0, -50.0), handle_out: Vec2::ZERO },
                ],
                closed: false,
            }],
        };
        let (lo, hi) = control_extent(&path).unwrap();
        assert_eq!((lo, hi), (Vec2::new(0.0, -50.0), Vec2::new(10.0, 0.0)));
        assert_eq!(control_extent(&Path::default()), None);
    }
}
