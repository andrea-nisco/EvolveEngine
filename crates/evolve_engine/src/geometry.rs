//! Small 2D math helpers shared by the whole engine.

pub use glam::{Affine2, Vec2};

/// Axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn from_points(a: Vec2, b: Vec2) -> Self {
        Self { min: a.min(b), max: a.max(b) }
    }

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    pub fn expanded(&self, amount: f32) -> Rect {
        Rect { min: self.min - Vec2::splat(amount), max: self.max + Vec2::splat(amount) }
    }

    /// Bounding box of a point cloud, or `None` if it is empty.
    pub fn from_cloud(points: impl IntoIterator<Item = Vec2>) -> Option<Rect> {
        points
            .into_iter()
            .map(|p| Rect { min: p, max: p })
            .reduce(|a, b| a.union(&b))
    }

    /// The four corners, clockwise from the top-left.
    pub fn corners(&self) -> [Vec2; 4] {
        [
            self.min,
            Vec2::new(self.max.x, self.min.y),
            self.max,
            Vec2::new(self.min.x, self.max.y),
        ]
    }
}

/// Approximate uniform scale factor of an affine transform (sqrt of |det|).
pub fn mean_scale(m: &Affine2) -> f32 {
    m.matrix2.determinant().abs().sqrt()
}

/// Distance from `p` to the segment `a`-`b`.
pub fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_point_on_segment(p, a, b))
}

/// Distance along a ray (`direction` must be a unit vector) at which it crosses the segment `a`-`b`.
pub fn ray_segment_distance(origin: Vec2, direction: Vec2, a: Vec2, b: Vec2) -> Option<f32> {
    let edge = b - a;
    let denominator = direction.perp_dot(edge);
    if denominator.abs() < 1e-9 {
        return None; // parallel
    }
    let to_start = a - origin;
    let along_ray = to_start.perp_dot(edge) / denominator;
    let along_edge = to_start.perp_dot(direction) / denominator;
    (along_ray >= 0.0 && (0.0..=1.0).contains(&along_edge)).then_some(along_ray)
}

/// Closest point to `p` on the segment `a`-`b`.
pub fn closest_point_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0)
}

/// Rotates `p` around `pivot` by `angle` radians.
pub fn rotate_around(angle: f32, pivot: Vec2) -> Affine2 {
    Affine2::from_translation(pivot) * Affine2::from_angle(angle) * Affine2::from_translation(-pivot)
}

/// Scales around `pivot` by `scale`.
pub fn scale_around(scale: Vec2, pivot: Vec2) -> Affine2 {
    Affine2::from_translation(pivot) * Affine2::from_scale(scale) * Affine2::from_translation(-pivot)
}
