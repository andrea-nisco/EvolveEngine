use crate::shapes::{convert_path, rounded_rect};
use crate::{build_scene, logical_view, render_cpu};
use evolve_engine::scene::{NodeKind, Scene};
use evolve_engine::tiny_skia::Pixmap;
use evolve_ui_core::{Color, DrawList, Fill, PathCmd, Point, Rect, UiPath};
use evolve_ui_text::{TextStyle, TextSystem};

pub(crate) fn text_list(system: &mut TextSystem, text: &str, size: f32, color: Color, scale: f32) -> (DrawList, f32, f32) {
    let layout = system.layout(text, &TextStyle { size, color, ..TextStyle::default() }, scale);
    let mut list = DrawList::new();
    for run in layout.runs {
        list.text(Point::new(4.0, 4.0), run);
    }
    (list, layout.width, layout.height)
}

fn count(scene: &Scene, wanted: fn(&NodeKind) -> bool) -> usize {
    let mut n = 0;
    scene.for_each_node(|node| n += wanted(&node.kind) as usize);
    n
}

/// Alpha of a pixel, 0 for one outside the picture.
pub(crate) fn alpha(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
    pixmap.pixel(x, y).map_or(0, |p| p.alpha())
}

/// The box around every pixel that has ink.
pub(crate) fn ink_box(pixmap: &Pixmap) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..pixmap.height() {
        for x in 0..pixmap.width() {
            if alpha(pixmap, x, y) > 0 {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    found
}

#[test]
fn a_text_is_one_instance_per_glyph_and_one_symbol_per_distinct_glyph() {
    let mut system = TextSystem::new();
    let (list, _, _) = text_list(&mut system, "eee a", 16.0, Color::BLACK, 1.0);
    let scene = build_scene(&list, &mut system);
    assert_eq!(count(&scene, |k| matches!(k, NodeKind::Instance(_))), 4, "three e and an a; the space draws nothing");
    assert_eq!(scene.symbols.len(), 2, "e and a");
    assert!(!scene.has_symbol_cycle());
    assert_eq!(count(&scene, |k| matches!(k, NodeKind::Camera(_))), 0, "the UI scene has no camera");
    let mut ids = scene.all_ids();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), scene.all_ids().len(), "node ids are unique");
}

#[test]
fn the_same_letter_in_two_texts_shares_its_outline() {
    let mut system = TextSystem::new();
    let (mut list, _, _) = text_list(&mut system, "oo", 16.0, Color::BLACK, 1.0);
    let (other, _, _) = text_list(&mut system, "o", 40.0, Color::WHITE, 1.0);
    list.cmds.extend(other.cmds);
    let scene = build_scene(&list, &mut system);
    assert_eq!(scene.symbols.len(), 1, "one symbol however many sizes and colors");
    assert_eq!(system.cached_outlines(), 1);
}

#[test]
fn a_filled_rectangle_covers_exactly_its_pixels() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.rect(Rect::new(10.0, 20.0, 30.0, 8.0), [0.0; 4], Color::rgb(255, 0, 0));
    let mut pixmap = Pixmap::new(64, 64).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(ink_box(&pixmap), Some((10, 20, 39, 27)));
    assert_eq!(pixmap.pixel(20, 24).map(|p| (p.red(), p.green(), p.alpha())), Some((255, 0, 255)));
}

#[test]
fn rounded_corners_leave_the_corner_pixel_empty_and_the_middle_filled() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.rect(Rect::new(0.0, 0.0, 40.0, 40.0), [16.0; 4], Color::BLACK);
    let mut pixmap = Pixmap::new(40, 40).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 0, 0), 0, "the corner is cut");
    assert_eq!(alpha(&pixmap, 39, 39), 0);
    assert_eq!(alpha(&pixmap, 20, 0), 255, "the straight part of the top edge");
    assert_eq!(alpha(&pixmap, 20, 20), 255);
    // The corner is an arc, not a diagonal cut: a radius of 16 passes 4.7 pixels from both edges at 45 degrees.
    assert_eq!(alpha(&pixmap, 6, 6), 255, "inside the arc");
    assert_eq!(alpha(&pixmap, 3, 3), 0, "outside the arc");
    // A radius is cut to what fits: asking for a huge one gives a pill, not a broken shape.
    let mut pill = DrawList::new();
    pill.rect(Rect::new(0.0, 0.0, 40.0, 10.0), [1000.0; 4], Color::BLACK);
    let mut pixmap = Pixmap::new(40, 10).expect("pixmap");
    render_cpu(&pill, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 0, 0), 0);
    assert_eq!(alpha(&pixmap, 20, 5), 255);
    let path = rounded_rect(Rect::new(0.0, 0.0, 40.0, 10.0), [1000.0; 4]);
    assert!(path.subpaths[0].anchors.iter().all(|a| (0.0..=40.0).contains(&a.pos.x) && (0.0..=10.0).contains(&a.pos.y)), "no anchor outside the rectangle");
}

#[test]
fn radii_per_corner_are_independent() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.rect(Rect::new(0.0, 0.0, 40.0, 40.0), [20.0, 0.0, 0.0, 0.0], Color::BLACK);
    let mut pixmap = Pixmap::new(40, 40).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 0, 0), 0, "top-left is round");
    assert_eq!(alpha(&pixmap, 39, 0), 255, "top-right is square");
    assert_eq!(alpha(&pixmap, 0, 39), 255);
    assert_eq!(alpha(&pixmap, 39, 39), 255);
}

#[test]
fn a_border_stays_inside_the_rectangle() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.push(evolve_ui_core::DrawCmd::RoundedRect {
        rect: Rect::new(4.0, 4.0, 32.0, 32.0),
        radii: [0.0; 4],
        fill: Some(Fill::Solid(Color::WHITE)),
        border: Some((4.0, Color::rgb(0, 0, 255))),
    });
    let mut pixmap = Pixmap::new(40, 40).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(ink_box(&pixmap), Some((4, 4, 35, 35)), "nothing outside the rectangle");
    let blue = |x, y| pixmap.pixel(x, y).map(|p| (p.red(), p.blue()));
    assert_eq!(blue(5, 20), Some((0, 255)), "inside the border band");
    assert_eq!(blue(20, 20), Some((255, 255)), "white fill inside");
}

#[test]
fn a_clip_cuts_what_is_drawn_inside_it() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.push_clip(Rect::new(10.0, 10.0, 10.0, 10.0), [0.0; 4]);
    list.rect(Rect::new(0.0, 0.0, 40.0, 40.0), [0.0; 4], Color::BLACK);
    list.pop_clip();
    list.rect(Rect::new(30.0, 30.0, 5.0, 5.0), [0.0; 4], Color::BLACK);
    let mut pixmap = Pixmap::new(40, 40).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 15, 15), 255);
    assert_eq!(alpha(&pixmap, 5, 5), 0, "outside the clip");
    assert_eq!(alpha(&pixmap, 25, 15), 0);
    assert_eq!(alpha(&pixmap, 32, 32), 255, "after the clip, drawing is free again");
}

#[test]
fn text_is_cut_by_a_clip() {
    let mut system = TextSystem::new();
    let (text, width, _) = text_list(&mut system, "Evolve Quest", 24.0, Color::BLACK, 1.0);
    let mut list = DrawList::new();
    list.push_clip(Rect::new(0.0, 0.0, width / 2.0, 60.0), [0.0; 4]);
    list.cmds.extend(text.cmds);
    list.pop_clip();
    let mut pixmap = Pixmap::new(200, 60).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    let (_, _, right, _) = ink_box(&pixmap).expect("some text");
    assert!(right as f32 <= width / 2.0, "ink up to {right}, clip at {}", width / 2.0);
    assert!(right as f32 > width / 4.0, "the first half is there");
}

#[test]
fn a_list_with_clips_left_open_or_closed_twice_still_makes_a_scene() {
    let mut system = TextSystem::new();
    let mut open = DrawList::new();
    open.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    open.rect(Rect::new(0.0, 0.0, 5.0, 5.0), [0.0; 4], Color::BLACK);
    let mut pixmap = Pixmap::new(10, 10).expect("pixmap");
    render_cpu(&open, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 2, 2), 255, "the open clip was closed at the end");

    let mut extra = DrawList::new();
    extra.pop_clip();
    extra.rect(Rect::new(0.0, 0.0, 5.0, 5.0), [0.0; 4], Color::BLACK);
    extra.pop_clip();
    let mut pixmap = Pixmap::new(10, 10).expect("pixmap");
    render_cpu(&extra, &mut system, 1.0, &mut pixmap);
    assert_eq!(alpha(&pixmap, 2, 2), 255, "a pop with nothing to close is ignored");
}

#[test]
fn text_has_ink_in_the_color_asked_for_and_only_inside_its_box() {
    let mut system = TextSystem::new();
    let (list, width, height) = text_list(&mut system, "Hello", 24.0, Color::rgb(0, 128, 0), 1.0);
    let mut pixmap = Pixmap::new(120, 48).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    let (left, top, right, bottom) = ink_box(&pixmap).expect("ink");
    assert!(left >= 4 && top >= 4, "{left} {top}: nothing before the origin (4, 4)");
    assert!(right as f32 <= 4.0 + width + 1.0 && bottom as f32 <= 4.0 + height + 1.0, "{right} {bottom} for a {width} x {height} text");
    // A pixel in the stem of the H is the text's color, premultiplied.
    let solid = (left..right).flat_map(|x| (top..bottom).map(move |y| (x, y))).find(|&(x, y)| alpha(&pixmap, x, y) == 255).expect("a solid pixel");
    let p = pixmap.pixel(solid.0, solid.1).expect("pixel");
    assert_eq!((p.red(), p.green(), p.blue()), (0, 128, 0));
}

#[test]
fn drawing_at_twice_the_scale_doubles_the_extent() {
    let mut system = TextSystem::new();
    let extent = |system: &mut TextSystem, scale: f32| {
        let (list, _, _) = text_list(system, "Evolve", 20.0, Color::BLACK, scale);
        let mut pixmap = Pixmap::new((160.0 * scale) as u32, (50.0 * scale) as u32).expect("pixmap");
        render_cpu(&list, system, scale, &mut pixmap);
        let (l, t, r, b) = ink_box(&pixmap).expect("ink");
        ((r - l) as f32 / scale, (b - t) as f32 / scale)
    };
    let (w1, h1) = extent(&mut system, 1.0);
    let (w2, h2) = extent(&mut system, 2.0);
    assert!((w1 - w2).abs() < 2.0 && (h1 - h2).abs() < 2.0, "{w1}x{h1} at 1, {w2}x{h2} at 2 (in logical pixels)");
    assert_eq!(logical_view(2.0).transform_point2(evolve_engine::geometry::Vec2::new(3.0, 4.0)), evolve_engine::geometry::Vec2::new(6.0, 8.0));
}

#[test]
fn the_ui_scene_is_rebuilt_equal_from_an_equal_list() {
    // Not an optimisation yet, but the property later phases rely on: the same list gives the same picture.
    let mut system = TextSystem::new();
    let (list, _, _) = text_list(&mut system, "Evolve", 16.0, Color::BLACK, 1.0);
    let draw = |system: &mut TextSystem| {
        let mut pixmap = Pixmap::new(100, 30).expect("pixmap");
        render_cpu(&list, system, 1.0, &mut pixmap);
        pixmap.data().to_vec()
    };
    assert_eq!(draw(&mut system), draw(&mut system));
}

#[test]
fn rounded_rect_geometry_is_closed_and_has_no_empty_corners() {
    let square = rounded_rect(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    assert_eq!(square.subpaths[0].anchors.len(), 4, "plain corners");
    let round = rounded_rect(Rect::new(0.0, 0.0, 10.0, 10.0), [2.0; 4]);
    assert_eq!(round.subpaths[0].anchors.len(), 8, "two anchors per rounded corner");
    assert!(round.subpaths[0].closed);
    let bad = rounded_rect(Rect::new(0.0, 0.0, 10.0, 10.0), [f32::NAN, -3.0, f32::INFINITY, 1.0]);
    assert!(bad.subpaths[0].anchors.iter().all(|a| a.pos.is_finite()), "a radius that is not a number is taken as zero");
}

#[test]
fn a_contour_that_returns_to_its_start_is_closed_without_a_duplicate_anchor() {
    let mut path = UiPath::default();
    path.move_to(Point::new(0.0, 0.0));
    path.quad_to(Point::new(5.0, 0.0), Point::new(5.0, 5.0));
    path.quad_to(Point::new(0.0, 5.0), Point::new(0.0, 0.0));
    path.close();
    let converted = convert_path(&path);
    assert_eq!(converted.subpaths.len(), 1);
    assert_eq!(converted.subpaths[0].anchors.len(), 2, "start and the corner at (5, 5)");
    assert!(converted.subpaths[0].closed);
    // The curve that came back to the start gave its incoming handle to the first anchor.
    assert_ne!(converted.subpaths[0].anchors[0].handle_in, evolve_engine::geometry::Vec2::ZERO);
    assert!(matches!(path.cmds.last(), Some(PathCmd::Close)));
}

#[test]
fn degenerate_paths_are_dropped() {
    let mut path = UiPath::default();
    path.move_to(Point::new(1.0, 1.0));
    path.close();
    path.move_to(Point::new(2.0, 2.0));
    assert!(convert_path(&path).subpaths.is_empty(), "a lone point is no contour");
}

#[test]
fn a_quadratic_curve_becomes_the_same_curve_as_a_cubic() {
    let mut path = UiPath::default();
    path.move_to(Point::new(0.0, 0.0));
    path.quad_to(Point::new(6.0, 0.0), Point::new(6.0, 6.0));
    let converted = convert_path(&path);
    let curve = converted.subpaths[0].segment(0);
    // The middle of the quadratic curve is a quarter of the start, half of the control point and a quarter of the end.
    let middle = curve.point(0.5);
    assert!((middle.x - 4.5).abs() < 1e-4 && (middle.y - 1.5).abs() < 1e-4, "{middle:?}");
    let quarter = curve.point(0.25);
    assert!((quarter.x - 2.625).abs() < 1e-4 && (quarter.y - 0.375).abs() < 1e-4, "{quarter:?}");
}

#[test]
fn clips_nest_and_each_one_cuts_to_the_overlap() {
    let mut system = TextSystem::new();
    let mut list = DrawList::new();
    list.push_clip(Rect::new(0.0, 0.0, 30.0, 30.0), [0.0; 4]);
    list.push_clip(Rect::new(20.0, 20.0, 30.0, 30.0), [0.0; 4]);
    list.rect(Rect::new(0.0, 0.0, 60.0, 60.0), [0.0; 4], Color::BLACK);
    list.pop_clip();
    list.rect(Rect::new(0.0, 0.0, 6.0, 6.0), [0.0; 4], Color::rgb(255, 0, 0));
    list.pop_clip();
    assert!(list.is_balanced());
    let mut pixmap = Pixmap::new(60, 60).expect("pixmap");
    render_cpu(&list, &mut system, 1.0, &mut pixmap);
    assert_eq!(ink_box(&pixmap).map(|(_, _, r, b)| (r, b)), Some((29, 29)), "nothing past the outer clip");
    assert_eq!(alpha(&pixmap, 25, 25), 255, "the overlap of the two clips");
    assert_eq!(alpha(&pixmap, 10, 10), 0, "inside the outer clip only: the black rectangle was cut by the inner one");
    assert_eq!(alpha(&pixmap, 3, 3), 255, "the red square drawn after the inner clip closed");
    assert_eq!(alpha(&pixmap, 40, 40), 0, "inside the inner clip only");
}
