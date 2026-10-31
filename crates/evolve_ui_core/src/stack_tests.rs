use crate::draw::{DrawCmd, Fill};
use crate::geometry::Color;
use crate::stack::{Level, ScreenStack};
use crate::style::Style;
use crate::tree::UiTree;
use crate::tree_tests::{screen, Fixed};

/// A screen that is one filled box of `shade` (the red channel tells screens apart in a draw list).
fn screen_of(shade: u8) -> UiTree {
    let mut tree = UiTree::new(Style::screen());
    tree.add_view(tree.root(), Style { background: Some(Fill::Solid(Color::rgb(shade, 0, 0))), ..Style::default().px(10.0, 10.0) }).unwrap();
    tree
}

fn shades(stack: &mut ScreenStack) -> Vec<u8> {
    let list = stack.draw_list(&screen(), &mut Fixed::default()).0.clone();
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::RoundedRect { fill: Some(Fill::Solid(c)), .. } => Some(c.r),
            _ => None,
        })
        .collect()
}

#[test]
fn levels_are_drawn_in_their_order_whatever_the_order_they_were_filled_in() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Debug, screen_of(4));
    stack.push(Level::Hud, screen_of(1));
    stack.show_modal(screen_of(3));
    stack.push(Level::Menu, screen_of(2));
    assert_eq!(shades(&mut stack), [1, 2, 3, 4], "hud under menu under modal under debug");
}

#[test]
fn a_menu_pushed_over_another_covers_it_and_pop_brings_it_back() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Menu, screen_of(10));
    stack.push(Level::Menu, screen_of(20));
    assert_eq!(shades(&mut stack), [20], "only the top screen of a menu level is shown");
    assert_eq!(stack.len(Level::Menu), 2, "the one below is kept");
    assert!(stack.pop(Level::Menu).is_some());
    assert_eq!(shades(&mut stack), [10]);
    assert!(stack.pop(Level::Menu).is_some());
    assert!(stack.pop(Level::Menu).is_none(), "an empty level has nothing to pop");
    assert!(shades(&mut stack).is_empty());
}

#[test]
fn replace_puts_a_screen_in_the_place_of_the_top_one() {
    let mut stack = ScreenStack::new();
    assert!(stack.replace(Level::Menu, screen_of(1)).is_none(), "on an empty level it is a push");
    stack.push(Level::Menu, screen_of(2));
    let old = stack.replace(Level::Menu, screen_of(3));
    assert!(old.is_some());
    assert_eq!(stack.len(Level::Menu), 2, "the number of screens stays");
    assert_eq!(shades(&mut stack), [3]);
    stack.pop(Level::Menu);
    assert_eq!(shades(&mut stack), [1], "what was under the replaced one is still there");
}

#[test]
fn dialogs_and_debug_panels_all_show_and_a_dialog_blocks_what_is_below() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Hud, screen_of(1));
    assert!(!stack.blocks_input_below());
    stack.show_modal(screen_of(5));
    stack.show_modal(screen_of(6));
    assert!(stack.blocks_input_below());
    assert_eq!(shades(&mut stack), [1, 5, 6], "two dialogs, bottom to top, both shown");
    assert!(stack.close_modal().is_some());
    assert_eq!(shades(&mut stack), [1, 5]);
    stack.close_modal();
    assert!(!stack.blocks_input_below(), "no dialog, no block");
    assert!(stack.close_modal().is_none());
}

#[test]
fn nothing_is_drawn_again_while_nothing_changes() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Hud, screen_of(1));
    stack.push(Level::Menu, screen_of(2));
    let mut ts = Fixed::default();
    assert!(stack.draw_list(&screen(), &mut ts).1, "the first time it is made");
    for _ in 0..4 {
        assert!(!stack.draw_list(&screen(), &mut ts).1, "then it is the same list");
    }
    // A change inside a screen is noticed, a change of the stack is noticed, a screen that is not shown is not asked.
    let tree = stack.top_mut(Level::Menu).unwrap();
    let kid = tree.children(tree.root())[0];
    assert!(tree.set_style(kid, Style { background: Some(Fill::Solid(Color::rgb(9, 0, 0))), ..Style::default().px(10.0, 10.0) }));
    assert!(stack.draw_list(&screen(), &mut ts).1);
    assert_eq!(shades(&mut stack), [1, 9]);
    stack.push(Level::Debug, screen_of(7));
    assert!(stack.draw_list(&screen(), &mut ts).1);
    assert!(!stack.draw_list(&screen(), &mut ts).1);
}

#[test]
fn a_screen_below_the_top_is_not_laid_out_while_it_is_hidden() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Menu, screen_of(1));
    stack.push(Level::Menu, screen_of(2));
    shades(&mut stack);
    let below = stack.screens_mut(Level::Menu)[0].stats();
    let top = stack.screens_mut(Level::Menu)[1].stats();
    assert_eq!(below.layouts, 0, "hidden: no work");
    assert_eq!(top.layouts, 1);
    assert!(!stack.is_empty());
    assert!(ScreenStack::new().is_empty());
}

#[test]
fn two_debug_panels_are_both_shown_but_two_huds_are_not() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Debug, screen_of(8));
    stack.push(Level::Debug, screen_of(9));
    stack.push(Level::Hud, screen_of(1));
    stack.push(Level::Hud, screen_of(2));
    assert_eq!(shades(&mut stack), [2, 8, 9]);
}

#[test]
fn popping_or_pushing_makes_the_list_again_even_if_no_screen_changed() {
    let mut stack = ScreenStack::new();
    stack.push(Level::Menu, screen_of(1));
    stack.push(Level::Menu, screen_of(2));
    let mut ts = Fixed::default();
    assert!(stack.draw_list(&screen(), &mut ts).1);
    assert!(!stack.draw_list(&screen(), &mut ts).1);
    stack.pop(Level::Menu);
    assert!(stack.draw_list(&screen(), &mut ts).1, "the screen under the top is shown now");
    assert_eq!(shades(&mut stack), [1]);
    assert!(stack.pop(Level::Hud).is_none());
    assert!(!stack.draw_list(&screen(), &mut ts).1, "popping nothing is not a change");
}
