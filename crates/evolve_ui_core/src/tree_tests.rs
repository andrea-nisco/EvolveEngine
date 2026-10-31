//! Layout snapshots: the rectangles the tree computes, worked out by hand, for the shapes the plan asks for (stacks, grid, padding,
//! gap, hug and fill, absolute) and for the four screens of its acceptance test.

use crate::draw::{DrawCmd, Fill, FontId, GlyphPos, ImageId, TextRun};
use crate::geometry::{Color, Point, Rect};
use crate::style::{Align, Dim, Direction, Display, Justify, Overflow, Position, Shadow, Sides, Style, Track};
use crate::text::{TextAlign, TextLayout, TextStyle, Typesetter};
use crate::tree::{NodeId, UiTree};
use crate::units::{Insets, MatchMode, ScreenMetrics};

/// A typesetter with metrics that can be worked out in the head: a letter is half the size wide, a line is `line_height` times
/// the size tall, lines break at spaces. It remembers every text it was asked to shape.
#[derive(Default)]
pub(crate) struct Fixed {
    pub seen: Vec<String>,
}

impl Typesetter for Fixed {
    fn layout(&mut self, text: &str, style: &TextStyle, _scale: f32) -> TextLayout {
        self.seen.push(text.to_string());
        let (cw, lh) = (style.size * 0.5, style.size * style.line_height);
        let fits = |chars: usize| style.max_width.is_none_or(|w| chars as f32 * cw <= w + 1e-3);
        let mut lines: Vec<String> = Vec::new();
        if style.wrap && style.max_width.is_some() {
            for word in text.split(' ') {
                match lines.last_mut() {
                    Some(line) if fits(line.chars().count() + 1 + word.chars().count()) => {
                        line.push(' ');
                        line.push_str(word);
                    }
                    _ => lines.push(word.to_string()),
                }
            }
        } else if style.ellipsis && !fits(text.chars().count()) {
            let keep = (style.max_width.unwrap_or(0.0) / cw).floor() as usize;
            lines.push(text.chars().take(keep.saturating_sub(1)).chain(std::iter::once('…')).collect());
        } else {
            lines.push(text.to_string());
        }
        let width = lines.iter().map(|l| l.chars().count() as f32 * cw).fold(0.0, f32::max);
        let mut glyphs = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            let line_width = line.chars().count() as f32 * cw;
            let shift = match (style.align, style.max_width) {
                (TextAlign::Center, Some(w)) => (w - line_width) / 2.0,
                (TextAlign::Right, Some(w)) => w - line_width,
                _ => 0.0,
            };
            for (i, ch) in line.chars().enumerate() {
                glyphs.push(GlyphPos { id: ch as u16, x: shift + i as f32 * cw, y: row as f32 * lh + style.size });
            }
        }
        TextLayout { runs: vec![TextRun { font: FontId(0), size: style.size, color: style.color, glyphs }], width, height: lines.len() as f32 * lh, lines: lines.len() }
    }
}

pub(crate) fn screen() -> ScreenMetrics {
    ScreenMetrics::new((1920.0, 1080.0))
}

/// A tree whose root is the screen's root (a column that fills the canvas) with `style`d container inside; returns both.
fn tree_with(container: Style) -> (UiTree, NodeId) {
    let mut tree = UiTree::new(Style { direction: Direction::Column, ..Style::default() });
    let id = tree.add_view(tree.root(), container).expect("the root exists");
    (tree, id)
}

fn boxed(w: f32, h: f32) -> Style {
    Style::default().px(w, h)
}

fn laid(tree: &mut UiTree, screen: &ScreenMetrics) {
    tree.layout(screen, &mut Fixed::default());
}

fn rect(tree: &UiTree, id: NodeId) -> Rect {
    tree.rect(id).expect("a node of the tree")
}

fn near(r: Rect, x: f32, y: f32, w: f32, h: f32) -> bool {
    [(r.x, x), (r.y, y), (r.w, w), (r.h, h)].iter().all(|(a, b)| (a - b).abs() < 0.01)
}

macro_rules! assert_rect {
    ($tree:expr, $id:expr, $x:expr, $y:expr, $w:expr, $h:expr) => {
        let r = rect(&$tree, $id);
        assert!(near(r, $x, $y, $w, $h), "{} is {:?}, expected ({}, {}, {}, {})", stringify!($id), r, $x, $y, $w, $h);
    };
}

// --- Stacks, padding, gap ---------------------------------------------------------------------------------------------------

#[test]
fn a_row_puts_children_side_by_side_with_gap_and_padding() {
    let (mut tree, row) = tree_with(Style::hstack().gap(10.0).padding(Sides::all(20.0)));
    let kids: Vec<NodeId> = (0..3).map(|_| tree.add_view(row, boxed(100.0, 50.0)).unwrap()).collect();
    laid(&mut tree, &screen());
    // 20 + 3 x 100 + 2 x 10 + 20 wide, 20 + 50 + 20 tall: the container hugs its children.
    assert_rect!(tree, row, 0.0, 0.0, 360.0, 90.0);
    assert_rect!(tree, kids[0], 20.0, 20.0, 100.0, 50.0);
    assert_rect!(tree, kids[1], 130.0, 20.0, 100.0, 50.0);
    assert_rect!(tree, kids[2], 240.0, 20.0, 100.0, 50.0);
}

#[test]
fn a_column_stacks_down_and_aligns_across() {
    for (align, x0, x1) in [(Align::Start, 0.0, 0.0), (Align::Center, 100.0, 50.0), (Align::End, 200.0, 100.0)] {
        let (mut tree, column) = tree_with(Style::vstack().gap(8.0).align(align).size(Dim::Px(300.0), Dim::Hug));
        let a = tree.add_view(column, boxed(100.0, 40.0)).unwrap();
        let b = tree.add_view(column, boxed(200.0, 40.0)).unwrap();
        laid(&mut tree, &screen());
        assert_rect!(tree, a, x0, 0.0, 100.0, 40.0);
        assert_rect!(tree, b, x1, 48.0, 200.0, 40.0);
        assert_rect!(tree, column, 0.0, 0.0, 300.0, 88.0);
    }
}

#[test]
fn justify_distributes_the_free_space_along_the_row() {
    let cases = [
        (Justify::Start, [0.0, 50.0, 100.0]),
        (Justify::Center, [125.0, 175.0, 225.0]),
        (Justify::End, [250.0, 300.0, 350.0]),
        (Justify::SpaceBetween, [0.0, 175.0, 350.0]),
        (Justify::SpaceEvenly, [62.5, 175.0, 287.5]),
    ];
    for (justify, xs) in cases {
        let (mut tree, row) = tree_with(Style::hstack().justify(justify).size(Dim::Px(400.0), Dim::Px(50.0)));
        let kids: Vec<NodeId> = (0..3).map(|_| tree.add_view(row, boxed(50.0, 50.0)).unwrap()).collect();
        laid(&mut tree, &screen());
        for (kid, x) in kids.iter().zip(xs) {
            assert_rect!(tree, *kid, x, 0.0, 50.0, 50.0);
        }
    }
}

#[test]
fn a_row_can_run_backwards_and_wrap() {
    let (mut tree, row) = tree_with(Style { direction: Direction::RowReverse, ..boxed(300.0, 50.0) });
    let (a, b) = (tree.add_view(row, boxed(100.0, 50.0)).unwrap(), tree.add_view(row, boxed(100.0, 50.0)).unwrap());
    laid(&mut tree, &screen());
    assert_rect!(tree, a, 200.0, 0.0, 100.0, 50.0);
    assert_rect!(tree, b, 100.0, 0.0, 100.0, 50.0);

    let (mut tree, row) = tree_with(Style { wrap: true, gap: (10.0, 10.0), ..boxed(250.0, 200.0) });
    let kids: Vec<NodeId> = (0..3).map(|_| tree.add_view(row, boxed(100.0, 40.0)).unwrap()).collect();
    laid(&mut tree, &screen());
    assert_rect!(tree, kids[1], 110.0, 0.0, 100.0, 40.0);
    assert_rect!(tree, kids[2], 0.0, 50.0, 100.0, 40.0);
}

// --- Hug, fill, fixed, percent -----------------------------------------------------------------------------------------------

#[test]
fn fill_shares_what_the_fixed_sizes_leave() {
    let (mut tree, row) = tree_with(Style::hstack().size(Dim::Px(400.0), Dim::Px(100.0)));
    let fixed = tree.add_view(row, boxed(100.0, 20.0)).unwrap();
    let a = tree.add_view(row, Style::default().size(Dim::Fill, Dim::Fill)).unwrap();
    let b = tree.add_view(row, Style::default().size(Dim::Fill, Dim::Px(30.0))).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, fixed, 0.0, 0.0, 100.0, 20.0);
    assert_rect!(tree, a, 100.0, 0.0, 150.0, 100.0); // along the row a share, across it the whole height
    assert_rect!(tree, b, 250.0, 0.0, 150.0, 30.0);
}

#[test]
fn fill_follows_the_direction_of_its_container() {
    let (mut tree, column) = tree_with(Style::vstack().size(Dim::Px(200.0), Dim::Px(300.0)));
    let top = tree.add_view(column, boxed(50.0, 100.0)).unwrap();
    let rest = tree.add_view(column, Style::default().size(Dim::Fill, Dim::Fill)).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, top, 0.0, 0.0, 50.0, 100.0);
    assert_rect!(tree, rest, 0.0, 100.0, 200.0, 200.0);
}

#[test]
fn grow_weighs_the_shares_of_fill() {
    let (mut tree, row) = tree_with(Style::hstack().size(Dim::Px(400.0), Dim::Px(50.0)));
    tree.add_view(row, boxed(100.0, 50.0)).unwrap();
    let one = tree.add_view(row, Style { grow: 1.0, ..Style::default().size(Dim::Fill, Dim::Px(50.0)) }).unwrap();
    let three = tree.add_view(row, Style { grow: 3.0, ..Style::default().size(Dim::Fill, Dim::Px(50.0)) }).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, one, 100.0, 0.0, 75.0, 50.0);
    assert_rect!(tree, three, 175.0, 0.0, 225.0, 50.0);
}

#[test]
fn percent_is_a_share_of_the_container_and_limits_cut_a_size() {
    let (mut tree, container) = tree_with(boxed(400.0, 200.0));
    let half = tree.add_view(container, Style::default().size(Dim::Percent(50.0), Dim::Percent(25.0))).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, half, 0.0, 0.0, 200.0, 50.0);

    let (mut tree, row) = tree_with(Style::hstack().size(Dim::Px(400.0), Dim::Px(50.0)));
    let capped = tree.add_view(row, Style { max_width: Some(120.0), ..Style::default().size(Dim::Fill, Dim::Px(50.0)) }).unwrap();
    let floor = tree.add_view(row, Style { min_width: Some(30.0), ..boxed(10.0, 50.0) }).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, capped, 0.0, 0.0, 120.0, 50.0);
    assert_rect!(tree, floor, 120.0, 0.0, 30.0, 50.0);
}

#[test]
fn margin_and_aspect_ratio() {
    let (mut tree, container) = tree_with(Style::hstack().size(Dim::Px(300.0), Dim::Px(200.0)));
    let kid = tree.add_view(container, Style { margin: Sides::new(10.0, 0.0, 0.0, 30.0), ..boxed(50.0, 50.0) }).unwrap();
    let wide = tree.add_view(container, Style { aspect_ratio: Some(2.0), ..Style::default().size(Dim::Px(100.0), Dim::Hug) }).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, kid, 30.0, 10.0, 50.0, 50.0);
    assert_rect!(tree, wide, 80.0, 0.0, 100.0, 50.0);
}

// --- Grid -------------------------------------------------------------------------------------------------------------------

#[test]
fn a_grid_gives_its_tracks_their_sizes() {
    let (mut tree, grid) = tree_with(Style { gap: (10.0, 10.0), ..Style::grid(vec![Track::Px(100.0), Track::Fr(1.0), Track::Fr(2.0)]).size(Dim::Px(410.0), Dim::Hug) });
    let cells: Vec<NodeId> = (0..6).map(|_| tree.add_view(grid, Style::default().size(Dim::Fill, Dim::Px(30.0))).unwrap()).collect();
    laid(&mut tree, &screen());
    // (410 - 100 - 2 x 10) is 290: a third and two thirds of it for the two flexible columns.
    let (fr1, fr2) = (290.0 / 3.0, 290.0 * 2.0 / 3.0);
    assert_rect!(tree, cells[0], 0.0, 0.0, 100.0, 30.0);
    assert_rect!(tree, cells[1], 110.0, 0.0, fr1, 30.0);
    assert_rect!(tree, cells[2], 110.0 + fr1 + 10.0, 0.0, fr2, 30.0);
    assert_rect!(tree, cells[3], 0.0, 40.0, 100.0, 30.0);
    assert_rect!(tree, grid, 0.0, 0.0, 410.0, 70.0);
}

#[test]
fn a_grid_item_can_span_columns() {
    let (mut tree, grid) = tree_with(Style::grid(vec![Track::Px(100.0), Track::Px(100.0)]).size(Dim::Px(200.0), Dim::Hug));
    let wide = tree.add_view(grid, Style { span: (2, 1), ..Style::default().size(Dim::Fill, Dim::Px(20.0)) }).unwrap();
    let next = tree.add_view(grid, Style::default().size(Dim::Fill, Dim::Px(20.0))).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, wide, 0.0, 0.0, 200.0, 20.0);
    assert_rect!(tree, next, 0.0, 20.0, 100.0, 20.0);
}

// --- Absolute, display none -------------------------------------------------------------------------------------------------

#[test]
fn an_absolute_node_is_placed_from_the_edges_and_takes_no_room() {
    let (mut tree, container) = tree_with(boxed(300.0, 200.0));
    let flow = tree.add_view(container, boxed(100.0, 40.0)).unwrap();
    let badge = tree.add_view(container, Style { position: Position::Absolute, inset: (Some(20.0), Some(10.0), None, None), ..boxed(50.0, 50.0) }).unwrap();
    let corner = tree.add_view(container, Style { position: Position::Absolute, inset: (None, Some(0.0), Some(0.0), None), ..boxed(30.0, 30.0) }).unwrap();
    let after = tree.add_view(container, boxed(100.0, 40.0)).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, flow, 0.0, 0.0, 100.0, 40.0);
    assert_rect!(tree, badge, 240.0, 20.0, 50.0, 50.0);
    assert_rect!(tree, corner, 270.0, 170.0, 30.0, 30.0);
    assert_rect!(tree, after, 100.0, 0.0, 100.0, 40.0); // in the row, right after `flow`: the absolute ones took no room
}

#[test]
fn a_node_with_display_none_takes_no_room_and_draws_nothing() {
    let (mut tree, row) = tree_with(Style::hstack());
    let a = tree.add_view(row, boxed(50.0, 50.0)).unwrap();
    let hidden = tree.add_view(row, Style { display: Display::None, background: Some(Fill::Solid(Color::BLACK)), ..boxed(80.0, 50.0) }).unwrap();
    let b = tree.add_view(row, boxed(50.0, 50.0)).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, b, 50.0, 0.0, 50.0, 50.0);
    assert_eq!((rect(&tree, hidden).w, rect(&tree, hidden).h), (0.0, 0.0));
    let _ = a;
    let (list, _) = tree.draw_list(&screen(), &mut Fixed::default());
    assert!(list.cmds.is_empty(), "{:?}", list.cmds);
}

// --- Text -------------------------------------------------------------------------------------------------------------------

fn text_style(size: f32) -> TextStyle {
    TextStyle { size, ..TextStyle::default() }
}

#[test]
fn a_text_is_as_large_as_its_words_and_wraps_in_the_width_it_gets() {
    let (mut tree, row) = tree_with(Style::hstack().padding(Sides::all(5.0)));
    let hello = tree.add_text(row, "Hello", text_style(20.0), Style::default()).unwrap();
    laid(&mut tree, &screen());
    // 5 letters of 10, a line of 26; the container adds its padding.
    assert_rect!(tree, hello, 5.0, 5.0, 50.0, 26.0);
    assert_rect!(tree, row, 0.0, 0.0, 60.0, 36.0);

    let (mut tree, column) = tree_with(Style::vstack().size(Dim::Px(100.0), Dim::Hug));
    let long = tree.add_text(column, "aaaa bbbb cccc", TextStyle { wrap: true, ..text_style(20.0) }, Style::default()).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, long, 0.0, 0.0, 90.0, 52.0); // "aaaa bbbb" (90) and "cccc": two lines
    assert_rect!(tree, column, 0.0, 0.0, 100.0, 52.0);
}

#[test]
fn a_filled_text_wraps_at_the_width_of_its_container_and_cuts_with_an_ellipsis() {
    let (mut tree, column) = tree_with(Style::vstack().size(Dim::Px(150.0), Dim::Hug));
    let long = tree.add_text(column, "aaaa bbbb cccc dddd", TextStyle { wrap: true, ..text_style(20.0) }, Style::default().size(Dim::Fill, Dim::Hug)).unwrap();
    let cut = tree.add_text(column, "a very long title", TextStyle { ellipsis: true, ..text_style(20.0) }, Style::default().size(Dim::Fill, Dim::Hug)).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, long, 0.0, 0.0, 150.0, 52.0); // "aaaa bbbb cccc" is 140 wide: two lines
    assert_rect!(tree, cut, 0.0, 52.0, 150.0, 26.0);
    let (list, _) = tree.draw_list(&screen(), &mut Fixed::default());
    let last_text = list.cmds.iter().rev().find_map(|c| match c {
        DrawCmd::Text { run, .. } => Some(run.glyphs.iter().map(|g| char::from_u32(u32::from(g.id)).unwrap()).collect::<String>()),
        _ => None,
    });
    assert_eq!(last_text.as_deref(), Some("a very long ti…"), "cut to 15 letters of 10, the last of them the ellipsis");
}

#[test]
fn text_is_drawn_inside_the_padding_and_the_border_of_its_node() {
    let (mut tree, column) = tree_with(Style::vstack());
    tree.add_text(column, "Hi", text_style(20.0), Style { padding: Sides::xy(8.0, 4.0), border: Some((2.0, Color::BLACK)), ..Style::default() }).unwrap();
    let (list, _) = tree.draw_list(&screen(), &mut Fixed::default());
    let origin = list.cmds.iter().find_map(|c| if let DrawCmd::Text { origin, .. } = c { Some(*origin) } else { None }).expect("a text");
    assert_eq!(origin, Point::new(10.0, 6.0), "2 of border and 8 across, 2 and 4 down");
    let size = tree.rect(tree.children(column)[0]).unwrap();
    assert!((size.w - (20.0 + 16.0 + 4.0)).abs() < 0.01 && (size.h - (26.0 + 8.0 + 4.0)).abs() < 0.01, "{size:?}");
}

// --- Images -----------------------------------------------------------------------------------------------------------------

#[test]
fn an_image_keeps_its_proportions() {
    let (mut tree, row) = tree_with(Style::hstack().size(Dim::Px(400.0), Dim::Px(200.0)));
    let natural = tree.add_image(row, ImageId(0), (24.0, 12.0), None, Style::default()).unwrap();
    let by_width = tree.add_image(row, ImageId(0), (24.0, 12.0), None, Style::default().size(Dim::Px(100.0), Dim::Hug)).unwrap();
    let by_height = tree.add_image(row, ImageId(0), (24.0, 12.0), Some(Color::WHITE), Style::default().size(Dim::Hug, Dim::Px(30.0))).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, natural, 0.0, 0.0, 24.0, 12.0);
    assert_rect!(tree, by_width, 24.0, 0.0, 100.0, 50.0);
    assert_rect!(tree, by_height, 124.0, 0.0, 60.0, 30.0);
    let (list, _) = tree.draw_list(&screen(), &mut Fixed::default());
    assert!(list.cmds.iter().any(|c| matches!(c, DrawCmd::Image { tint: Some(_), .. })), "the tint travels to the draw list");
}

// --- Screens ----------------------------------------------------------------------------------------------------------------

/// The same small UI on the four screens of the plan: a header with a title and a button at each end, a body that fills the rest.
fn menu(tree: &mut UiTree) -> (NodeId, NodeId, NodeId) {
    let root = tree.root();
    let header = tree.add_view(root, Style::hstack().size(Dim::Fill, Dim::Px(100.0)).justify(Justify::SpaceBetween)).unwrap();
    let title = tree.add_view(header, boxed(400.0, 100.0)).unwrap();
    let button = tree.add_view(header, boxed(200.0, 100.0)).unwrap();
    let body = tree.add_view(root, Style::default().size(Dim::Fill, Dim::Fill)).unwrap();
    let _ = title;
    (button, body, header)
}

#[test]
fn the_same_ui_fits_four_screens() {
    let screens = [(1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1080.0), (1080.0, 2400.0)];
    for pixels in screens {
        let metrics = ScreenMetrics::new(pixels);
        let canvas = metrics.canvas();
        let mut tree = UiTree::new(Style::screen());
        let (button, body, header) = menu(&mut tree);
        laid(&mut tree, &metrics);
        assert_rect!(tree, tree.root(), 0.0, 0.0, canvas.0, canvas.1);
        assert_rect!(tree, header, 0.0, 0.0, canvas.0, 100.0);
        assert_rect!(tree, button, canvas.0 - 200.0, 0.0, 200.0, 100.0); // at the right edge, whatever the screen
        assert_rect!(tree, body, 0.0, 100.0, canvas.0, canvas.1 - 100.0);
        assert!(canvas.0 >= 1920.0 - 0.01 && canvas.1 >= 1080.0 - 0.01, "{pixels:?}: the whole reference screen is in the canvas");
    }
}

#[test]
fn match_modes_change_the_canvas_not_the_tree() {
    let mut tree = UiTree::new(Style::screen());
    let (_, body, _) = menu(&mut tree);
    let wide = ScreenMetrics::new((2560.0, 1080.0));
    laid(&mut tree, &wide.with_mode(MatchMode::Min));
    assert_rect!(tree, body, 0.0, 100.0, 2560.0, 980.0);
    laid(&mut tree, &wide.with_mode(MatchMode::Width));
    assert_rect!(tree, body, 0.0, 100.0, 1920.0, 1080.0 / (2560.0 / 1920.0) - 100.0);
}

#[test]
fn the_safe_area_moves_the_content_of_a_screen_and_only_of_a_screen() {
    let phone = ScreenMetrics::new((1080.0, 1920.0)).with_safe_area(Insets::new(0.0, 60.0, 0.0, 40.0));
    let scale = phone.scale();
    let (top, bottom) = (60.0 / scale, 40.0 / scale);
    let mut tree = UiTree::new(Style::screen());
    let (_, body, _) = menu(&mut tree);
    laid(&mut tree, &phone);
    assert_rect!(tree, tree.root(), 0.0, 0.0, phone.canvas().0, phone.canvas().1);
    let first = tree.children(tree.root())[0];
    assert!((rect(&tree, first).y - top).abs() < 0.01, "the header starts below the notch");
    assert!((rect(&tree, body).bottom() - (phone.canvas().1 - bottom)).abs() < 0.01, "the body ends above the bar");

    // A root that does not ask for it ignores the safe area, as `safe_area: false` says.
    let mut tree = UiTree::new(Style { safe_area: false, ..Style::screen() });
    let (_, body, _) = menu(&mut tree);
    laid(&mut tree, &phone);
    assert!((rect(&tree, body).bottom() - phone.canvas().1).abs() < 0.01);
}

// --- What is laid out again -------------------------------------------------------------------------------------------------

/// Two columns, each with a text; changing one must not measure the other.
fn two_texts() -> (UiTree, NodeId, NodeId) {
    let mut tree = UiTree::new(Style::screen());
    let row = tree.add_view(tree.root(), Style::hstack().gap(10.0)).unwrap();
    let left = tree.add_view(row, Style::vstack()).unwrap();
    let right = tree.add_view(row, Style::vstack()).unwrap();
    let a = tree.add_text(left, "left text", text_style(20.0), Style::default()).unwrap();
    let b = tree.add_text(right, "right text", text_style(20.0), Style::default()).unwrap();
    (tree, a, b)
}

#[test]
fn a_tree_that_did_not_change_is_not_laid_out_or_drawn_again() {
    let (mut tree, _, _) = two_texts();
    let mut ts = Fixed::default();
    assert!(tree.layout(&screen(), &mut ts));
    let after_first = (tree.stats(), ts.seen.len());
    assert_eq!(after_first.0.layouts, 1);
    for _ in 0..5 {
        assert!(!tree.layout(&screen(), &mut ts));
    }
    assert_eq!((tree.stats(), ts.seen.len()), after_first, "five more frames, no layout and no text shaped");

    let (_, made) = tree.draw_list(&screen(), &mut ts);
    assert!(made);
    let shaped = ts.seen.len();
    for _ in 0..5 {
        assert!(!tree.draw_list(&screen(), &mut ts).1, "the draw list of the last call is kept");
    }
    assert_eq!(ts.seen.len(), shaped);
    assert_eq!(tree.stats().paints, 1);
}

#[test]
fn changing_one_text_measures_only_that_text() {
    let (mut tree, a, _) = two_texts();
    let mut ts = Fixed::default();
    tree.layout(&screen(), &mut ts);
    ts.seen.clear();
    assert!(tree.set_text(a, "a new and longer text"));
    assert!(tree.layout(&screen(), &mut ts));
    assert!(ts.seen.iter().all(|t| t == "a new and longer text"), "only the changed text was measured: {:?}", ts.seen);
    assert!(!ts.seen.is_empty());
    let r = rect(&tree, a);
    assert!((r.w - 21.0 * 10.0).abs() < 0.01, "and its box follows: {r:?}");
}

#[test]
fn setting_what_is_already_there_changes_nothing() {
    let (mut tree, a, _) = two_texts();
    let mut ts = Fixed::default();
    tree.draw_list(&screen(), &mut ts);
    let style = tree.style(a).unwrap().clone();
    assert!(tree.set_text(a, "left text"));
    assert!(tree.set_style(a, style));
    assert!(!tree.layout(&screen(), &mut ts), "the same text and the same style are not a change");
    assert!(!tree.draw_list(&screen(), &mut ts).1);
}

#[test]
fn another_scale_shapes_the_texts_again_another_canvas_lays_out_again() {
    let (mut tree, _, _) = two_texts();
    let mut ts = Fixed::default();
    tree.layout(&screen(), &mut ts);
    ts.seen.clear();
    let sharper = ScreenMetrics::new((3840.0, 2160.0)); // the same canvas at twice the pixels
    assert!(tree.layout(&sharper, &mut ts));
    assert!(ts.seen.contains(&"left text".to_string()) && ts.seen.contains(&"right text".to_string()), "snapping depends on the scale: {:?}", ts.seen);
    ts.seen.clear();
    assert!(!tree.layout(&sharper, &mut ts), "and then it is quiet again");
    assert!(tree.layout(&ScreenMetrics::new((3000.0, 2160.0)), &mut ts), "another canvas is another layout");
}

#[test]
fn removing_a_node_removes_what_is_in_it() {
    let (mut tree, _, _) = two_texts();
    let row = tree.children(tree.root())[0];
    let left = tree.children(row)[0];
    let text = tree.children(left)[0];
    let count = tree.len();
    assert!(tree.remove(left));
    assert_eq!(tree.len(), count - 2, "the column and the text in it");
    assert!(!tree.contains(left) && !tree.contains(text) && tree.rect(text).is_none());
    assert!(!tree.remove(tree.root()), "the root stays");
    assert!(!tree.remove(left), "a node that is gone cannot be removed twice");
    assert!(tree.add_view(left, Style::default()).is_none(), "nothing can be added to it");
    laid(&mut tree, &screen());
    assert_eq!(tree.children(row).len(), 1);
}

#[test]
fn changing_the_direction_of_a_container_changes_what_fill_means_in_it() {
    let (mut tree, container) = tree_with(Style::hstack().size(Dim::Px(300.0), Dim::Px(100.0)));
    let kid = tree.add_view(container, Style::default().size(Dim::Fill, Dim::Px(20.0))).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, kid, 0.0, 0.0, 300.0, 20.0);
    tree.set_style(container, Style::vstack().size(Dim::Px(300.0), Dim::Px(100.0)));
    laid(&mut tree, &screen());
    assert_rect!(tree, kid, 0.0, 0.0, 300.0, 20.0); // now across the column: the whole width, still
    let tall = tree.add_view(container, Style::default().size(Dim::Px(10.0), Dim::Fill)).unwrap();
    laid(&mut tree, &screen());
    assert_rect!(tree, tall, 0.0, 20.0, 10.0, 80.0);
}

// --- Drawing ----------------------------------------------------------------------------------------------------------------

fn commands(tree: &mut UiTree) -> Vec<DrawCmd> {
    tree.draw_list(&screen(), &mut Fixed::default()).0.cmds.clone()
}

#[test]
fn a_node_draws_its_shadow_then_its_box_then_what_is_in_it() {
    let (mut tree, container) = tree_with(Style {
        background: Some(Fill::Solid(Color::WHITE)),
        border: Some((1.0, Color::BLACK)),
        radii: [8.0; 4],
        shadows: vec![Shadow { offset: Point::new(0.0, 4.0), blur: 12.0, spread: 0.0, color: Color::rgba(0, 0, 0, 80) }],
        padding: Sides::all(10.0),
        ..Style::default()
    });
    tree.add_text(container, "Hi", text_style(20.0), Style::default()).unwrap();
    let cmds = commands(&mut tree);
    assert!(matches!(cmds[0], DrawCmd::Shadow { blur, .. } if blur == 12.0));
    assert!(matches!(&cmds[1], DrawCmd::RoundedRect { radii, border: Some((w, _)), fill: Some(_), .. } if *radii == [8.0; 4] && *w == 1.0));
    assert!(matches!(cmds[2], DrawCmd::Text { .. }));
    assert_eq!(cmds.len(), 3);
    let DrawCmd::Shadow { rect, .. } = cmds[0] else { unreachable!() };
    assert_eq!(rect, Rect::new(0.0, 0.0, 2.0 + 20.0 + 20.0 + 0.0 + 0.0, 2.0 + 20.0 + 26.0).clone_with_text_width());
}

trait CloneWithTextWidth {
    fn clone_with_text_width(self) -> Rect;
}

impl CloneWithTextWidth for Rect {
    /// The box of the test above: border 1, padding 10, a text of two letters (20 wide) and a line (26 tall).
    fn clone_with_text_width(self) -> Rect {
        Rect::new(0.0, 0.0, 2.0 + 20.0 + 20.0, 2.0 + 20.0 + 26.0)
    }
}

#[test]
fn a_gradient_given_in_fractions_is_placed_on_the_box_of_its_node() {
    let (mut tree, column) = tree_with(Style::vstack().padding(Sides::all(10.0)));
    let fill = Fill::LinearGradient { start: Point::new(0.0, 0.0), end: Point::new(0.0, 1.0), stops: vec![(0.0, Color::WHITE), (1.0, Color::BLACK)] };
    tree.add_view(column, Style { background: Some(fill), ..boxed(100.0, 50.0) }).unwrap();
    let cmds = commands(&mut tree);
    let DrawCmd::RoundedRect { fill: Some(Fill::LinearGradient { start, end, .. }), rect, .. } = &cmds[0] else { panic!("{cmds:?}") };
    assert_eq!((rect.y, start.y, end.y), (10.0, 10.0, 60.0), "from the top of the box to its bottom");
}

#[test]
fn clipped_nodes_bracket_their_children_and_z_index_orders_siblings() {
    let (mut tree, container) = tree_with(Style { overflow: Overflow::Clip, radii: [6.0; 4], ..boxed(100.0, 100.0) });
    let low = Style { background: Some(Fill::Solid(Color::rgb(1, 0, 0))), z_index: 1, ..boxed(10.0, 10.0) };
    let high = Style { background: Some(Fill::Solid(Color::rgb(2, 0, 0))), z_index: 5, ..boxed(10.0, 10.0) };
    let plain = Style { background: Some(Fill::Solid(Color::rgb(3, 0, 0))), ..boxed(10.0, 10.0) };
    tree.add_view(container, high).unwrap();
    tree.add_view(container, low).unwrap();
    tree.add_view(container, plain).unwrap();
    let cmds = commands(&mut tree);
    assert!(matches!(cmds[0], DrawCmd::PushClip { radii, .. } if radii == [6.0; 4]) && matches!(cmds.last(), Some(DrawCmd::PopClip)));
    let reds: Vec<u8> = cmds.iter().filter_map(|c| if let DrawCmd::RoundedRect { fill: Some(Fill::Solid(c)), .. } = c { Some(c.r) } else { None }).collect();
    assert_eq!(reds, [3, 1, 2], "z 0, then 1, then 5: the highest is drawn last, on top");
    let mut list = crate::draw::DrawList::new();
    list.cmds = cmds;
    assert!(list.is_balanced());
}

#[test]
fn boxes_are_drawn_on_whole_physical_pixels() {
    // At scale 2, a box at x = 0.3 wide 10.3 would blur its edges; both edges move to the nearest half logical pixel.
    let metrics = ScreenMetrics::new((3840.0, 2160.0));
    let mut tree = UiTree::new(Style::screen());
    tree.add_view(tree.root(), Style { margin: Sides::new(0.3, 0.0, 0.0, 0.3), background: Some(Fill::Solid(Color::BLACK)), ..boxed(10.3, 10.3) }).unwrap();
    let list = tree.draw_list(&metrics, &mut Fixed::default()).0.clone();
    let DrawCmd::RoundedRect { rect, .. } = &list.cmds[0] else { panic!("{:?}", list.cmds) };
    for v in [rect.x, rect.y, rect.right(), rect.bottom()] {
        assert!(((v * 2.0) - (v * 2.0).round()).abs() < 1e-4, "{v} is not on a physical pixel");
    }
}

#[test]
fn nonsense_numbers_in_a_style_do_not_break_the_layout() {
    let (mut tree, container) = tree_with(Style {
        width: Dim::Px(f32::NAN),
        height: Dim::Percent(f32::INFINITY),
        padding: Sides::all(f32::NAN),
        gap: (f32::NAN, -5.0),
        grow: f32::NAN,
        shrink: -1.0,
        min_width: Some(f32::NAN),
        aspect_ratio: Some(-1.0),
        border: Some((f32::NAN, Color::BLACK)),
        ..Style::default()
    });
    let kid = tree.add_view(container, boxed(10.0, 10.0)).unwrap();
    laid(&mut tree, &screen());
    let r = rect(&tree, kid);
    assert!(r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite(), "{r:?}");
    let list = commands(&mut tree);
    assert!(list.iter().all(|c| !matches!(c, DrawCmd::Text { .. })));
}

#[test]
fn nodes_can_be_asked_about_and_text_can_be_read_back() {
    let (mut tree, a, _) = two_texts();
    assert_eq!(tree.text(a), Some("left text"));
    assert_eq!(tree.text(tree.root()), None);
    let parent = tree.parent(a).unwrap();
    assert!(tree.children(parent).contains(&a));
    assert_eq!(tree.parent(tree.root()), None);
    assert!(!tree.set_text(tree.root(), "no text here"), "only a text node has words");
    assert!(tree.set_text_style(a, TextStyle { size: 40.0, ..TextStyle::default() }));
    laid(&mut tree, &screen());
    assert!((rect(&tree, a).h - 52.0).abs() < 0.01, "a larger text is a taller box");
    assert!(!tree.is_empty() && tree.len() >= 6);
}

#[test]
fn fill_siblings_share_equally_whatever_their_contents_are() {
    let (mut tree, row) = tree_with(Style::hstack().size(Dim::Px(400.0), Dim::Px(50.0)));
    let a = tree.add_view(row, Style::default().size(Dim::Fill, Dim::Px(50.0))).unwrap();
    let b = tree.add_view(row, Style::default().size(Dim::Fill, Dim::Px(50.0))).unwrap();
    tree.add_view(a, boxed(100.0, 10.0)).unwrap();
    tree.add_view(b, boxed(20.0, 10.0)).unwrap();
    laid(&mut tree, &screen());
    // Not 240 and 160 (what the contents would ask for, plus half of the rest each): the share is the same.
    assert_rect!(tree, a, 0.0, 0.0, 200.0, 50.0);
    assert_rect!(tree, b, 200.0, 0.0, 200.0, 50.0);
}

#[test]
fn a_text_box_is_rounded_up_to_the_pixels_of_the_screen() {
    // "abc" at 15 is 3 letters of 7.5: 22.5 wide. On an ordinary screen a pixel is whole, so the box takes 23; on a screen of twice
    // the pixels 22.5 is whole already.
    let measured = |pixels: (f32, f32)| {
        let (mut tree, row) = tree_with(Style::hstack());
        let text = tree.add_text(row, "abc", text_style(15.0), Style::default()).unwrap();
        tree.layout(&ScreenMetrics::new(pixels), &mut Fixed::default());
        rect(&tree, text).w
    };
    assert!((measured((1920.0, 1080.0)) - 23.0).abs() < 1e-4);
    assert!((measured((3840.0, 2160.0)) - 22.5).abs() < 1e-4);
}
