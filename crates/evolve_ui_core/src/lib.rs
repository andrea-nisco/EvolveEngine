//! Evolve UI, the neutral part: what every UI backend reads.
//!
//! The UI never draws anything itself. It produces a [`draw::DrawList`]: rounded rectangles, paths, text runs and clip
//! brackets, in *logical* units and with no engine or GPU types in it (ADR 0001, `docs/adr/`). A backend turns the list into
//! pictures: `evolve_ui_scene` makes a temporary scene of `evolve_engine` for the existing renderers; a future wgpu backend
//! could read the very same list.
//!
//! Besides the draw list this crate holds what makes it: the units and the screen ([`units`]), the style of a node ([`style`]), and
//! the tree that lays nodes out with Taffy and draws them ([`tree`]). Nothing here depends on the engine, so layout, text and
//! state can be tested without a window or a graphics card.

pub mod draw;
pub mod geometry;
pub mod path;
pub mod stack;
pub mod style;
pub mod text;
pub mod tree;
pub mod units;

pub use draw::{DrawCmd, DrawList, Fill, FontId, GlyphPos, ImageId, TextRun, MAX_GRADIENT_STOPS};
pub use geometry::{Color, Point, Rect};
pub use path::{PathCmd, UiPath};
pub use stack::{Level, ScreenStack};
pub use style::{Align, Dim, Direction, Display, Justify, Overflow, Position, Shadow, Sides, Style, Track};
pub use text::{TextAlign, TextLayout, TextStyle, Typesetter};
pub use tree::{NodeId, TreeStats, UiTree};
pub use units::{Insets, MatchMode, ScreenMetrics, REFERENCE_SIZE};

#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod tree_tests;
