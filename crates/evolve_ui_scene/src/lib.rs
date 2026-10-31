//! Evolve UI, the first backend: a draw list becomes a temporary scene of `evolve_engine` (ADR 0001).
//!
//! The scene made here is *not* the user's scene: it is built from the UI, drawn on top of the game and thrown away. It is
//! never saved and never shown in the editor's hierarchy. The renderers of the engine (CPU and GPU) draw it like any
//! scene, with a view that maps logical pixels to physical ones ([`logical_view`]).
//!
//! This is the only crate of the UI that knows the engine's types.
//!
//! - rounded rectangles and paths become shapes;
//! - a run of text becomes one *instance* per glyph of a symbol made once per `(font, glyph)`, so the outline of a letter
//!   is shared by every place it is used and tessellated once by the GPU renderer;
//! - `PushClip` … `PopClip` becomes a clipping group (its top child is the mask);
//! - a linear gradient is the engine's gradient paint; a shadow is a few rounded rectangles, each larger and fainter than the next
//!   ([`shadow`]); an SVG image is a symbol made by `UiLayer::add_svg`, placed as an instance that can take one color (`tint`).
//!
//! [`UiLayer`] keeps the scene from frame to frame and does nothing when the draw list is the same as the last one.

mod build;
pub mod demo;
mod runtime;
mod shadow;
mod shapes;
mod source;

pub use build::{build_scene, logical_view, render_cpu, UiLayer};
pub use runtime::UiRuntime;
pub use source::{draw_over, draw_over_screen, UiSource};

#[cfg(test)]
mod demo_tests;
#[cfg(test)]
mod layer_tests;
#[cfg(test)]
mod tests;
