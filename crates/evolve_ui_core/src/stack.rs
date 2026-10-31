//! The screen stack (EVOLVE_UI_PLAN.md 4.1): the screens of a game, in four levels drawn in this order, one over the other.
//!
//! | level | what lives there | shown |
//! |---|---|---|
//! | `Hud` | what the player sees while playing | the top screen |
//! | `Menu` | the menus: a screen pushed over another one covers it, `pop` brings it back | the top screen |
//! | `Modal` | dialogs that stop everything below them | all, bottom to top |
//! | `Debug` | the developer's own panels | all, bottom to top |
//!
//! A screen is a [`UiTree`]. (A screen is made from a UI document by the code that reads documents; the stack only keeps them.)

use crate::draw::DrawList;
use crate::text::Typesetter;
use crate::tree::UiTree;
use crate::units::ScreenMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Hud = 0,
    Menu = 1,
    Modal = 2,
    Debug = 3,
}

impl Level {
    /// In the order they are drawn: the last is on top.
    pub const ALL: [Level; 4] = [Level::Hud, Level::Menu, Level::Modal, Level::Debug];

    /// Whether every screen of the level is shown (the others show only their top one).
    fn shows_all(self) -> bool {
        matches!(self, Level::Modal | Level::Debug)
    }
}

#[derive(Default)]
pub struct ScreenStack {
    levels: [Vec<UiTree>; 4],
    /// Raised when screens come or go, so that the list made from them is made again.
    structure: u64,
    cache: Option<(u64, DrawList)>,
}

impl ScreenStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `tree` on top of the screens of `level`.
    pub fn push(&mut self, level: Level, tree: UiTree) {
        self.levels[level as usize].push(tree);
        self.structure += 1;
    }

    /// Takes the top screen of `level` off the stack and gives it back; the one under it is shown again.
    pub fn pop(&mut self, level: Level) -> Option<UiTree> {
        let tree = self.levels[level as usize].pop();
        if tree.is_some() {
            self.structure += 1;
        }
        tree
    }

    /// Puts `tree` in the place of the top screen of `level` (on an empty level, just on it) and gives the old one back.
    pub fn replace(&mut self, level: Level, tree: UiTree) -> Option<UiTree> {
        let old = self.levels[level as usize].pop();
        self.push(level, tree);
        old
    }

    /// A dialog: a screen on the modal level.
    pub fn show_modal(&mut self, tree: UiTree) {
        self.push(Level::Modal, tree);
    }

    /// Closes the top dialog.
    pub fn close_modal(&mut self) -> Option<UiTree> {
        self.pop(Level::Modal)
    }

    pub fn top_mut(&mut self, level: Level) -> Option<&mut UiTree> {
        self.levels[level as usize].last_mut()
    }

    /// The screens of a level, bottom first.
    pub fn screens_mut(&mut self, level: Level) -> &mut [UiTree] {
        &mut self.levels[level as usize]
    }

    pub fn len(&self, level: Level) -> usize {
        self.levels[level as usize].len()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(Vec::is_empty)
    }

    /// Whether a dialog is open: what is under it must not get the input.
    pub fn blocks_input_below(&self) -> bool {
        !self.levels[Level::Modal as usize].is_empty()
    }

    /// The draw list of every screen that is shown, level over level. Only a screen that changed is drawn again, and when none did
    /// (and none came or went) the list of the last call is returned. The second value says whether the list was made now.
    pub fn draw_list(&mut self, screen: &ScreenMetrics, ts: &mut dyn Typesetter) -> (&DrawList, bool) {
        let mut any_made = false;
        for level in Level::ALL {
            let trees = &mut self.levels[level as usize];
            let from = if level.shows_all() { 0 } else { trees.len().saturating_sub(1) };
            for tree in &mut trees[from..] {
                any_made |= tree.draw_list(screen, ts).1;
            }
        }
        let fresh = self.cache.as_ref().is_some_and(|(structure, _)| *structure == self.structure) && !any_made;
        if !fresh {
            let mut list = DrawList::new();
            for level in Level::ALL {
                let trees = &self.levels[level as usize];
                let from = if level.shows_all() { 0 } else { trees.len().saturating_sub(1) };
                for tree in &trees[from..] {
                    if let Some(part) = tree.last_list() {
                        list.cmds.extend(part.cmds.iter().cloned());
                    }
                }
            }
            self.cache = Some((self.structure, list));
        }
        (&self.cache.as_ref().expect("made above").1, !fresh)
    }
}
