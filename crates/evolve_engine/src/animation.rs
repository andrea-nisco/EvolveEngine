//! Keyframe animation: clips made of per-property tracks, with cubic-Bézier easing.
//!
//! A [`Clip`] never touches the scene by itself. Sampling it at a time gives a [`Pose`]
//! (values for the animated nodes) which is either drawn directly or written into the scene.

use crate::geometry::{Affine2, Vec2};
use crate::morph;
use crate::path::Path;
use crate::scene::{Node, NodeId, NodeKind, Scene};
use crate::style::{Color, Paint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const TIME_EPSILON: f32 = 1e-4;

// --- Easing ----------------------------------------------------------------------------------

/// A cubic Bézier easing curve through (0,0) and (1,1) with control points (x1,y1) and (x2,y2),
/// like CSS `cubic-bezier()`. `x` is the normalized time, `y` the progress.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Easing {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl Easing {
    pub const LINEAR: Easing = Easing { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
    pub const EASE_IN: Easing = Easing { x1: 0.42, y1: 0.0, x2: 1.0, y2: 1.0 };
    pub const EASE_OUT: Easing = Easing { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
    pub const EASE_IN_OUT: Easing = Easing { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    pub const PRESETS: [(&'static str, Easing); 4] =
        [("Linear", Self::LINEAR), ("Ease in", Self::EASE_IN), ("Ease out", Self::EASE_OUT), ("Ease in-out", Self::EASE_IN_OUT)];

    fn bezier(a: f32, b: f32, s: f32) -> f32 {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    }

    /// Progress (0..=1, may overshoot) for normalized time `t` (0..=1).
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        // x(s) is monotonic for control x in 0..=1, so bisection finds the curve parameter.
        let (x1, x2) = (self.x1.clamp(0.0, 1.0), self.x2.clamp(0.0, 1.0));
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if Self::bezier(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self::bezier(self.y1, self.y2, (lo + hi) * 0.5)
    }
}

// --- Properties and values -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Property {
    Position,
    /// Radians.
    Rotation,
    Scale,
    Opacity,
    /// Solid fill color.
    Fill,
    /// Solid stroke color.
    Stroke,
    /// The outline itself: keys of this property morph one shape into another.
    Shape,
}

impl Property {
    pub const ALL: [Property; 7] = [
        Property::Position,
        Property::Rotation,
        Property::Scale,
        Property::Opacity,
        Property::Fill,
        Property::Stroke,
        Property::Shape,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Property::Position => "Position",
            Property::Rotation => "Rotation",
            Property::Scale => "Scale",
            Property::Opacity => "Opacity",
            Property::Fill => "Fill",
            Property::Stroke => "Stroke",
            Property::Shape => "Shape (morph)",
        }
    }

    /// The node's current value, or `None` if the property does not apply
    /// (for instance fills that are gradients cannot be animated).
    pub fn read(self, node: &Node) -> Option<Value> {
        let (position, rotation, scale) = decompose(&node.transform);
        match self {
            Property::Position => Some(Value::Vec2(position)),
            Property::Rotation => Some(Value::Float(rotation)),
            Property::Scale => Some(Value::Vec2(scale)),
            Property::Opacity => Some(Value::Float(node.opacity)),
            Property::Shape => match &node.kind {
                // A skinned shape's weights belong to its current anchors, so it can't be morphed.
                NodeKind::Shape(shape) if shape.skin.is_none() => Some(Value::Path(shape.path.clone())),
                _ => None,
            },
            Property::Fill | Property::Stroke => {
                let NodeKind::Shape(shape) = &node.kind else { return None };
                let paint = if self == Property::Fill { shape.fill.as_ref() } else { shape.stroke.as_ref().map(|s| &s.paint) };
                match paint {
                    Some(Paint::Solid(color)) => Some(Value::Color(*color)),
                    _ => None,
                }
            }
        }
    }
}

/// Splits a transform into translation, rotation (radians) and scale. Skew is dropped.
pub fn decompose(m: &Affine2) -> (Vec2, f32, Vec2) {
    let x_axis = m.matrix2.x_axis;
    let scale_x = x_axis.length();
    let scale_y = if scale_x > 1e-9 { m.matrix2.determinant() / scale_x } else { m.matrix2.y_axis.length() };
    let angle = if scale_x > 1e-9 { x_axis.y.atan2(x_axis.x) } else { 0.0 };
    (m.translation, angle, Vec2::new(scale_x, scale_y))
}

pub fn compose(position: Vec2, rotation: f32, scale: Vec2) -> Affine2 {
    Affine2::from_scale_angle_translation(scale, rotation, position)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Vec2(Vec2),
    Float(f32),
    Color(Color),
    /// A whole outline, for shape morphing.
    Path(Path),
}

impl Value {
    /// Linear blend towards `other` (paths are morphed). Mismatched kinds keep `self`.
    pub fn lerp(&self, other: &Value, t: f32) -> Value {
        match (self, other) {
            (Value::Vec2(a), Value::Vec2(b)) => Value::Vec2(a.lerp(*b, t)),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + (b - a) * t),
            (Value::Color(a), Value::Color(b)) => {
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8;
                Value::Color(Color::rgba(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a)))
            }
            (Value::Path(a), Value::Path(b)) => Value::Path(morph::morph(a, b, t)),
            _ => self.clone(),
        }
    }

    /// True if the two values differ by more than editing noise.
    pub fn differs(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Vec2(a), Value::Vec2(b)) => a.distance(*b) > 1e-3,
            (Value::Float(a), Value::Float(b)) => (a - b).abs() > 1e-4,
            (Value::Color(a), Value::Color(b)) => a != b,
            (Value::Path(a), Value::Path(b)) => a != b,
            _ => true,
        }
    }
}

// --- Tracks and clips ------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Seconds from the start of the clip.
    pub time: f32,
    pub value: Value,
    /// Shapes the transition from this key to the next one.
    pub easing: Easing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub node: NodeId,
    pub property: Property,
    /// Sorted by time.
    pub keys: Vec<Keyframe>,
}

impl Track {
    /// Value at `time`: held before the first and after the last key, eased in between.
    pub fn sample(&self, time: f32) -> Option<Value> {
        let first = self.keys.first()?;
        let last = self.keys.last()?;
        if time <= first.time {
            return Some(first.value.clone());
        }
        if time >= last.time {
            return Some(last.value.clone());
        }
        let next = self.keys.iter().position(|k| k.time > time)?;
        let (a, b) = (&self.keys[next - 1], &self.keys[next]);
        let t = (time - a.time) / (b.time - a.time);
        Some(a.value.lerp(&b.value, a.easing.apply(t)))
    }

    /// Adds a key at `time`, or replaces the value of the key already there. Returns its index.
    pub fn set_key(&mut self, time: f32, mut value: Value) -> usize {
        // Keep rotations on the shortest turn from what the track shows at that time.
        if let (Property::Rotation, Value::Float(angle), Some(Value::Float(reference))) =
            (self.property, &value, self.sample(time))
        {
            let turns = ((reference - angle) / std::f32::consts::TAU).round();
            value = Value::Float(angle + turns * std::f32::consts::TAU);
        }
        if let Some(i) = self.keys.iter().position(|k| (k.time - time).abs() < TIME_EPSILON) {
            self.keys[i].value = value;
            return i;
        }
        let index = self.keys.iter().position(|k| k.time > time).unwrap_or(self.keys.len());
        self.keys.insert(index, Keyframe { time, value, easing: Easing::EASE_IN_OUT });
        index
    }
}

/// One edit made by the user: `node`'s `property` went from `old` to `new`.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub node: NodeId,
    pub property: Property,
    pub old: Value,
    pub new: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub fps: f32,
    pub tracks: Vec<Track>,
}

impl Clip {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), duration: 2.0, fps: 30.0, tracks: Vec::new() }
    }

    pub fn track_index(&self, node: NodeId, property: Property) -> Option<usize> {
        self.tracks.iter().position(|t| t.node == node && t.property == property)
    }

    fn track_mut(&mut self, node: NodeId, property: Property) -> &mut Track {
        let index = self.track_index(node, property).unwrap_or_else(|| {
            self.tracks.push(Track { node, property, keys: Vec::new() });
            self.tracks.len() - 1
        });
        &mut self.tracks[index]
    }

    /// Snaps `time` to the clip's frame grid and keeps it inside the clip.
    pub fn snap_time(&self, time: f32) -> f32 {
        ((time * self.fps).round() / self.fps).clamp(0.0, self.duration)
    }

    /// Sets a key at `time`. A track's first key made after the start also gets a key at 0 holding
    /// `before`, so the animation starts from the original look.
    pub fn set_key(&mut self, node: NodeId, property: Property, time: f32, value: Value, before: Option<Value>) {
        let track = self.track_mut(node, property);
        if let (true, true, Some(before)) = (track.keys.is_empty(), time > TIME_EPSILON, before) {
            track.set_key(0.0, before);
        }
        track.set_key(time, value);
    }

    /// Records user edits as keys at `time`.
    pub fn record(&mut self, changes: &[Change], time: f32) {
        for c in changes {
            self.set_key(c.node, c.property, time, c.new.clone(), Some(c.old.clone()));
        }
    }

    /// Values of every animated node at `time`. Transform parts without a track keep the
    /// node's current value.
    pub fn pose_at(&self, scene: &Scene, time: f32) -> Pose {
        let mut pose = Pose::new();
        let mut transforms: HashMap<NodeId, (Vec2, f32, Vec2)> = HashMap::new();
        for track in &self.tracks {
            let (Some(node), Some(value)) = (scene.find(track.node), track.sample(time)) else { continue };
            let entry = pose.entry(track.node).or_default();
            match (track.property, value) {
                (Property::Position, Value::Vec2(v)) => transforms.entry(track.node).or_insert_with(|| decompose(&node.transform)).0 = v,
                (Property::Rotation, Value::Float(v)) => transforms.entry(track.node).or_insert_with(|| decompose(&node.transform)).1 = v,
                (Property::Scale, Value::Vec2(v)) => transforms.entry(track.node).or_insert_with(|| decompose(&node.transform)).2 = v,
                (Property::Opacity, Value::Float(v)) => entry.opacity = Some(v.clamp(0.0, 1.0)),
                (Property::Fill, Value::Color(c)) => entry.fill = Some(c),
                (Property::Stroke, Value::Color(c)) => entry.stroke = Some(c),
                (Property::Shape, Value::Path(p)) => entry.path = Some(p),
                _ => {}
            }
        }
        for (id, (position, rotation, scale)) in transforms {
            pose.entry(id).or_default().transform = Some(compose(position, rotation, scale));
        }
        pose
    }
}

// --- Poses -----------------------------------------------------------------------------------

/// The animated values of one node. `None` means "not animated".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodePose {
    pub transform: Option<Affine2>,
    pub opacity: Option<f32>,
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub path: Option<Path>,
}

pub type Pose = HashMap<NodeId, NodePose>;

impl Scene {
    /// Writes a pose into the nodes (so the animated state can be edited and saved).
    pub fn apply_pose(&mut self, pose: &Pose) {
        for (&id, p) in pose {
            // Only what the pose really sets is stamped, so a pose that moves things does not make a renderer
            // rebuild their shapes.
            let Some(mut node) = self.find_mut(id) else { continue };
            if let Some(transform) = p.transform {
                node.set_transform(transform);
            }
            if let Some(opacity) = p.opacity {
                node.set_opacity(opacity);
            }
            if let (Some(path), true) = (&p.path, matches!(&node.kind, NodeKind::Shape(shape) if shape.skin.is_none())) {
                if let Some(target) = node.path_mut() {
                    *target = path.clone();
                }
            }
            if let Some(color) = p.fill {
                node.set_solid_fill_color(color);
            }
            if let Some(color) = p.stroke {
                node.set_solid_stroke_color(color);
            }
        }
    }

    /// Animatable properties that differ between `before` and this scene (nodes present in both).
    pub fn changes_since(&self, before: &Scene) -> Vec<Change> {
        let mut changes = Vec::new();
        for id in self.all_ids() {
            let (Some(new_node), Some(old_node)) = (self.find(id), before.find(id)) else { continue };
            for property in Property::ALL {
                if let (Some(old), Some(new)) = (property.read(old_node), property.read(new_node)) {
                    if old.differs(&new) {
                        changes.push(Change { node: id, property, old, new });
                    }
                }
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path;
    use crate::scene::Shape;

    fn scene_with_square() -> (Scene, NodeId) {
        let mut scene = Scene::default();
        let node = scene.new_node("sq", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)))));
        let id = scene.add(node, None);
        (scene, id)
    }

    fn linear_track(node: NodeId) -> Track {
        let key = |time, x| Keyframe { time, value: Value::Vec2(Vec2::new(x, 0.0)), easing: Easing::LINEAR };
        Track { node, property: Property::Position, keys: vec![key(0.0, 0.0), key(1.0, 100.0)] }
    }

    #[test]
    fn easing_hits_its_end_points_and_eases() {
        for easing in [Easing::LINEAR, Easing::EASE_IN, Easing::EASE_OUT, Easing::EASE_IN_OUT] {
            assert!(easing.apply(0.0).abs() < 1e-3);
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-3);
        }
        assert!((Easing::LINEAR.apply(0.3) - 0.3).abs() < 1e-3);
        assert!(Easing::EASE_IN.apply(0.5) < 0.5);
        assert!(Easing::EASE_OUT.apply(0.5) > 0.5);
    }

    #[test]
    fn tracks_hold_at_the_ends_and_interpolate_between_keys() {
        let track = linear_track(NodeId(1));
        assert_eq!(track.sample(-1.0), Some(Value::Vec2(Vec2::ZERO)));
        assert_eq!(track.sample(2.0), Some(Value::Vec2(Vec2::new(100.0, 0.0))));
        let Some(Value::Vec2(mid)) = track.sample(0.25) else { panic!("no value") };
        assert!((mid.x - 25.0).abs() < 0.1);
    }

    #[test]
    fn set_key_replaces_or_inserts_in_order() {
        let mut track = linear_track(NodeId(1));
        track.set_key(0.5, Value::Vec2(Vec2::new(10.0, 0.0)));
        track.set_key(1.0, Value::Vec2(Vec2::new(50.0, 0.0)));
        let times: Vec<f32> = track.keys.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![0.0, 0.5, 1.0]);
        assert_eq!(track.keys[2].value, Value::Vec2(Vec2::new(50.0, 0.0)));
    }

    #[test]
    fn pose_moves_the_node_and_keeps_its_other_transform_parts() {
        let (mut scene, id) = scene_with_square();
        scene.find_mut(id).unwrap().transform = compose(Vec2::ZERO, 0.5, Vec2::splat(2.0));
        let mut clip = Clip::new("test");
        clip.tracks.push(linear_track(id));
        let pose = clip.pose_at(&scene, 0.5);
        scene.apply_pose(&pose);
        let (position, rotation, scale) = decompose(&scene.find(id).unwrap().transform);
        assert!((position.x - 50.0).abs() < 0.1);
        assert!((rotation - 0.5).abs() < 1e-4);
        assert!((scale.x - 2.0).abs() < 1e-4);
    }

    #[test]
    fn recording_an_edit_keys_the_start_and_the_edit_time() {
        let (before, id) = scene_with_square();
        let mut after = before.clone();
        after.find_mut(id).unwrap().transform = Affine2::from_translation(Vec2::new(40.0, 0.0));
        let changes = after.changes_since(&before);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].property, Property::Position);

        let mut clip = Clip::new("test");
        clip.record(&changes, 1.5);
        let times: Vec<f32> = clip.tracks[0].keys.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![0.0, 1.5]);
    }

    #[test]
    fn rotation_keys_take_the_short_way_round() {
        let mut track = Track { node: NodeId(1), property: Property::Rotation, keys: Vec::new() };
        track.set_key(0.0, Value::Float(3.0));
        track.set_key(1.0, Value::Float(-3.0)); // same direction, 0.28 rad further, not 6 rad back
        let Value::Float(end) = track.keys[1].value else { panic!("not a float") };
        assert!((end - (-3.0 + std::f32::consts::TAU)).abs() < 1e-4);
    }

    #[test]
    fn shape_keys_morph_the_outline() {
        let (mut scene, id) = scene_with_square(); // 0..10 square
        let big = Path::rect(Vec2::ZERO, Vec2::splat(30.0));
        let mut clip = Clip::new("morph");
        let small = Property::Shape.read(scene.find(id).unwrap()).unwrap();
        clip.set_key(id, Property::Shape, 0.0, small, None);
        clip.set_key(id, Property::Shape, 1.0, Value::Path(big), None);
        clip.tracks[0].keys[0].easing = Easing::LINEAR;
        let pose = clip.pose_at(&scene, 0.5);
        scene.apply_pose(&pose);
        let NodeKind::Shape(shape) = &scene.find(id).unwrap().kind else { panic!("not a shape") };
        let size = shape.path.bounds().unwrap().size();
        assert!((size.x - 20.0).abs() < 0.1, "halfway between 10 and 30 is 20, got {size:?}");
    }

    #[test]
    fn gradient_fills_are_not_animatable() {
        let (mut scene, id) = scene_with_square();
        if let Some(mut node) = scene.find_mut(id) {
            if let NodeKind::Shape(shape) = &mut node.kind {
                shape.fill = Some(Paint::linear(Vec2::ZERO, Vec2::X, Color::BLACK, Color::WHITE));
            }
        }
        assert_eq!(Property::Fill.read(scene.find(id).unwrap()), None);
    }
}
