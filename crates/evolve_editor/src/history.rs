//! Undo / redo by scene snapshots.
//!
//! Instead of asking every edit to record itself, the history watches the scene:
//! whenever it differs from the last committed state and no pointer button is held
//! (so a whole drag counts as one edit), the old state becomes an undo step.

use evolve_engine::scene::Scene;

const MAX_STEPS: usize = 100;

pub struct History {
    committed: Scene,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
}

impl History {
    pub fn new(scene: &Scene) -> Self {
        Self { committed: scene.clone(), undo: Vec::new(), redo: Vec::new() }
    }

    /// Forget all history, e.g. after opening another document.
    pub fn reset(&mut self, scene: &Scene) {
        *self = Self::new(scene);
    }

    /// Records an undo step if the scene changed. Returns true when it did.
    pub fn update(&mut self, scene: &Scene, pointer_down: bool) -> bool {
        if pointer_down || *scene == self.committed {
            return false;
        }
        self.undo.push(std::mem::replace(&mut self.committed, scene.clone()));
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    pub fn undo(&mut self, scene: &mut Scene) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.committed, previous.clone()));
        *scene = previous;
        true
    }

    pub fn redo(&mut self, scene: &mut Scene) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.committed, next.clone()));
        *scene = next;
        true
    }

    /// The scene as of the last recorded step.
    pub fn committed(&self) -> &Scene {
        &self.committed
    }

    /// Changes the committed state without creating an undo step. Used for edits that are not
    /// user actions, like showing the animation pose of another time.
    pub fn edit_committed(&mut self, edit: impl FnOnce(&mut Scene)) {
        edit(&mut self.committed);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::scene::NodeKind;

    #[test]
    fn undo_and_redo_restore_states() {
        let mut scene = Scene::default();
        let mut history = History::new(&scene);

        let node = scene.new_node("box", NodeKind::Group(Vec::new()));
        scene.add(node, None);
        assert!(!history.update(&scene, true), "no step while the pointer is down");
        assert!(history.update(&scene, false));

        assert!(history.undo(&mut scene));
        assert!(scene.nodes.is_empty());
        assert!(history.redo(&mut scene));
        assert_eq!(scene.nodes.len(), 1);
        assert!(!history.redo(&mut scene));
    }
}
