//! Turns egui keyboard and pointer state into the engine's [`Input`], so the editor's Play mode
//! and the standalone player feed games identically.

use eframe::egui::{self, Key};
use evolve_engine::geometry::Vec2;
use evolve_engine::runtime::Input;

/// The name scripts use for a key: `"Left"`, `"Space"`, `"A"`, `"Num1"`, ...
pub fn key_name(key: Key) -> String {
    match key {
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        other => format!("{other:?}"),
    }
}

/// This frame's input. `mouse` is the pointer position in world space.
pub fn collect_input(ctx: &egui::Context, mouse: Vec2) -> Input {
    ctx.input(|i| {
        let keys_pressed = i
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(key_name(*key)),
                _ => None,
            })
            .collect();
        Input { keys_down: i.keys_down.iter().map(|k| key_name(*k)).collect(), keys_pressed, mouse, mouse_down: i.pointer.primary_down() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_have_the_names_scripts_expect() {
        assert_eq!(key_name(Key::ArrowLeft), "Left");
        assert_eq!(key_name(Key::Space), "Space");
        assert_eq!(key_name(Key::A), "A");
    }
}
