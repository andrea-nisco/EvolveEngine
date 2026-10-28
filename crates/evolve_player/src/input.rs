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

/// This frame's input. `mouse` is the pointer position in world space. The whole window is the game (the standalone player).
pub fn collect_input(ctx: &egui::Context, mouse: Vec2) -> Input {
    collect_input_over(ctx, mouse, true)
}

/// The same, for a game that is shown in a part of the window (the editor's Game view): `over_game` says whether the pointer is over
/// it. The mouse wheel, and the right button going down, only count then; a right button that goes up always counts, so that a drag
/// that began over the game ends even if the pointer left it.
pub fn collect_input_over(ctx: &egui::Context, mouse: Vec2, over_game: bool) -> Input {
    ctx.input(|i| {
        let keys_pressed = i
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(key_name(*key)),
                _ => None,
            })
            .collect();
        let right = egui::PointerButton::Secondary;
        let wheel: f32 = i.events.iter().filter_map(|e| if let egui::Event::MouseWheel { delta, .. } = e { Some(delta.y) } else { None }).sum();
        Input {
            keys_down: i.keys_down.iter().map(|k| key_name(*k)).collect(),
            keys_pressed,
            mouse,
            mouse_down: i.pointer.primary_down(),
            mouse_right_down: i.pointer.button_down(right),
            mouse_right_pressed: over_game && i.pointer.button_pressed(right),
            mouse_right_released: i.pointer.button_released(right),
            wheel: if over_game && wheel.is_finite() { wheel } else { 0.0 },
        }
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

    /// Runs one egui frame with these events and returns what the game would be given for it.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, over_game: bool) -> Input {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), events, ..Default::default() };
        run(ctx, raw);
        collect_input_over(ctx, Vec2::new(5.0, 6.0), over_game)
    }

    /// One egui frame. (The textures it asks for are dropped on purpose: nothing here draws.)
    fn run(ctx: &egui::Context, raw: egui::RawInput) {
        ctx.run_ui(raw, |_ui| {}).textures_delta.clear();
    }

    fn right(pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(100.0, 100.0), button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE }
    }

    fn wheel(dy: f32) -> egui::Event {
        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, dy), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move }
    }

    #[test]
    fn the_right_button_and_the_wheel_reach_the_game_only_over_it_and_a_release_always_does() {
        for over in [true, false] {
            let ctx = egui::Context::default();
            frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))], over);
            let down = frame(&ctx, vec![right(true), wheel(2.0), wheel(1.0)], over);
            assert!(down.mouse_right_down, "the button is down, wherever the pointer is");
            assert_eq!(down.mouse_right_pressed, over, "it went down over the game: {over}");
            assert_eq!(down.wheel, if over { 3.0 } else { 0.0 }, "the wheel of the frame, added up, over the game only");
            assert_eq!(down.mouse, Vec2::new(5.0, 6.0));
            let held = frame(&ctx, vec![], over);
            assert!(held.mouse_right_down && !held.mouse_right_pressed && !held.mouse_right_released && held.wheel == 0.0);
            // Let go with the pointer anywhere: the game is told, so that a drag ends.
            let up = frame(&ctx, vec![right(false)], over);
            assert!(!up.mouse_right_down && up.mouse_right_released, "released, over the game or not: {over}");
            assert!(!frame(&ctx, vec![], over).mouse_right_released, "once");
        }
    }

    #[test]
    fn the_standalone_player_is_all_game() {
        let ctx = egui::Context::default();
        frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))], true);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), events: vec![right(true), wheel(-1.0)], ..Default::default() };
        run(&ctx, raw);
        let input = collect_input(&ctx, Vec2::ZERO);
        assert!(input.mouse_right_pressed && input.wheel == -1.0);
    }

    // (egui itself refuses a wheel that is not a number when it is built with debug assertions.)
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_wheel_that_is_not_a_number_is_nothing() {
        let ctx = egui::Context::default();
        let input = frame(&ctx, vec![wheel(f32::NAN)], true);
        assert_eq!(input.wheel, 0.0);
    }
}
