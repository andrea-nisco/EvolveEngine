//! Where a program that draws a game gets the UI from, and how the UI is placed over the picture of the game.
//!
//! The UI lives in *logical* pixels, on a screen whose size follows from the physical one by the rules of
//! [`evolve_ui_core::ScreenMetrics`] (the reference screen, the match mode, the player's own scale, the safe area).

use crate::build::logical_view;
use evolve_engine::render;
use evolve_engine::scene::Scene;
use evolve_engine::tiny_skia::Pixmap;
use evolve_ui_core::ScreenMetrics;

/// Something that knows what the UI looks like now: the UI runtime of a game, or a demo.
pub trait UiSource {
    /// The scene of the UI for `screen`. It is the same scene (and the same change stamps) while nothing changed, so a renderer can
    /// tell that there is nothing to draw again.
    fn frame(&mut self, screen: &ScreenMetrics) -> &Scene;
}

/// Draws the UI of `source` over what `target` holds (the picture of the game), with the CPU renderer, for a screen as large as the
/// picture and with the default rules of the plan. The UI is drawn last, so it is on top of everything in the game.
pub fn draw_over(source: &mut dyn UiSource, target: &mut Pixmap) {
    draw_over_screen(source, target, &ScreenMetrics::new((target.width() as f32, target.height() as f32)));
}

/// Like [`draw_over`], for a screen with other rules (a scale of the player's, a safe area...). `screen.pixels` should be the size
/// of `target`.
pub fn draw_over_screen(source: &mut dyn UiSource, target: &mut Pixmap, screen: &ScreenMetrics) {
    let scene = source.frame(screen);
    render::render_content(scene, logical_view(screen.scale()), target);
}
