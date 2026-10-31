//! The text the UI asks for and what comes back. The data lives here, in the neutral crate, so that the tree can measure and draw
//! text; the shaping itself (fonts, fallback, bidirectional text) is a [`Typesetter`], made by `evolve_ui_text`.

use crate::draw::TextRun;
use crate::geometry::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Family name; `None` is the default sans (Inter). Characters the family lacks come from the fallback fonts.
    pub family: Option<String>,
    /// Em size in logical pixels.
    pub size: f32,
    /// 400 is regular, 700 bold.
    pub weight: u16,
    pub color: Color,
    /// Distance between baselines, as a multiple of the size.
    pub line_height: f32,
    pub align: TextAlign,
    /// The width the text may take; with `wrap` it breaks into lines, without it the end is cut (see `ellipsis`).
    pub max_width: Option<f32>,
    /// Break lines at word boundaries (only with `max_width`).
    pub wrap: bool,
    /// Lines to keep; the last one ends with an ellipsis if text was left out. `None`: all of them (a single line with `wrap` off).
    pub max_lines: Option<usize>,
    /// Put an ellipsis where text does not fit (only with `max_width`).
    pub ellipsis: bool,
    /// Pixel snapping: letters advance by whole physical pixels and the baseline sits on a pixel row, so that the stems of small
    /// text fall on the pixel grid. Makes the layout depend on the scale. The origin of the text should itself be on a whole
    /// physical pixel for this to show.
    pub snap: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: None,
            size: 16.0,
            weight: 400,
            color: Color::BLACK,
            line_height: 1.3,
            align: TextAlign::Left,
            max_width: None,
            wrap: false,
            max_lines: None,
            ellipsis: false,
            snap: true,
        }
    }
}

/// A shaped text: runs with glyph positions relative to the text's top-left corner, and its size.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub runs: Vec<TextRun>,
    /// The widest line.
    pub width: f32,
    /// Lines times the line height.
    pub height: f32,
    pub lines: usize,
}

/// Whatever shapes text. `scale` is the number of physical pixels per logical pixel; the result is in logical pixels.
pub trait Typesetter {
    fn layout(&mut self, text: &str, style: &TextStyle, scale: f32) -> TextLayout;
}
