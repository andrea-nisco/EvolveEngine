//! A project: an ordered list of scenes and the one the game starts with.
//!
//! The order is the game's sequence: `next_scene()` in a script goes to the scene below the
//! current one. Scenes are found by name (`load_scene("Level 2")`), so names are unique.

use crate::game_camera::DEFAULT_SIZE;
use crate::physics::PhysicsSettings;
use crate::scene::Scene;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// Never empty.
    pub scenes: Vec<Scene>,
    /// Index of the scene the game starts with.
    pub start: usize,
    /// Physics options for every scene of the game.
    pub physics: PhysicsSettings,
}

impl Project {
    /// A project with one empty scene that has a camera.
    pub fn new() -> Self {
        Self::single(Scene::with_camera(DEFAULT_SIZE))
    }

    pub fn single(scene: Scene) -> Self {
        Self { scenes: vec![scene], start: 0, physics: PhysicsSettings::default() }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.scenes.iter().position(|s| s.name == name)
    }

    /// `wanted`, or `wanted 2`, `wanted 3`... if another scene (not `except`) already has that name.
    pub fn unique_name(&self, wanted: &str, except: Option<usize>) -> String {
        let wanted = wanted.trim();
        let wanted = if wanted.is_empty() { "Scene" } else { wanted };
        let taken = |name: &str| self.scenes.iter().enumerate().any(|(i, s)| Some(i) != except && s.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        (2..).map(|n| format!("{wanted} {n}")).find(|name| !taken(name)).expect("an unused number exists")
    }

    /// Adds an empty scene with a camera at the end of the list. Returns its index.
    pub fn add_scene(&mut self) -> usize {
        let mut scene = Scene::with_camera(DEFAULT_SIZE);
        scene.name = self.unique_name(&format!("Scene {}", self.scenes.len() + 1), None);
        self.scenes.push(scene);
        self.scenes.len() - 1
    }

    /// Copies scene `index` right below itself. Returns the copy's index.
    pub fn duplicate_scene(&mut self, index: usize) -> Option<usize> {
        let mut copy = self.scenes.get(index)?.clone();
        copy.name = self.unique_name(&format!("{} copy", copy.name), None);
        self.scenes.insert(index + 1, copy);
        if self.start > index {
            self.start += 1;
        }
        Some(index + 1)
    }

    /// Removes scene `index`; the last remaining scene cannot be removed.
    pub fn remove_scene(&mut self, index: usize) -> bool {
        if self.scenes.len() <= 1 || index >= self.scenes.len() {
            return false;
        }
        self.scenes.remove(index);
        if self.start > index || self.start >= self.scenes.len() {
            self.start = self.start.saturating_sub(1);
        }
        true
    }

    /// Moves scene `from` to position `to`. The start scene stays the same scene. Returns the new index.
    pub fn move_scene(&mut self, from: usize, to: usize) -> usize {
        if from >= self.scenes.len() {
            return from;
        }
        let to = to.min(self.scenes.len() - 1);
        let starting_scene = self.scenes[self.start].name.clone(); // names are unique
        let scene = self.scenes.remove(from);
        self.scenes.insert(to, scene);
        self.start = self.index_of(&starting_scene).unwrap_or(0);
        to
    }

    /// Renames scene `index`, keeping names unique.
    pub fn rename_scene(&mut self, index: usize, name: &str) {
        let name = self.unique_name(name, Some(index));
        if let Some(scene) = self.scenes.get_mut(index) {
            scene.name = name;
        }
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(project: &Project) -> Vec<&str> {
        project.scenes.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn a_new_project_has_one_scene_with_a_camera() {
        let project = Project::new();
        assert_eq!((project.scenes.len(), project.start), (1, 0));
        assert!(project.scenes[0].main_camera().is_some());
    }

    #[test]
    fn scene_names_stay_unique() {
        let mut project = Project::new();
        let second = project.add_scene();
        assert_eq!(project.scenes[second].name, "Scene 2");
        project.rename_scene(second, "Scene 1");
        assert_eq!(names(&project), ["Scene 1", "Scene 1 2"]);
        project.rename_scene(second, "  ");
        assert_eq!(project.scenes[second].name, "Scene");
        project.rename_scene(0, "Scene"); // another scene has this name
        assert_eq!(project.scenes[0].name, "Scene 2");
    }

    #[test]
    fn duplicating_inserts_below_and_keeps_the_start_scene() {
        let mut project = Project::new();
        project.add_scene();
        project.start = 1;
        assert_eq!(project.duplicate_scene(0), Some(1));
        assert_eq!(names(&project), ["Scene 1", "Scene 1 copy", "Scene 2"]);
        assert_eq!(project.start, 2, "still the same scene");
    }

    #[test]
    fn removing_keeps_at_least_one_scene_and_a_valid_start() {
        let mut project = Project::new();
        project.add_scene();
        project.add_scene();
        project.start = 2;
        assert!(project.remove_scene(0));
        assert_eq!(project.start, 1);
        assert!(project.remove_scene(1));
        assert_eq!(project.start, 0);
        assert!(!project.remove_scene(0), "the last scene stays");
        assert_eq!(project.scenes.len(), 1);
    }

    #[test]
    fn moving_a_scene_keeps_the_start_scene() {
        let mut project = Project::new();
        project.add_scene();
        project.add_scene();
        project.start = 0;
        assert_eq!(project.move_scene(0, 2), 2);
        assert_eq!(names(&project), ["Scene 2", "Scene 3", "Scene 1"]);
        assert_eq!(project.start, 2);
    }
}
