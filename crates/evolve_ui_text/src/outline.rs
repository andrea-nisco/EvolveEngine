//! Glyph outlines as UI paths, from swash.

use evolve_ui_core::{Point, UiPath};
use swash::scale::{ScaleContext, Scaler};
use swash::zeno::Verb;
use swash::{FontRef, GlyphId};

/// The size glyphs are scaled to before their coordinates are divided back into em units. Large, so that the integer
/// rounding of the scaler is far below anything visible.
const SCALE: f32 = 1000.0;

/// The outline of `glyph` in `font` in em units (1.0 is the font size), y growing downwards, the origin on the baseline at
/// the glyph's pen position. `None` for a glyph with no outline (a space).
pub(crate) fn glyph_path(context: &mut ScaleContext, font: FontRef<'_>, glyph: GlyphId) -> Option<UiPath> {
    let mut scaler: Scaler<'_> = context.builder(font).size(SCALE).hint(false).build();
    let outline = scaler.scale_outline(glyph)?;
    let (verbs, points) = (outline.verbs(), outline.points());
    if verbs.is_empty() {
        return None;
    }
    let mut taken = 0usize;
    let mut next = |count: usize| -> Option<Vec<Point>> {
        let slice = points.get(taken..taken + count)?;
        taken += count;
        Some(slice.iter().map(|p| Point::new(p.x / SCALE, -p.y / SCALE)).collect())
    };
    let mut path = UiPath::default();
    for verb in verbs {
        match verb {
            Verb::MoveTo => path.move_to(next(1)?[0]),
            Verb::LineTo => path.line_to(next(1)?[0]),
            Verb::QuadTo => {
                let p = next(2)?;
                path.quad_to(p[0], p[1]);
            }
            Verb::CurveTo => {
                let p = next(3)?;
                path.cubic_to(p[0], p[1], p[2]);
            }
            Verb::Close => path.close(),
        }
    }
    (!path.is_empty()).then_some(path)
}
