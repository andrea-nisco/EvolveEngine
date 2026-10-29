//! EvolveEngine: a vector-first 2D engine core.
//!
//! - [`scene`]: the node tree (groups and shapes)
//! - [`path`]: Bézier paths
//! - [`style`]: fills, strokes, gradients
//! - [`camera`]: pan / zoom / rotate view of the editor
//! - [`game_camera`]: the camera object that frames what the player sees
//! - [`render`]: tiny-skia rasterizer
//! - [`revision`]: change tracking (what changed since a renderer last looked)
//! - [`io`]: project files and SVG import

pub mod animation;
pub mod boolean;
pub mod camera;
pub mod clipboard;
pub mod export_image;
pub mod game_camera;
pub mod geometry;
pub mod io;
pub mod morph;
pub mod path;
pub mod physics;
mod physics_schedule;
pub mod project;
pub mod render;
pub mod revision;
#[cfg(test)]
mod revision_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod hit_tests;
#[cfg(test)]
mod remove_tests;
pub mod rig;
pub mod runtime;
pub mod scene;
pub mod script;
pub mod style;
pub mod symbol;

pub use geometry::{Affine2, Rect, Vec2};
pub use tiny_skia;
