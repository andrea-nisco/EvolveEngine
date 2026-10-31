//! A static demo of what the primitives of Phases 2 and 3 can draw: cards with soft shadows, borders and gradients, an SVG icon,
//! buttons, a bar and text in three scripts. It exists to look at (the editor's *View > UI demo*, the player's `--ui-demo`, the
//! `ui_demo` example) and to test the backend on something that looks like a real screen.
//!
//! Unlike the first version, nothing here has a coordinate: the card is a column, the HUD panel a box pushed to the right edge, and
//! the layout of the tree puts them where they belong on any screen.

use crate::source::UiSource;
use crate::UiRuntime;
use evolve_engine::scene::Scene;
use evolve_ui_core::{Align, Color, Dim, Fill, ImageId, Justify, Level, Overflow, Point, ScreenMetrics, Shadow, Sides, Style, TextStyle, UiTree};

const HEART_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M12 21 C12 21 3 14.5 3 8.5 C3 5.4 5.4 3 8.3 3 C10 3 11.4 3.8 12 5 C12.6 3.8 14 3 15.7 3 C18.6 3 21 5.4 21 8.5 C21 14.5 12 21 12 21 Z" fill="#000000"/></svg>"##;
const STAR_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M12 2 L14.8 8.6 L22 9.3 L16.6 14 L18.2 21 L12 17.3 L5.8 21 L7.4 14 L2 9.3 L9.2 8.6 Z" fill="#000000" stroke="#000000" stroke-width="1" stroke-linejoin="round"/></svg>"##;

const INK: Color = Color::rgb(28, 32, 42);
const MUTED: Color = Color::rgb(98, 106, 122);
const LINE: Color = Color::rgb(214, 219, 229);
const BLUE: Color = Color::rgb(56, 108, 232);
const BLUE_DARK: Color = Color::rgb(36, 80, 196);
const GREEN: Color = Color::rgb(46, 170, 98);
const GREEN_LIGHT: Color = Color::rgb(112, 214, 148);
const RED: Color = Color::rgb(222, 64, 72);

fn text(size: f32, weight: u16, color: Color) -> TextStyle {
    TextStyle { size, weight, color, ..TextStyle::default() }
}

/// A fill that goes from the top of a box to its bottom.
fn vertical(top: Color, bottom: Color) -> Option<Fill> {
    Some(Fill::LinearGradient { start: Point::new(0.0, 0.0), end: Point::new(0.0, 1.0), stops: vec![(0.0, top), (1.0, bottom)] })
}

fn solid(color: Color) -> Option<Fill> {
    Some(Fill::Solid(color))
}

pub struct UiDemo {
    runtime: UiRuntime,
}

impl Default for UiDemo {
    fn default() -> Self {
        Self::new()
    }
}

impl UiDemo {
    pub fn new() -> Self {
        let mut runtime = UiRuntime::new();
        let (heart, heart_size) = runtime.add_svg(HEART_SVG).expect("the heart is a valid SVG");
        let (star, star_size) = runtime.add_svg(STAR_SVG).expect("the star is a valid SVG");
        runtime.stack.push(Level::Menu, menu(heart, heart_size));
        runtime.stack.push(Level::Hud, hud(star, star_size));
        Self { runtime }
    }

    /// The screens of the demo, to look into them (the tests do).
    pub fn runtime(&mut self) -> &mut UiRuntime {
        &mut self.runtime
    }
}

impl UiSource for UiDemo {
    fn frame(&mut self, screen: &ScreenMetrics) -> &Scene {
        self.runtime.frame(screen)
    }
}

/// The main menu: a card at the top left with a title, a bar, three buttons and a footer.
fn menu(heart: ImageId, heart_size: (f32, f32)) -> UiTree {
    let mut tree = UiTree::new(Style::screen().padding(Sides::all(96.0)));
    let root = tree.root();
    let card = tree
        .add_view(
            root,
            Style {
                width: Dim::Px(560.0),
                gap: (24.0, 24.0),
                background: vertical(Color::rgb(255, 255, 255), Color::rgb(240, 243, 249)),
                border: Some((1.0, LINE)),
                radii: [28.0; 4],
                shadows: vec![
                    Shadow { offset: Point::new(0.0, 16.0), blur: 40.0, spread: 0.0, color: Color::rgba(20, 30, 60, 90) },
                    Shadow { offset: Point::new(0.0, 3.0), blur: 8.0, spread: 0.0, color: Color::rgba(20, 30, 60, 50) },
                ],
                ..Style::vstack().padding(Sides::all(40.0))
            },
        )
        .expect("the root exists");

    // Icon and titles.
    let header = tree.add_view(card, Style::hstack().gap(20.0).align(Align::Center)).unwrap();
    tree.add_image(header, heart, heart_size, Some(RED), Style::default().px(64.0, 64.0)).unwrap();
    let titles = tree.add_view(header, Style::vstack().gap(2.0)).unwrap();
    tree.add_text(titles, "Evolve Quest", text(44.0, 700, INK), Style::default()).unwrap();
    tree.add_text(titles, "Salute: 72 / 100", text(24.0, 400, MUTED), Style::default()).unwrap();

    // The bar: a track, and in it the part that is full.
    let track = tree.add_view(card, Style { width: Dim::Fill, height: Dim::Px(24.0), background: solid(Color::rgb(222, 227, 237)), radii: [12.0; 4], ..Style::vstack() }).unwrap();
    tree.add_view(track, Style { width: Dim::Percent(72.0), height: Dim::Fill, background: vertical(GREEN_LIGHT, GREEN), radii: [12.0; 4], ..Style::default() }).unwrap();

    // Buttons: a box that centers its label, with the looks it is given.
    let button = |tree: &mut UiTree, label: &str, ink: Color, looks: Style| {
        let style = Style {
            width: Dim::Fill,
            height: Dim::Px(76.0),
            radii: [20.0; 4],
            background: looks.background,
            border: looks.border,
            shadows: looks.shadows,
            ..Style::default().align(Align::Center).justify(Justify::Center)
        };
        let b = tree.add_view(card, style).unwrap();
        tree.add_text(b, label, text(32.0, 700, ink), Style::default()).unwrap();
    };
    button(
        &mut tree,
        "Gioca",
        Color::WHITE,
        Style {
            background: vertical(BLUE, BLUE_DARK),
            shadows: vec![Shadow { offset: Point::new(0.0, 6.0), blur: 14.0, spread: 0.0, color: Color::rgba(36, 80, 196, 110) }],
            ..Style::default()
        },
    );
    button(&mut tree, "Impostazioni", BLUE, Style { background: solid(Color::WHITE), border: Some((2.0, BLUE)), ..Style::default() });
    button(&mut tree, "Esci", INK, Style::default());

    // A footer in three scripts, cut by its own rounded edge.
    let footer = tree
        .add_view(
            card,
            Style { width: Dim::Fill, height: Dim::Px(80.0), background: solid(Color::rgb(232, 236, 246)), radii: [16.0; 4], overflow: Overflow::Clip, ..Style::vstack().padding(Sides::xy(20.0, 6.0)) },
        )
        .unwrap();
    tree.add_text(footer, "Perché è già così? Città, più, può.", text(22.0, 400, INK), Style::default()).unwrap();
    tree.add_text(footer, "Evolve  ·  مرحبا بالعالم  ·  こんにちは、世界  ·  a very long tail that the clip cuts", text(22.0, 400, MUTED), Style::default()).unwrap();
    tree
}

/// The HUD: a dark panel at the top right.
fn hud(star: ImageId, star_size: (f32, f32)) -> UiTree {
    let mut tree = UiTree::new(Style::screen().padding(Sides::all(96.0)));
    let root = tree.root();
    let panel = tree
        .add_view(
            root,
            Style {
                width: Dim::Px(400.0),
                align_self: Some(Align::End),
                gap: (12.0, 12.0),
                background: vertical(Color::rgb(46, 52, 68), Color::rgb(28, 32, 44)),
                border: Some((1.0, Color::rgba(255, 255, 255, 40))),
                radii: [24.0; 4],
                shadows: vec![Shadow { offset: Point::new(0.0, 10.0), blur: 28.0, spread: 0.0, color: Color::rgba(0, 0, 0, 120) }],
                ..Style::vstack().padding(Sides::all(32.0))
            },
        )
        .expect("the root exists");
    let row = tree.add_view(panel, Style::hstack().gap(20.0).align(Align::Center)).unwrap();
    tree.add_image(row, star, star_size, Some(Color::rgb(255, 200, 60)), Style::default().px(56.0, 56.0)).unwrap();
    let scores = tree.add_view(row, Style::vstack().gap(2.0)).unwrap();
    tree.add_text(scores, "Punti: 1280", text(40.0, 700, Color::WHITE), Style::default()).unwrap();
    tree.add_text(scores, "Record: 4096", text(24.0, 400, Color::rgb(170, 178, 196)), Style::default()).unwrap();
    tree.add_text(panel, "12 px   14 px   16 px", text(14.0, 400, Color::rgb(220, 224, 234)), Style::default()).unwrap();
    tree
}
