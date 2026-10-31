//! Evolve UI, text: fonts, shaping and glyph outlines (ADR 0002, `docs/adr/`).
//!
//! [`TextSystem`] shapes a string with `cosmic-text` (fallback between fonts, right-to-left text, wrapping, ellipsis) into
//! [`evolve_ui_core::TextRun`]s: glyph indices with positions, in logical pixels. It also gives the outline of a glyph as a
//! [`evolve_ui_core::UiPath`] in em units, made once per `(font, glyph)` and kept. A backend draws the outline scaled by the
//! run's size, so text stays sharp at any scale.
//!
//! The system only ever uses the fonts it was given: the default ones are inside the program ([`fonts`]), never the machine's
//! own fonts, so a text looks the same everywhere (and in the browser, which has none to offer).

pub mod fonts;
mod outline;
mod system;

pub use evolve_ui_core::{TextAlign, TextLayout, TextStyle, Typesetter};
pub use system::TextSystem;

#[cfg(test)]
mod tests;
