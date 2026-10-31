//! Gradients, shadows, images and the persistence of the UI layer.

use crate::tests::{alpha, ink_box, text_list};
use crate::{render_cpu, UiLayer};
use evolve_engine::revision::Changes;
use evolve_engine::tiny_skia::Pixmap;
use evolve_ui_core::{Color, DrawCmd, DrawList, Fill, ImageId, Point, Rect};
use evolve_ui_text::TextSystem;

fn gradient_rect(list: &mut DrawList, rect: Rect, start: Point, end: Point, stops: Vec<(f32, Color)>) {
    list.push(DrawCmd::RoundedRect { rect, radii: [0.0; 4], fill: Some(Fill::LinearGradient { start, end, stops }), border: None });
}

fn red_at(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
    pixmap.pixel(x, y).map_or(0, |p| p.red())
}

#[test]
fn a_linear_gradient_goes_from_the_first_color_to_the_last() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    gradient_rect(&mut list, Rect::new(0.0, 0.0, 100.0, 10.0), Point::new(20.0, 0.0), Point::new(80.0, 0.0), vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
    let mut pixmap = Pixmap::new(100, 10).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(red_at(&pixmap, 5, 5), 0, "before the start the first color goes on");
    assert_eq!(red_at(&pixmap, 95, 5), 255, "after the end the last one");
    let middle = red_at(&pixmap, 50, 5);
    assert!((120..=135).contains(&middle), "half way: {middle}");
    assert!(red_at(&pixmap, 35, 5) < middle && middle < red_at(&pixmap, 65, 5), "monotonic");
}

#[test]
fn gradient_stops_are_sorted_clamped_and_bad_ones_draw_nothing() {
    let mut system = TextSystem::new();
    let at = |system: &mut TextSystem, stops: Vec<(f32, Color)>, start: Point, end: Point| {
        let mut list = DrawList::new();
        gradient_rect(&mut list, Rect::new(0.0, 0.0, 10.0, 10.0), start, end, stops);
        let mut pixmap = Pixmap::new(10, 10).expect("pixmap");
        render_cpu(&list, system, 1.0, &mut pixmap);
        pixmap
    };
    let (a, b) = (Point::new(0.0, 0.0), Point::new(10.0, 0.0));
    // Stops given out of order give the same picture as in order.
    let shuffled = at(&mut system, vec![(1.0, Color::WHITE), (0.0, Color::BLACK)], a, b);
    let ordered = at(&mut system, vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], a, b);
    assert_eq!(shuffled.data(), ordered.data());
    assert!(at(&mut system, Vec::new(), a, b).data().iter().all(|v| *v == 0), "no stops, no fill");
    assert!(at(&mut system, vec![(0.0, Color::BLACK)], Point::new(f32::NAN, 0.0), b).data().iter().all(|v| *v == 0), "a line that is not a number draws nothing");
    // A position that is not a number is dropped; the stop that is left gives a flat fill.
    let flat = at(&mut system, vec![(f32::NAN, Color::WHITE), (0.5, Color::rgb(255, 0, 0))], a, b);
    assert_eq!(flat.pixel(5, 5).map(|p| (p.red(), p.green())), Some((255, 0)));
}

#[test]
fn a_shadow_fades_across_the_edge_like_a_blurred_one() {
    let mut system = TextSystem::new();
    let rect = Rect::new(40.0, 40.0, 120.0, 120.0);
    let mut list = DrawList::new();
    list.shadow(rect, [0.0; 4], Point::ZERO, 16.0, Color::BLACK);
    let mut pixmap = Pixmap::new(200, 200).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    // Along the middle of the top edge, `d` pixels outside it.
    let alpha_at = |d: i32| f32::from(alpha(&pixmap, 100, (40 - d) as u32)) / 255.0;
    for d in [-12, -8, -4, 0, 4, 8, 12] {
        let want = crate::shadow::profile(d as f32 - 0.5, 16.0);
        assert!((alpha_at(d) - want).abs() < 0.09, "{d} px outside: {} against {want}", alpha_at(d));
    }
    assert_eq!(alpha(&pixmap, 100, 100), 255, "full strength in the middle");
    assert_eq!(alpha(&pixmap, 100, 15), 0, "nothing beyond the reach");
    assert!(ink_box(&pixmap).is_some_and(|(l, t, r, b)| l >= 20 && t >= 20 && r <= 180 && b <= 180), "the shadow stays within its reach");
}

#[test]
fn a_shadow_is_moved_by_its_offset_tinted_by_its_color_and_lies_under_what_comes_after() {
    let mut system = TextSystem::new();
    let rect = Rect::new(20.0, 20.0, 60.0, 60.0);
    let mut list = DrawList::new();
    list.shadow(rect, [8.0; 4], Point::new(0.0, 12.0), 10.0, Color::rgba(0, 0, 255, 128));
    list.rect(rect, [8.0; 4], Color::WHITE);
    let mut pixmap = Pixmap::new(100, 120).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(pixmap.pixel(50, 50).map(|p| (p.red(), p.alpha())), Some((255, 255)), "the box covers the shadow");
    let below = pixmap.pixel(50, 88).expect("pixel");
    assert!(below.alpha() > 40 && below.alpha() < 140, "alpha {}: a faint shadow below the box", below.alpha());
    assert!(below.blue() > 0 && below.red() == 0, "in the shadow's color");
    assert!(alpha(&pixmap, 50, 100) > alpha(&pixmap, 50, 8), "it reaches further down than up: the offset moved it down");
}

const SQUARE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M2 2h20v20H2z" fill="#ff0000"/></svg>"##;

#[test]
fn an_svg_image_is_fitted_in_proportion_and_centered_in_its_rectangle() {
    let mut system = TextSystem::new();
    let mut layer = UiLayer::new();
    let image = layer.add_svg(SQUARE_SVG).expect("a valid SVG");
    let mut list = DrawList::new();
    list.image(Rect::new(10.0, 10.0, 40.0, 20.0), image, None);
    layer.update(&list, &mut system);
    let mut pixmap = Pixmap::new(60, 40).expect("pixmap");
    layer.draw_cpu(&mut pixmap, 1.0);
    // The drawing is a 20 x 20 square: the height of the rectangle decides, and it sits in the middle.
    assert_eq!(ink_box(&pixmap), Some((20, 10, 39, 29)));
    assert_eq!(pixmap.pixel(30, 20).map(|p| (p.red(), p.green(), p.blue())), Some((255, 0, 0)), "its own color");
}

#[test]
fn a_tint_replaces_every_color_of_an_image_and_scale_makes_it_larger_not_blurrier() {
    let mut system = TextSystem::new();
    let mut layer = UiLayer::new();
    let image = layer.add_svg(SQUARE_SVG).expect("a valid SVG");
    let mut list = DrawList::new();
    list.image(Rect::new(0.0, 0.0, 10.0, 10.0), image, Some(Color::rgb(0, 0, 255)));
    layer.update(&list, &mut system);
    let mut pixmap = Pixmap::new(20, 20).expect("pixmap");
    layer.draw_cpu(&mut pixmap, 2.0);
    assert_eq!(pixmap.pixel(10, 10).map(|p| (p.red(), p.blue())), Some((0, 255)), "tinted");
    assert_eq!(ink_box(&pixmap), Some((0, 0, 19, 19)), "at 2x it fills 20 pixels, edge to edge, no soft border");
    assert_eq!(alpha(&pixmap, 0, 0), 255);
}

#[test]
fn bad_images_are_refused_or_skipped_without_a_crash() {
    let mut system = TextSystem::new();
    let mut layer = UiLayer::new();
    assert!(layer.add_svg(b"this is not an svg").is_err());
    assert!(layer.add_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#).is_err(), "an SVG with nothing in it");
    let mut list = DrawList::new();
    list.image(Rect::new(0.0, 0.0, 10.0, 10.0), ImageId(7), None);
    let image = layer.add_svg(SQUARE_SVG).expect("a valid SVG");
    list.image(Rect::new(0.0, 0.0, 0.0, 10.0), image, None);
    layer.update(&list, &mut system);
    let mut pixmap = Pixmap::new(10, 10).expect("pixmap");
    layer.draw_cpu(&mut pixmap, 1.0);
    assert!(ink_box(&pixmap).is_none(), "an unknown image and an empty rectangle draw nothing");
}

#[test]
fn an_unchanged_list_leaves_the_scene_and_its_change_stamps_alone() {
    let mut system = TextSystem::new();
    let mut layer = UiLayer::new();
    let (mut list, _, _) = text_list(&mut system, "Evolve", 16.0, Color::BLACK, 1.0);
    list.rect(Rect::new(0.0, 0.0, 50.0, 20.0), [4.0; 4], Color::WHITE);
    assert!(layer.update(&list, &mut system), "the first list is built");
    let before = layer.scene().revision();
    assert!(!layer.update(&list.clone(), &mut system), "an equal list is not built again");
    assert_eq!(layer.scene().changed_since(&before), Changes::NONE, "no stamp moved: a renderer has nothing to redo");

    let mut changed = list.clone();
    changed.rect(Rect::new(0.0, 0.0, 5.0, 5.0), [0.0; 4], Color::BLACK);
    assert!(layer.update(&changed, &mut system));
    assert_ne!(layer.scene().changed_since(&before), Changes::NONE, "a different list is noticed");
}

#[test]
fn a_changed_list_replaces_the_nodes_but_keeps_the_symbols_of_letters() {
    let mut system = TextSystem::new();
    let mut layer = UiLayer::new();
    let (first, _, _) = text_list(&mut system, "Evolve", 16.0, Color::BLACK, 1.0);
    layer.update(&first, &mut system);
    let (symbols, nodes) = (layer.scene().symbols.len(), layer.scene().all_ids().len());
    let (second, _, _) = text_list(&mut system, "Evolve", 16.0, Color::rgb(200, 0, 0), 1.0);
    assert!(layer.update(&second, &mut system));
    assert_eq!(layer.scene().symbols.len(), symbols, "no new symbol: the same letters");
    assert_eq!(layer.scene().all_ids().len(), nodes, "the old nodes are gone, not piled up");
    let (third, _, _) = text_list(&mut system, "Evolve Quest", 16.0, Color::BLACK, 1.0);
    layer.update(&third, &mut system);
    assert!(layer.scene().symbols.len() > symbols, "new letters get symbols");
}

#[test]
fn a_gradient_uses_at_most_the_limit_of_stops() {
    use evolve_engine::style::{GradientKind, Paint};
    let many: Vec<(f32, Color)> = (0..100).map(|i| (i as f32 / 99.0, Color::rgb(i as u8, 0, 0))).collect();
    let fill = Fill::LinearGradient { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0), stops: many };
    let Some(Paint::Gradient(gradient)) = crate::build::paint(&fill) else { panic!("a gradient") };
    assert_eq!(gradient.stops.len(), evolve_ui_core::MAX_GRADIENT_STOPS);
    assert!(matches!(gradient.kind, GradientKind::Linear { .. }));
    assert!(gradient.stops.windows(2).all(|w| w[0].offset <= w[1].offset), "in order");
    // Positions outside 0..=1 are pulled in.
    let wild = Fill::LinearGradient { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0), stops: vec![(-3.0, Color::BLACK), (7.0, Color::WHITE)] };
    let Some(Paint::Gradient(gradient)) = crate::build::paint(&wild) else { panic!("a gradient") };
    assert_eq!((gradient.stops[0].offset, gradient.stops[1].offset), (0.0, 1.0));
}
