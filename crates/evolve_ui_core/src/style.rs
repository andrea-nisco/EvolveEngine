//! The style of a node: how it is laid out (EVOLVE_UI_PLAN.md 4.3) and how it looks. The layout part is the one of flexbox and
//! grid with the words of the plan (`hug`, `fill`, `gap`, `align`, `justify`...); it is turned into Taffy's own style by the tree.

use crate::draw::Fill;
use crate::geometry::{Color, Point};

/// How a node is laid out among its children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Display {
    /// Children in a row or a column (`direction`).
    #[default]
    Flex,
    /// Children in the cells of the `columns` and `rows` tracks.
    Grid,
    /// The node and everything in it takes no space and is not drawn.
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

impl Direction {
    pub fn is_row(self) -> bool {
        matches!(self, Direction::Row | Direction::RowReverse)
    }
}

/// Where children (or one child, with `align_self`) sit across the direction of the container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    /// Grows to the whole width (in a column) or height (in a row) of the container.
    Stretch,
}

/// Where children sit along the direction of the container, when they do not fill it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// One side of a node's size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dim {
    /// As large as the content needs (Figma's *hug*).
    #[default]
    Hug,
    /// As large as the container leaves: along its direction a share of what remains (more with `grow`), across it the whole side.
    Fill,
    /// A number of logical pixels.
    Px(f32),
    /// A share of the container's size (0 to 100).
    Percent(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    /// In the flow of the container.
    #[default]
    Relative,
    /// Out of the flow: placed by `inset` from the container's edges.
    Absolute,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overflow {
    /// Content may go past the node's box and is drawn there.
    #[default]
    Visible,
    /// Content is cut at the node's box.
    Clip,
    /// Cut like `Clip`; the scrolling itself comes with the widgets.
    Scroll,
}

/// A row or column of a grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    Px(f32),
    /// A share of what the fixed tracks leave.
    Fr(f32),
    /// As large as its cells need.
    Auto,
}

/// A distance on each side. Used for padding and margin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Sides {
    pub const ZERO: Sides = Sides { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };

    pub const fn all(v: f32) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    /// `x` left and right, `y` top and bottom.
    pub const fn xy(x: f32, y: f32) -> Self {
        Self { top: y, right: x, bottom: y, left: x }
    }

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self { top, right, bottom, left }
    }
}

/// A soft shadow of a node's box (see [`crate::DrawCmd::Shadow`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    // --- Layout ---------------------------------------------------------------------------------------------------------
    pub display: Display,
    pub direction: Direction,
    /// Children that do not fit go on to another line.
    pub wrap: bool,
    /// Space between children: along a row, down a column.
    pub gap: (f32, f32),
    pub padding: Sides,
    pub margin: Sides,
    pub width: Dim,
    pub height: Dim,
    pub min_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    /// Width over height; the side that is not set follows.
    pub aspect_ratio: Option<f32>,
    /// How much of the free space along the container's direction this node takes, in proportion to its siblings.
    pub grow: f32,
    /// How much this node gives up when its siblings do not fit.
    pub shrink: f32,
    /// How the children sit across the direction.
    pub align: Align,
    /// How this node sits across the direction of its container, when that is not what the container says.
    pub align_self: Option<Align>,
    /// How the children sit along the direction.
    pub justify: Justify,
    pub position: Position,
    /// For an absolute node: the distance from each edge of the container (`None`: not tied to that edge).
    pub inset: (Option<f32>, Option<f32>, Option<f32>, Option<f32>), // top, right, bottom, left
    /// Drawn after the siblings with a lower number (default 0); equal numbers keep the order of the tree.
    pub z_index: i32,
    pub overflow: Overflow,
    /// Grid tracks.
    pub columns: Vec<Track>,
    pub rows: Vec<Track>,
    /// How many columns and rows this node takes in a grid.
    pub span: (u16, u16),
    /// The node keeps clear of what the system keeps for itself (a notch, bars). Meant for the root of a screen.
    pub safe_area: bool,

    // --- Looks ----------------------------------------------------------------------------------------------------------
    pub background: Option<Fill>,
    pub border: Option<(f32, Color)>,
    /// The corners, clockwise from the top left.
    pub radii: [f32; 4],
    pub shadows: Vec<Shadow>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            display: Display::Flex,
            direction: Direction::Row,
            wrap: false,
            gap: (0.0, 0.0),
            padding: Sides::ZERO,
            margin: Sides::ZERO,
            width: Dim::Hug,
            height: Dim::Hug,
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            aspect_ratio: None,
            grow: 0.0,
            shrink: 1.0,
            align: Align::Start,
            align_self: None,
            justify: Justify::Start,
            position: Position::Relative,
            inset: (None, None, None, None),
            z_index: 0,
            overflow: Overflow::Visible,
            columns: Vec::new(),
            rows: Vec::new(),
            span: (1, 1),
            safe_area: false,
            background: None,
            border: None,
            radii: [0.0; 4],
            shadows: Vec::new(),
        }
    }
}

impl Style {
    /// A row: the plan's `HStack`.
    pub fn hstack() -> Self {
        Self { direction: Direction::Row, ..Self::default() }
    }

    /// A column: the plan's `VStack`.
    pub fn vstack() -> Self {
        Self { direction: Direction::Column, ..Self::default() }
    }

    /// A grid with these columns (rows are made as needed, as tall as their content).
    pub fn grid(columns: Vec<Track>) -> Self {
        Self { display: Display::Grid, columns, ..Self::default() }
    }

    /// The root of a screen: fills the canvas, keeps clear of the safe area, children in a column.
    pub fn screen() -> Self {
        Self { direction: Direction::Column, width: Dim::Fill, height: Dim::Fill, safe_area: true, ..Self::default() }
    }

    pub fn size(mut self, width: Dim, height: Dim) -> Self {
        (self.width, self.height) = (width, height);
        self
    }

    pub fn px(self, width: f32, height: f32) -> Self {
        self.size(Dim::Px(width), Dim::Px(height))
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = (gap, gap);
        self
    }

    pub fn padding(mut self, padding: Sides) -> Self {
        self.padding = padding;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    pub fn grow(mut self, grow: f32) -> Self {
        self.grow = grow;
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(Fill::Solid(color));
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radii = [radius; 4];
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_predictable() {
        let s = Style::default();
        assert_eq!((s.width, s.height), (Dim::Hug, Dim::Hug), "a node is as large as its content unless told");
        assert_eq!(s.align, Align::Start, "children do not stretch unless told");
        assert_eq!((s.grow, s.shrink), (0.0, 1.0));
        assert!(!s.safe_area && s.background.is_none() && s.shadows.is_empty());
    }

    #[test]
    fn the_shorthands_set_what_their_names_say() {
        assert_eq!(Style::hstack().direction, Direction::Row);
        assert_eq!(Style::vstack().direction, Direction::Column);
        let screen = Style::screen();
        assert!(screen.safe_area && screen.width == Dim::Fill && screen.height == Dim::Fill);
        assert_eq!(Style::grid(vec![Track::Fr(1.0)]).display, Display::Grid);
        assert_eq!(Sides::xy(3.0, 4.0), Sides::new(4.0, 3.0, 4.0, 3.0));
    }
}
