//! The demo on the four screens of the plan: where its pieces end up, worked out from the rules of the layout.

use crate::demo::UiDemo;
use crate::UiSource;
use evolve_ui_core::{Level, ScreenMetrics};

const SCREENS: [(f32, f32); 4] = [(1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1080.0), (1080.0, 2400.0)];

/// The box of the first child of the root of the top screen of a level (the card, the HUD panel), in logical pixels.
fn panel(demo: &mut UiDemo, level: Level) -> evolve_ui_core::Rect {
    let tree = demo.runtime().stack.top_mut(level).expect("a screen");
    let first = tree.children(tree.root())[0];
    tree.rect(first).expect("laid out")
}

#[test]
fn the_card_is_at_the_top_left_and_the_hud_at_the_top_right_on_every_screen() {
    for pixels in SCREENS {
        let screen = ScreenMetrics::new(pixels);
        let canvas = screen.canvas();
        let mut demo = UiDemo::new();
        demo.frame(&screen);
        let (card, hud) = (panel(&mut demo, Level::Menu), panel(&mut demo, Level::Hud));
        assert!((card.x - 96.0).abs() < 0.01 && (card.y - 96.0).abs() < 0.01, "{pixels:?}: card {card:?}");
        assert!((card.w - 560.0).abs() < 0.01 && card.h > 500.0 && card.h < 700.0, "{pixels:?}: card {card:?}");
        assert!((hud.right() - (canvas.0 - 96.0)).abs() < 0.01, "{pixels:?}: the HUD panel ends 96 from the right edge: {hud:?}");
        assert!((hud.y - 96.0).abs() < 0.01 && (hud.w - 400.0).abs() < 0.01, "{pixels:?}: hud {hud:?}");
        assert!(card.right() < hud.x, "{pixels:?}: they do not meet");
    }
}

#[test]
fn a_notch_pushes_both_screens_down() {
    let phone = ScreenMetrics::new((1080.0, 2400.0)).with_safe_area(evolve_ui_core::Insets::new(0.0, 120.0, 0.0, 0.0));
    let mut demo = UiDemo::new();
    demo.frame(&phone);
    let logical_top = 120.0 / phone.scale();
    for level in [Level::Menu, Level::Hud] {
        let r = panel(&mut demo, level);
        assert!((r.y - (96.0 + logical_top)).abs() < 0.01, "{level:?}: {r:?}");
    }
}

#[test]
fn the_demo_draws_nothing_again_while_the_screen_is_the_same() {
    let screen = ScreenMetrics::new((1280.0, 720.0));
    let mut demo = UiDemo::new();
    demo.frame(&screen);
    let before = demo.runtime().layer().scene().revision();
    let stats = (demo.runtime().stack.screens_mut(Level::Menu)[0].stats(), demo.runtime().stack.screens_mut(Level::Hud)[0].stats());
    for _ in 0..10 {
        demo.frame(&screen);
    }
    assert_eq!(demo.runtime().layer().scene().changed_since(&before), evolve_engine::revision::Changes::NONE, "the scene's stamps did not move");
    assert_eq!((demo.runtime().stack.screens_mut(Level::Menu)[0].stats(), demo.runtime().stack.screens_mut(Level::Hud)[0].stats()), stats, "no layout, no shaping");
    // Another size of the window is a new layout, for both screens.
    demo.frame(&ScreenMetrics::new((1920.0, 1080.0)));
    assert_ne!(demo.runtime().layer().scene().changed_since(&before), evolve_engine::revision::Changes::NONE);
}

#[test]
fn the_user_scale_makes_the_card_larger_on_the_screen() {
    let mut demo = UiDemo::new();
    let normal = ScreenMetrics::new((1920.0, 1080.0));
    demo.frame(&normal);
    let card = panel(&mut demo, Level::Menu);
    let big = normal.with_user_scale(1.5);
    demo.frame(&big);
    let card_big = panel(&mut demo, Level::Menu);
    assert_eq!(card.w, card_big.w, "in logical pixels the card is the same");
    assert_eq!(big.scale(), 1.5, "and each logical pixel is half as large again");
    assert!(big.canvas().0 < normal.canvas().0, "so less of them fit on the screen");
}
