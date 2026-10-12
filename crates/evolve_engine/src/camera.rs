//! Infinite 2D camera with pan, zoom and rotation.

use crate::game_camera::CameraView;
use crate::geometry::{Affine2, Vec2};

/// Maps world space to screen space. Screen units are whatever the host uses for
/// pointer input (egui points in the editor).
#[derive(Clone, Debug)]
pub struct Camera {
    /// World point shown at the center of the viewport.
    pub center: Vec2,
    /// Screen units per world unit.
    pub zoom: f32,
    /// View rotation in radians.
    pub rotation: f32,
    /// Viewport size in screen units.
    pub viewport: Vec2,
}

impl Camera {
    pub const MIN_ZOOM: f32 = 0.01;
    pub const MAX_ZOOM: f32 = 1000.0;

    pub fn new(viewport: Vec2) -> Self {
        Self { center: Vec2::ZERO, zoom: 1.0, rotation: 0.0, viewport }
    }

    /// A view that shows exactly what the game camera `view` sees, in a viewport of the same shape.
    /// Useful to draw things over a game picture with the same code that draws over the editor view.
    pub fn matching(view: &CameraView, viewport: Vec2) -> Self {
        let zoom = (viewport / view.size.max(Vec2::splat(1e-3))).min_element();
        Self { center: view.center, zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM), rotation: -view.rotation, viewport }
    }

    /// World → screen transform.
    pub fn view_matrix(&self) -> Affine2 {
        Affine2::from_translation(self.viewport * 0.5)
            * Affine2::from_angle(self.rotation)
            * Affine2::from_scale(Vec2::splat(self.zoom))
            * Affine2::from_translation(-self.center)
    }

    pub fn world_to_screen(&self, p: Vec2) -> Vec2 {
        self.view_matrix().transform_point2(p)
    }

    pub fn screen_to_world(&self, p: Vec2) -> Vec2 {
        self.view_matrix().inverse().transform_point2(p)
    }

    /// Moves the view so the scene follows a pointer drag of `delta` screen units.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        let scale = Affine2::from_angle(self.rotation) * Affine2::from_scale(Vec2::splat(self.zoom));
        self.center -= scale.inverse().transform_vector2(delta);
    }

    /// Zooms by `factor`, keeping the world point under `screen_point` fixed.
    pub fn zoom_at(&mut self, screen_point: Vec2, factor: f32) {
        let anchor = self.screen_to_world(screen_point);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.center += anchor - self.screen_to_world(screen_point);
    }

    /// Rotates the view by `angle` radians around `screen_point`.
    pub fn rotate_at(&mut self, screen_point: Vec2, angle: f32) {
        let anchor = self.screen_to_world(screen_point);
        self.rotation += angle;
        self.center += anchor - self.screen_to_world(screen_point);
    }

    /// Centers on `world_center` and picks the largest zoom that fits a `size` area with some margin.
    pub fn frame(&mut self, world_center: Vec2, size: Vec2) {
        let fit = (self.viewport / size.max(Vec2::splat(1e-3))).min_element() * 0.9;
        self.center = world_center;
        self.zoom = fit.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.rotation = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_world_round_trip() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.zoom = 2.5;
        cam.rotation = 0.7;
        cam.center = Vec2::new(30.0, -12.0);
        let p = Vec2::new(123.0, 456.0);
        assert!(cam.screen_to_world(cam.world_to_screen(p)).distance(p) < 1e-3);
    }

    #[test]
    fn a_matching_view_maps_the_game_frame_onto_the_viewport() {
        let mut scene = crate::scene::Scene::with_camera(Vec2::new(200.0, 100.0));
        let id = scene.main_camera().unwrap();
        scene.find_mut(id).unwrap().transform = Affine2::from_angle_translation(0.4, Vec2::new(30.0, 70.0));
        let view = scene.camera_view();
        let viewport = Vec2::new(400.0, 200.0);
        let (editor_view, game_view) = (Camera::matching(&view, viewport), view.view_matrix(viewport));
        for p in [Vec2::ZERO, Vec2::new(100.0, -20.0), Vec2::new(-5.0, 40.0)] {
            assert!(editor_view.world_to_screen(p).distance(game_view.transform_point2(p)) < 1e-2);
        }
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.rotation = 0.3;
        let cursor = Vec2::new(100.0, 80.0);
        let before = cam.screen_to_world(cursor);
        cam.zoom_at(cursor, 3.0);
        assert!(cam.screen_to_world(cursor).distance(before) < 1e-3);
    }
}
