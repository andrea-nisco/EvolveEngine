//! Standalone player for EvolveEngine projects.
//!
//! The same code runs as a desktop program and, compiled to WebAssembly, in a browser page.
//! The editor uses [`input`] for its Play mode and [`bundle`] to export games.

pub mod app;
pub mod bundle;
pub mod input;
#[cfg(target_arch = "wasm32")]
mod web;

pub use app::PlayerApp;
