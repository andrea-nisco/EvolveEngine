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
        if pointer_down || self.same_as_committed(scene) {
            return false;
        }
        self.undo.push(std::mem::replace(&mut self.committed, scene.clone()));
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    /// Whether `scene` is what was last committed. The scene's change stamps usually prove it without looking at
    /// the nodes, so a quiet frame costs next to nothing (it used to compare the whole scene on every repaint).
    fn same_as_committed(&self, scene: &Scene) -> bool {
        let quick = scene.same_content(&self.committed);
        // In development builds, check the stamps against the real comparison: a write that skipped the change
        // tracking would otherwise be a missed undo step.
        #[cfg(debug_assertions)]
        {
            let exact = *scene == self.committed;
            if quick && !exact {
                #[cfg(test)]
                panic!("a change to the scene was not recorded in its change stamps (see evolve_engine::revision)");
                #[cfg(not(test))]
                {
                    eprintln!("warning: a change to the scene was not recorded in its change stamps (see evolve_engine::revision)");
                    return exact;
                }
            }
        }
        quick
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

    fn scene_with_shape() -> (Scene, evolve_engine::scene::NodeId) {
        use evolve_engine::path::Path;
        use evolve_engine::scene::Shape;
        let mut scene = Scene::default();
        let node = scene.new_node("box", NodeKind::Shape(Shape::new(Path::rect(Default::default(), evolve_engine::Vec2::splat(10.0)))));
        let id = scene.add(node, None);
        (scene, id)
    }

    #[test]
    fn a_quiet_scene_makes_no_step_and_every_kind_of_edit_makes_one() {
        let (mut scene, id) = scene_with_shape();
        let mut history = History::new(&scene);
        for _ in 0..3 {
            assert!(!history.update(&scene, false), "nothing changed");
        }
        scene.find_mut(id).unwrap().set_transform(evolve_engine::Affine2::from_translation(evolve_engine::Vec2::new(5.0, 0.0)));
        assert!(history.update(&scene, false), "a move");
        *scene.find_mut(id).unwrap().name_mut() = "renamed".into();
        assert!(history.update(&scene, false), "a rename changes nothing that is drawn, and is still an edit");
        scene.find_mut(id).unwrap().set_opacity(0.5);
        assert!(history.update(&scene, false), "opacity");
        scene.find_mut(id).unwrap().opacity = 0.25;
        assert!(history.update(&scene, false), "a plain write through the guard");
        scene.clips.push(evolve_engine::animation::Clip::new("Clip 1"));
        assert!(history.update(&scene, false), "a clip is not in the node tree and is still an edit");
        scene.name = "Other".into();
        assert!(history.update(&scene, false), "the scene's own name");
        assert!(!history.update(&scene, false));
    }

    #[test]
    fn undo_and_redo_give_back_scenes_that_look_unchanged_to_the_history() {
        let (mut scene, id) = scene_with_shape();
        let mut history = History::new(&scene);
        scene.find_mut(id).unwrap().set_opacity(0.5);
        assert!(history.update(&scene, false));
        scene.find_mut(id).unwrap().set_opacity(0.2);
        assert!(history.update(&scene, false));

        assert!(history.undo(&mut scene));
        assert!(!history.update(&scene, false), "right after an undo there is nothing new to record");
        assert!(history.undo(&mut scene));
        assert!(!history.update(&scene, false));
        assert_eq!(scene.find(id).unwrap().opacity, 1.0);
        assert!(history.redo(&mut scene));
        assert!(!history.update(&scene, false));
        assert_eq!(scene.find(id).unwrap().opacity, 0.5);
        // An edit after an undo starts a new line of history and clears what could be redone.
        scene.find_mut(id).unwrap().set_opacity(0.9);
        assert!(history.update(&scene, false));
        assert!(!history.can_redo());
    }

    #[test]
    fn moving_back_to_where_it_was_is_not_a_new_step() {
        let (mut scene, id) = scene_with_shape();
        let mut history = History::new(&scene);
        let start = scene.find(id).unwrap().transform;
        scene.find_mut(id).unwrap().set_transform(evolve_engine::Affine2::from_translation(evolve_engine::Vec2::new(5.0, 0.0)));
        scene.find_mut(id).unwrap().set_transform(start);
        // The stamps say "changed", the real comparison says "equal": no step, as before change tracking.
        assert!(!history.update(&scene, false));
    }

    /// The history trusts the stamps, so a write that skips them must be reported (in development builds, where the
    /// history also compares for real) instead of silently losing an undo step.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "not recorded in its change stamps")]
    fn a_write_that_skips_the_change_stamps_is_reported() {
        let (mut scene, _) = scene_with_shape();
        let mut history = History::new(&scene);
        scene.nodes[0].name = "sneaky".into();
        history.update(&scene, false);
    }
}
