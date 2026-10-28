//! The game camera: an object whose frame is what the player sees.
//!
//! A camera node's position is the center of its frame and its rotation turns the frame; its
//! scale is ignored (the frame size is the `size` field). Parenting a camera to another object
//! makes it follow that object. The first camera in the scene is the one that is used; a scene
//! without any camera is shown through a default one, so older projects keep working.

use crate::geometry::{distance_to_segment, Affine2, Rect, Vec2};
use crate::path::Path;
use crate::scene::{NodeId, NodeKind, Scene};
use crate::style::Color;
use serde::{Deserialize, Serialize};

pub const DEFAULT_SIZE: Vec2 = Vec2::new(1280.0, 720.0);
pub const DEFAULT_BACKGROUND: Color = Color::rgb(245, 245, 245);

/// The data of a camera node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameCamera {
    /// How much of the world the frame covers, in world units. Its shape is the game's aspect ratio.
    pub size: Vec2,
    /// Fills the frame behind all objects.
    pub background: Color,
}

impl Default for GameCamera {
    fn default() -> Self {
        Self { size: DEFAULT_SIZE, background: DEFAULT_BACKGROUND }
    }
}

/// A camera placed in the world: where its frame is and what it looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    /// World position of the frame's center.
    pub center: Vec2,
    /// Rotation of the frame in radians.
    pub rotation: f32,
    pub size: Vec2,
    pub background: Color,
}

impl CameraView {
    /// The view of `camera` placed at `world` (a node's world transform; its scale is ignored).
    pub fn new(camera: &GameCamera, world: &Affine2) -> Self {
        let x_axis = world.matrix2.x_axis;
        Self { center: world.translation, rotation: x_axis.y.atan2(x_axis.x), size: camera.size, background: camera.background }
    }

    /// Maps frame space (origin at the frame's center) to world space.
    fn frame_to_world(&self) -> Affine2 {
        Affine2::from_angle_translation(self.rotation, self.center)
    }

    /// Corners of the frame in world space, clockwise from the top-left of the frame.
    pub fn corners(&self) -> [Vec2; 4] {
        let (h, to_world) = (self.size * 0.5, self.frame_to_world());
        [Vec2::new(-h.x, -h.y), Vec2::new(h.x, -h.y), Vec2::new(h.x, h.y), Vec2::new(-h.x, h.y)].map(|c| to_world.transform_point2(c))
    }

    /// Axis-aligned world bounds of the frame.
    pub fn bounds(&self) -> Rect {
        Rect::from_cloud(self.corners()).unwrap_or(Rect { min: self.center, max: self.center })
    }

    /// The frame as a closed rectangle path in world space.
    pub fn outline(&self) -> Path {
        Path::rect(-self.size * 0.5, self.size * 0.5).transformed(&self.frame_to_world())
    }

    /// Distance from `p` to the frame's border (0 on the border).
    pub fn distance_to_border(&self, p: Vec2) -> f32 {
        let c = self.corners();
        (0..4).map(|i| distance_to_segment(p, c[i], c[(i + 1) % 4])).fold(f32::INFINITY, f32::min)
    }

    /// World to pixels for a render target of `pixels` size that shows exactly the frame.
    pub fn view_matrix(&self, pixels: Vec2) -> Affine2 {
        Affine2::from_scale(pixels / self.size) * Affine2::from_translation(self.size * 0.5) * self.frame_to_world().inverse()
    }
}

impl Scene {
    /// A scene that holds a "Main Camera" whose frame spans `(0, 0)` to `size`.
    pub fn with_camera(size: Vec2) -> Scene {
        let mut scene = Scene::default();
        let camera = scene.new_camera(size, size * 0.5);
        scene.add(camera, None);
        scene
    }

    /// A camera node (not yet in the tree) with its frame centered at `center`.
    pub fn new_camera(&mut self, size: Vec2, center: Vec2) -> crate::scene::Node {
        let mut node = self.new_node("Main Camera", NodeKind::Camera(GameCamera { size, ..GameCamera::default() }));
        node.transform = Affine2::from_translation(center);
        node
    }

    /// The camera the game is shown through: the first one in document order.
    pub fn main_camera(&self) -> Option<NodeId> {
        let mut first = None;
        self.for_each_node(|node| {
            if first.is_none() && matches!(node.kind, NodeKind::Camera(_)) {
                first = Some(node.id);
            }
        });
        first
    }

    /// The top-level objects of `ids` that can be deleted without leaving the scene without a camera.
    /// If the deletion would remove every camera, the one holding the main camera (the camera itself,
    /// or the group around it) is left out. Scenes that have no camera to begin with are not affected.
    pub fn deletable(&self, ids: &[NodeId]) -> Vec<NodeId> {
        let mut top = self.top_level_selection(ids);
        let mut cameras: Vec<NodeId> = Vec::new();
        self.for_each_node(|node| {
            if matches!(node.kind, NodeKind::Camera(_)) {
                cameras.push(node.id);
            }
        });
        let doomed = |camera: &NodeId| top.iter().any(|&id| id == *camera || self.is_descendant(id, *camera));
        if !cameras.is_empty() && cameras.iter().all(doomed) {
            let main = cameras[0];
            top.retain(|&id| id != main && !self.is_descendant(id, main));
        }
        top
    }

    /// What the player sees: the main camera, or a default frame over `(0, 0)..(1280, 720)` without one.
    pub fn camera_view(&self) -> CameraView {
        self.main_camera()
            .and_then(|id| match (&self.find(id)?.kind, self.world_transform(id)) {
                (NodeKind::Camera(camera), Some(world)) => Some(CameraView::new(camera, &world)),
                _ => None,
            })
            .unwrap_or_else(|| CameraView::new(&GameCamera::default(), &Affine2::from_translation(DEFAULT_SIZE * 0.5)))
    }

    /// Projects saved before cameras existed stored the screen size and color on the scene:
    /// they become a camera spanning `(0, 0)..size`.
    pub(crate) fn upgrade_legacy_view(&mut self) {
        let (size, background) = (self.legacy_size.take(), self.legacy_background.take());
        if size.is_none() && background.is_none() || self.main_camera().is_some() {
            return;
        }
        let size = size.unwrap_or(DEFAULT_SIZE);
        let mut camera = self.new_camera(size, size * 0.5);
        if let NodeKind::Camera(data) = &mut camera.kind {
            data.background = background.unwrap_or(DEFAULT_BACKGROUND);
        }
        self.insert(camera, None, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_without_a_camera_uses_the_default_frame() {
        let view = Scene::default().camera_view();
        assert_eq!((view.size, view.center), (DEFAULT_SIZE, DEFAULT_SIZE * 0.5));
    }

    #[test]
    fn the_new_scene_camera_spans_the_given_size_from_the_origin() {
        let scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let view = scene.camera_view();
        assert_eq!(view.bounds(), Rect { min: Vec2::ZERO, max: Vec2::new(200.0, 100.0) });
        // The frame fills the target: its corners land on the corners of the pixel area.
        let m = view.view_matrix(Vec2::new(400.0, 200.0));
        assert!(m.transform_point2(Vec2::ZERO).distance(Vec2::ZERO) < 1e-3);
        assert!(m.transform_point2(Vec2::new(200.0, 100.0)).distance(Vec2::new(400.0, 200.0)) < 1e-3);
    }

    #[test]
    fn moving_and_rotating_the_camera_moves_the_view() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let id = scene.main_camera().unwrap();
        scene.find_mut(id).unwrap().transform = Affine2::from_angle_translation(std::f32::consts::FRAC_PI_2, Vec2::new(500.0, 500.0));
        let view = scene.camera_view();
        assert!(view.center.distance(Vec2::new(500.0, 500.0)) < 1e-3);
        // Turned a quarter turn, the 200 x 100 frame is 100 wide and 200 tall in the world.
        let size = view.bounds().size();
        assert!((size.x - 100.0).abs() < 1e-2 && (size.y - 200.0).abs() < 1e-2, "{size:?}");
        // The frame's center is the center of the picture.
        let center = view.view_matrix(Vec2::new(200.0, 100.0)).transform_point2(view.center);
        assert!(center.distance(Vec2::new(100.0, 50.0)) < 1e-3);
    }

    #[test]
    fn the_scale_of_a_camera_node_is_ignored() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let id = scene.main_camera().unwrap();
        scene.find_mut(id).unwrap().transform = Affine2::from_scale_angle_translation(Vec2::splat(3.0), 0.0, Vec2::new(100.0, 50.0));
        assert_eq!(scene.camera_view().bounds().size(), Vec2::new(200.0, 100.0));
    }

    #[test]
    fn a_camera_inside_a_group_follows_it() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let camera = scene.main_camera().unwrap();
        let group = scene.group(&[camera]).unwrap();
        scene.find_mut(group).unwrap().transform = Affine2::from_translation(Vec2::new(40.0, 0.0));
        assert!(scene.camera_view().center.distance(Vec2::new(140.0, 50.0)) < 1e-3);
    }

    #[test]
    fn the_only_camera_cannot_be_deleted_but_other_objects_can() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let camera = scene.main_camera().unwrap();
        let other = scene.new_node("Other", NodeKind::Group(Vec::new()));
        let other_id = other.id;
        scene.add(other, None);
        assert_eq!(scene.deletable(&[camera]), Vec::<NodeId>::new());
        assert_eq!(scene.deletable(&[camera, other_id]), vec![other_id]);
    }

    #[test]
    fn a_group_holding_the_only_camera_cannot_be_deleted() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let camera = scene.main_camera().unwrap();
        let group = scene.group(&[camera]).unwrap();
        assert!(scene.deletable(&[group]).is_empty());
    }

    #[test]
    fn a_second_camera_can_be_deleted_and_scenes_without_cameras_are_unaffected() {
        let mut scene = Scene::with_camera(Vec2::new(200.0, 100.0));
        let first = scene.main_camera().unwrap();
        let extra = scene.new_camera(Vec2::new(10.0, 10.0), Vec2::ZERO);
        let extra_id = extra.id;
        scene.add(extra, None);
        assert_eq!(scene.deletable(&[extra_id]), vec![extra_id]);
        // Asking for both keeps the main (first) one.
        assert_eq!(scene.deletable(&[first, extra_id]), vec![extra_id]);

        let mut bare = Scene::default();
        let node = bare.new_node("Solo", NodeKind::Group(Vec::new()));
        let id = node.id;
        bare.add(node, None);
        assert_eq!(bare.deletable(&[id]), vec![id]);
    }

    #[test]
    fn the_border_is_hit_but_the_inside_is_not() {
        let view = Scene::with_camera(Vec2::new(200.0, 100.0)).camera_view();
        assert!(view.distance_to_border(Vec2::new(100.0, 0.0)) < 1e-3);
        assert!(view.distance_to_border(Vec2::new(100.0, 50.0)) > 40.0);
    }

    #[test]
    fn old_projects_get_a_camera_from_their_screen_size_and_color() {
        let mut scene = Scene { legacy_size: Some(Vec2::new(300.0, 200.0)), legacy_background: Some(Color::rgb(1, 2, 3)), ..Scene::default() };
        scene.upgrade_legacy_view();
        let view = scene.camera_view();
        assert_eq!((view.bounds(), view.background), (Rect { min: Vec2::ZERO, max: Vec2::new(300.0, 200.0) }, Color::rgb(1, 2, 3)));
        assert_eq!(scene.nodes[0].name, "Main Camera");
        scene.upgrade_legacy_view();
        assert_eq!(scene.nodes.len(), 1, "upgrading twice must not add a second camera");
    }
}
