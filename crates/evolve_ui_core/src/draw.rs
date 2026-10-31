//! The draw list: the one thing the UI hands to a backend (ADR 0001).
//!
//! Commands are drawn in order, the last one on top. [`DrawCmd::PushClip`] and [`DrawCmd::PopClip`] bracket the commands
//! that are cut to a rectangle; brackets nest.

use crate::geometry::{Color, Point, Rect};
use crate::path::UiPath;

/// How a shape is filled.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    /// Colors along the line from `start` to `end` (in the draw list's own coordinates); before `start` and after `end` the
    /// first and the last color go on. Stops are `(position 0..=1, color)`; a backend uses at most [`MAX_GRADIENT_STOPS`].
    LinearGradient { start: Point, end: Point, stops: Vec<(f32, Color)> },
}

/// The most stops of a gradient that are used; more are ignored (a damaged file must not make unbounded work).
pub const MAX_GRADIENT_STOPS: usize = 32;

/// An image the backend knows (an SVG, for now) by this number. Ids come from the backend's asset store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u32);

/// A font as the text system numbers it. Only the system that made the id can say what is behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// One glyph of a run: the glyph's index in its font, and where its origin sits on the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPos {
    pub id: u16,
    /// Pen position, relative to the origin of the run's command.
    pub x: f32,
    /// Baseline, relative to the origin of the run's command (grows downwards).
    pub y: f32,
}

/// A stretch of text in one font, one size and one color, already shaped: glyphs in reading order, each with its position (in right-to-left text the first glyph is at the right end).
/// Holds no outlines and no pixels: a backend asks the text system for the outline of `(font, glyph)`, or fills an atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub font: FontId,
    /// Em size in logical pixels.
    pub size: f32,
    pub color: Color,
    pub glyphs: Vec<GlyphPos>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    /// A rectangle with a radius per corner (top-left, top-right, bottom-right, bottom-left) and an optional border.
    RoundedRect { rect: Rect, radii: [f32; 4], fill: Option<Fill>, border: Option<(f32, Color)> },
    /// A free path, drawn with its coordinates relative to `origin`.
    Path { origin: Point, path: UiPath, fill: Option<Fill> },
    /// A soft shadow of the rectangle `rect` (as if cast by it, moved by `offset`, grown by `spread`), fading out over `blur`
    /// pixels on each side of its edge. Drawn under what comes after it. A backend may approximate the blur.
    Shadow { rect: Rect, radii: [f32; 4], offset: Point, blur: f32, spread: f32, color: Color },
    /// An image fitted inside `rect` (kept in proportion, centered). With `tint` every color of it is replaced by that color,
    /// which makes one drawing of an icon serve for any color.
    Image { rect: Rect, image: ImageId, tint: Option<Color> },
    /// A shaped run; its glyph positions are relative to `origin`.
    Text { origin: Point, run: TextRun },
    /// Everything up to the matching [`DrawCmd::PopClip`] is cut to `rect` (with rounded corners if `radii` are not zero).
    PushClip { rect: Rect, radii: [f32; 4] },
    PopClip,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, cmd: DrawCmd) {
        self.cmds.push(cmd);
    }

    pub fn text(&mut self, origin: Point, run: TextRun) {
        self.push(DrawCmd::Text { origin, run });
    }

    pub fn shadow(&mut self, rect: Rect, radii: [f32; 4], offset: Point, blur: f32, color: Color) {
        self.push(DrawCmd::Shadow { rect, radii, offset, blur, spread: 0.0, color });
    }

    pub fn image(&mut self, rect: Rect, image: ImageId, tint: Option<Color>) {
        self.push(DrawCmd::Image { rect, image, tint });
    }

    pub fn rect(&mut self, rect: Rect, radii: [f32; 4], fill: Color) {
        self.push(DrawCmd::RoundedRect { rect, radii, fill: Some(Fill::Solid(fill)), border: None });
    }

    pub fn push_clip(&mut self, rect: Rect, radii: [f32; 4]) {
        self.push(DrawCmd::PushClip { rect, radii });
    }

    pub fn pop_clip(&mut self) {
        self.push(DrawCmd::PopClip);
    }

    /// Every `PushClip` has its `PopClip` and none comes before its push.
    pub fn is_balanced(&self) -> bool {
        let mut depth = 0usize;
        for cmd in &self.cmds {
            match cmd {
                DrawCmd::PushClip { .. } => depth += 1,
                DrawCmd::PopClip => match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => return false,
                },
                _ => {}
            }
        }
        depth == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_brackets_must_pair_up() {
        let mut list = DrawList::new();
        assert!(list.is_balanced());
        list.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
        assert!(!list.is_balanced(), "a clip left open");
        list.rect(Rect::new(1.0, 1.0, 4.0, 4.0), [2.0; 4], Color::WHITE);
        list.pop_clip();
        assert!(list.is_balanced());
        list.pop_clip();
        assert!(!list.is_balanced(), "a pop with nothing to pop");
    }

    #[test]
    fn a_pop_before_its_push_is_not_balanced_even_if_the_counts_match() {
        let mut list = DrawList::new();
        list.pop_clip();
        list.push_clip(Rect::default(), [0.0; 4]);
        assert!(!list.is_balanced());
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(Point::new(10.0, 20.0)));
        assert!(!r.contains(Point::new(40.0, 30.0)), "the right edge belongs to the next rectangle");
        assert!(!r.contains(Point::new(9.9, 30.0)));
    }
}
