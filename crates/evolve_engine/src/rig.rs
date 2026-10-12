//! 2D skeletal rigging: bones, inverse kinematics and skinning of vector paths.
//!
//! A *bone* is a group node (so bones nest, move and animate like any node) flagged with a length.
//! It starts at its origin and points along its local +X axis. Child bones usually start at the
//! parent's tip. Shapes placed inside a bone follow it rigidly; a [`Skin`] makes a shape's anchors
//! follow several bones with weights, so the outline bends smoothly.

use crate::geometry::{distance_to_segment, rotate_around, Affine2, Vec2};
use crate::path::Path;
use crate::scene::{Node, NodeId, NodeKind, Scene, Shape};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub length: f32,
}

/// Keeps the tip of a bone chain on a target node. The chain is the tip bone and its bone
/// ancestors, up to `chain_length` bones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ik {
    pub name: String,
    pub tip: NodeId,
    pub chain_length: usize,
    pub target: NodeId,
}

/// Binds a shape's anchors to bones. The shape's path is its *bind pose*.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    pub bones: Vec<NodeId>,
    /// World-to-bone transform of each bone when the skin was bound.
    pub inverse_bind: Vec<Affine2>,
    /// The shape's local-to-world transform when the skin was bound.
    pub bind_world: Affine2,
    /// `weights[subpath][anchor][bone]`; the weights of one anchor add up to 1.
    pub weights: Vec<Vec<Vec<f32>>>,
}

impl Skin {
    /// How far each bone has moved since binding, as a transform of world points.
    fn bone_matrices(&self, scene: &Scene) -> Vec<Affine2> {
        self.bones
            .iter()
            .zip(&self.inverse_bind)
            .map(|(&bone, inverse)| scene.world_transform(bone).map_or(Affine2::IDENTITY, |world| world * *inverse))
            .collect()
    }

    /// The path as deformed by the bones' current pose, in the space of `world`'s inverse
    /// (pass the shape's world transform to get shape-local coordinates, or the identity for
    /// world coordinates). `None` if the path no longer matches the weights.
    pub fn deform(&self, scene: &Scene, path: &Path, world: Affine2) -> Option<Path> {
        let matching = path.subpaths.len() == self.weights.len()
            && path.subpaths.iter().zip(&self.weights).all(|(s, w)| s.anchors.len() == w.len());
        if !matching {
            return None;
        }
        let matrices = self.bone_matrices(scene);
        let to_local = world.inverse();
        let blend = |weights: &[f32], v: Vec2, is_point: bool| -> Vec2 {
            let total: f32 = weights.iter().sum();
            if total < 1e-6 {
                return v;
            }
            let moved: Vec2 = weights
                .iter()
                .zip(&matrices)
                .map(|(w, m)| *w * if is_point { m.transform_point2(v) } else { m.transform_vector2(v) })
                .sum();
            moved / total
        };

        let mut deformed = path.clone();
        for (sub, sub_weights) in deformed.subpaths.iter_mut().zip(&self.weights) {
            for (anchor, w) in sub.anchors.iter_mut().zip(sub_weights) {
                let pos = blend(w, self.bind_world.transform_point2(anchor.pos), true);
                let handle_in = blend(w, self.bind_world.transform_vector2(anchor.handle_in), false);
                let handle_out = blend(w, self.bind_world.transform_vector2(anchor.handle_out), false);
                anchor.pos = to_local.transform_point2(pos);
                anchor.handle_in = to_local.transform_vector2(handle_in);
                anchor.handle_out = to_local.transform_vector2(handle_out);
            }
        }
        Some(deformed)
    }

    /// Weight of bone `bone` for every anchor, in path order (for drawing a heat map).
    pub fn weights_of_bone(&self, bone: usize) -> Vec<f32> {
        self.weights.iter().flatten().map(|w| w.get(bone).copied().unwrap_or(0.0)).collect()
    }
}

/// Moves `weights[bone]` by `delta` and rescales the other weights so the sum stays 1.
fn paint_weight(weights: &mut [f32], bone: usize, delta: f32) {
    if weights.len() < 2 || bone >= weights.len() {
        return;
    }
    let old = weights[bone];
    let new = (old + delta).clamp(0.0, 1.0);
    let (others_before, others_after) = (1.0 - old, 1.0 - new);
    let even_share = others_after / (weights.len() - 1) as f32;
    for (i, w) in weights.iter_mut().enumerate() {
        if i != bone {
            // Keep the proportions between the other bones, unless they had no weight at all.
            *w = if others_before > 1e-6 { *w * others_after / others_before } else { even_share };
        }
    }
    weights[bone] = new;
}

/// Initial weights: nearer bones get more influence.
fn auto_weights(point: Vec2, segments: &[(Vec2, Vec2)]) -> Vec<f32> {
    let average_length = segments.iter().map(|(a, b)| a.distance(*b)).sum::<f32>() / segments.len().max(1) as f32;
    let softness = (average_length * 0.25).powi(2) + 1e-6;
    let raw: Vec<f32> = segments
        .iter()
        .map(|&(a, b)| {
            let d = distance_to_segment(point, a, b);
            1.0 / (d * d + softness)
        })
        .collect();
    let total: f32 = raw.iter().sum();
    raw.into_iter().map(|w| w / total).collect()
}

impl Shape {
    /// The outline to draw and hit-test: the bind-pose path, deformed if the shape is skinned.
    /// `world` is the shape's current local-to-world transform.
    pub fn effective_path(&self, scene: &Scene, world: Affine2) -> Cow<'_, Path> {
        match self.skin.as_ref().and_then(|skin| skin.deform(scene, &self.path, world)) {
            Some(path) => Cow::Owned(path),
            None => Cow::Borrowed(&self.path),
        }
    }
}

impl Scene {
    // --- Bones ---------------------------------------------------------------------------------

    pub fn is_bone(&self, id: NodeId) -> bool {
        self.find(id).is_some_and(|n| n.bone.is_some())
    }

    pub fn bone_ids(&self) -> Vec<NodeId> {
        self.all_ids().into_iter().filter(|&id| self.is_bone(id)).collect()
    }

    /// World positions of a bone's origin and tip.
    pub fn bone_segment(&self, id: NodeId) -> Option<(Vec2, Vec2)> {
        let length = self.find(id)?.bone?.length;
        let world = self.world_transform(id)?;
        Some((world.translation, world.transform_point2(Vec2::new(length, 0.0))))
    }

    /// Adds a bone whose origin is at `origin` (world) pointing in direction `angle`.
    /// A `parent` that is not a bone is ignored.
    pub fn add_bone(&mut self, parent: Option<NodeId>, origin: Vec2, angle: f32, length: f32) -> NodeId {
        let parent = parent.filter(|&p| self.is_bone(p));
        let name = format!("Bone {}", self.bone_ids().len() + 1);
        let mut node = self.new_node(name, NodeKind::Group(Vec::new()));
        node.bone = Some(Bone { length });
        let parent_world = parent.and_then(|p| self.world_transform(p)).unwrap_or(Affine2::IDENTITY);
        node.transform = parent_world.inverse() * Affine2::from_angle_translation(angle, origin);
        self.add(node, parent)
    }

    /// Places a bone by its world origin and direction, optionally changing its length.
    pub fn set_bone_world(&mut self, id: NodeId, origin: Vec2, angle: f32, length: Option<f32>) {
        let parent_inverse = self.parent_world(id).inverse();
        if let Some(node) = self.find_mut(id) {
            node.transform = parent_inverse * Affine2::from_angle_translation(angle, origin);
            if let (Some(bone), Some(length)) = (node.bone.as_mut(), length) {
                bone.length = length.max(1e-3);
            }
        }
    }

    /// The visible bone closest to `p`, if it is within `tolerance`.
    pub fn pick_bone(&self, p: Vec2, tolerance: f32) -> Option<NodeId> {
        self.bone_ids()
            .into_iter()
            .filter(|&id| self.find(id).is_some_and(|n| n.visible))
            .filter_map(|id| {
                let (a, b) = self.bone_segment(id)?;
                Some((id, distance_to_segment(p, a, b)))
            })
            .filter(|&(_, d)| d <= tolerance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    // --- Inverse kinematics --------------------------------------------------------------------

    /// The bones driven by `ik`, from the chain's root to its tip.
    pub fn ik_chain(&self, ik: &Ik) -> Vec<NodeId> {
        let Some(ancestry) = self.ancestry(ik.tip) else { return Vec::new() };
        let mut chain: Vec<NodeId> =
            ancestry.iter().rev().take_while(|&&id| self.is_bone(id)).take(ik.chain_length.max(1)).copied().collect();
        chain.reverse();
        chain
    }

    /// Adds an IK constraint. `tip` must be a bone and `target` any other node.
    pub fn create_ik(&mut self, tip: NodeId, target: NodeId, chain_length: usize) -> Option<usize> {
        if !self.is_bone(tip) || self.is_bone(target) || self.find(target).is_none() {
            return None;
        }
        let name = format!("IK {}", self.ik.len() + 1);
        self.ik.push(Ik { name, tip, chain_length, target });
        Some(self.ik.len() - 1)
    }

    /// Rotates the bones of every IK chain so their tip reaches its target (cyclic coordinate descent).
    pub fn solve_ik(&mut self) {
        for ik in self.ik.clone() {
            let chain = self.ik_chain(&ik);
            let (Some(&last), Some(target)) = (chain.last(), self.world_transform(ik.target).map(|w| w.translation)) else {
                continue;
            };
            for _ in 0..24 {
                let Some((_, tip)) = self.bone_segment(last) else { break };
                if tip.distance(target) < 0.01 {
                    break;
                }
                for &bone in chain.iter().rev() {
                    let (Some((origin, _)), Some((_, tip))) = (self.bone_segment(bone), self.bone_segment(last)) else { continue };
                    let (to_tip, to_target) = (tip - origin, target - origin);
                    if to_tip.length_squared() < 1e-9 || to_target.length_squared() < 1e-9 {
                        continue;
                    }
                    let Some(world) = self.world_transform(bone) else { continue };
                    let parent_inverse = self.parent_world(bone).inverse();
                    if let Some(node) = self.find_mut(bone) {
                        node.transform = parent_inverse * rotate_around(to_tip.angle_to(to_target), origin) * world;
                    }
                }
            }
        }
    }

    /// Copies the transforms of all IK-driven bones into `other` (a snapshot of the same scene).
    pub fn copy_ik_bones_to(&self, other: &mut Scene) {
        for ik in &self.ik {
            for bone in self.ik_chain(ik) {
                if let (Some(from), Some(to)) = (self.find(bone), other.find_mut(bone)) {
                    to.transform = from.transform;
                }
            }
        }
    }

    // --- Skinning ------------------------------------------------------------------------------

    /// Binds a shape to bones with automatic distance-based weights. The shape's current outline
    /// becomes its bind pose.
    pub fn bind_skin(&mut self, shape: NodeId, bones: &[NodeId]) -> bool {
        let bones: Vec<NodeId> = bones.iter().copied().filter(|&b| self.is_bone(b)).collect();
        let (Some(world), false) = (self.world_transform(shape), bones.is_empty()) else { return false };
        let segments: Vec<(Vec2, Vec2)> = bones.iter().filter_map(|&b| self.bone_segment(b)).collect();
        let inverse_bind: Vec<Affine2> = bones.iter().filter_map(|&b| self.world_transform(b).map(|w| w.inverse())).collect();
        if segments.len() != bones.len() || inverse_bind.len() != bones.len() {
            return false;
        }
        let Some(Node { kind: NodeKind::Shape(s), .. }) = self.find_mut(shape) else { return false };
        let weights = s
            .path
            .subpaths
            .iter()
            .map(|sub| sub.anchors.iter().map(|a| auto_weights(world.transform_point2(a.pos), &segments)).collect())
            .collect();
        s.skin = Some(Box::new(Skin { bones, inverse_bind, bind_world: world, weights }));
        true
    }

    /// Removes the skin; the shape goes back to its bind pose.
    pub fn unbind_skin(&mut self, shape: NodeId) -> bool {
        match self.find_mut(shape).map(|n| &mut n.kind) {
            Some(NodeKind::Shape(s)) => s.skin.take().is_some(),
            _ => false,
        }
    }

    /// Weight-paint brush: adds `amount` (negative to erase) of bone `bone_index`'s influence to the
    /// anchors within `radius` of `point` (world), strongest at the center. Anchors are tested where
    /// they currently appear.
    pub fn paint_skin(&mut self, shape: NodeId, point: Vec2, radius: f32, bone_index: usize, amount: f32) -> bool {
        let Some(Node { kind: NodeKind::Shape(s), .. }) = self.find(shape) else { return false };
        let Some(skin) = s.skin.as_ref() else { return false };
        let Some(shown) = skin.deform(self, &s.path, Affine2::IDENTITY) else { return false };
        let mut hits = Vec::new();
        for (si, sub) in shown.subpaths.iter().enumerate() {
            for (ai, anchor) in sub.anchors.iter().enumerate() {
                let t = 1.0 - anchor.pos.distance(point) / radius;
                if t > 0.0 {
                    hits.push((si, ai, t * t));
                }
            }
        }
        let Some(Node { kind: NodeKind::Shape(Shape { skin: Some(skin), .. }), .. }) = self.find_mut(shape) else {
            return false;
        };
        for (si, ai, falloff) in hits {
            paint_weight(&mut skin.weights[si][ai], bone_index, amount * falloff);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(scene: &mut Scene) -> (NodeId, NodeId) {
        let upper = scene.add_bone(None, Vec2::ZERO, 0.0, 10.0);
        let lower = scene.add_bone(Some(upper), Vec2::new(10.0, 0.0), 0.0, 10.0);
        (upper, lower)
    }

    fn target(scene: &mut Scene, at: Vec2) -> NodeId {
        let mut node = scene.new_node("target", NodeKind::Group(Vec::new()));
        node.transform = Affine2::from_translation(at);
        scene.add(node, None)
    }

    #[test]
    fn child_bones_follow_their_parent() {
        let mut scene = Scene::default();
        let (upper, lower) = arm(&mut scene);
        let (_, tip) = scene.bone_segment(lower).unwrap();
        assert!(tip.distance(Vec2::new(20.0, 0.0)) < 1e-4);
        scene.set_bone_world(upper, Vec2::ZERO, std::f32::consts::FRAC_PI_2, None); // point the upper bone down
        let (origin, tip) = scene.bone_segment(lower).unwrap();
        assert!(origin.distance(Vec2::new(0.0, 10.0)) < 1e-3);
        assert!(tip.distance(Vec2::new(0.0, 20.0)) < 1e-3);
    }

    #[test]
    fn ik_reaches_a_target_within_reach() {
        let mut scene = Scene::default();
        let (_, lower) = arm(&mut scene);
        let goal = target(&mut scene, Vec2::new(8.0, 9.0));
        scene.create_ik(lower, goal, 2).unwrap();
        scene.solve_ik();
        let (_, tip) = scene.bone_segment(lower).unwrap();
        assert!(tip.distance(Vec2::new(8.0, 9.0)) < 0.05, "tip at {tip:?}");
    }

    #[test]
    fn ik_stretches_towards_unreachable_targets() {
        let mut scene = Scene::default();
        let (_, lower) = arm(&mut scene);
        let goal = target(&mut scene, Vec2::new(0.0, 100.0));
        scene.create_ik(lower, goal, 2).unwrap();
        scene.solve_ik();
        let (_, tip) = scene.bone_segment(lower).unwrap();
        assert!(tip.distance(Vec2::new(0.0, 20.0)) < 0.1);
    }

    #[test]
    fn bones_can_be_picked_and_shapes_are_not_bones() {
        let mut scene = Scene::default();
        let (upper, _) = arm(&mut scene);
        assert_eq!(scene.pick_bone(Vec2::new(5.0, 1.0), 2.0), Some(upper));
        assert_eq!(scene.pick_bone(Vec2::new(5.0, 30.0), 2.0), None);
        let goal = target(&mut scene, Vec2::ZERO);
        assert!(scene.create_ik(goal, upper, 2).is_none());
    }

    fn skinned_bar(scene: &mut Scene) -> (NodeId, NodeId, NodeId) {
        let (upper, lower) = arm(scene);
        let bar = Shape::new(Path::rect(Vec2::new(0.0, -2.0), Vec2::new(20.0, 2.0)));
        let node = scene.new_node("bar", NodeKind::Shape(bar));
        let id = scene.add(node, None);
        assert!(scene.bind_skin(id, &[upper, lower]));
        (id, upper, lower)
    }

    #[test]
    fn a_skin_at_rest_does_not_change_the_shape() {
        let mut scene = Scene::default();
        let (id, _, _) = skinned_bar(&mut scene);
        let before = scene.world_bounds(id).unwrap();
        assert!(before.min.distance(Vec2::new(0.0, -2.0)) < 1e-3 && before.max.distance(Vec2::new(20.0, 2.0)) < 1e-3);
    }

    #[test]
    fn bending_a_bone_bends_the_far_end_of_the_skin() {
        let mut scene = Scene::default();
        let (id, _, lower) = skinned_bar(&mut scene);
        scene.set_bone_world(lower, Vec2::new(10.0, 0.0), std::f32::consts::FRAC_PI_2, None);
        let bounds = scene.world_bounds(id).unwrap();
        assert!(bounds.max.y > 8.0, "far end did not bend: {bounds:?}");
        // The near end sits next to the upper bone and barely moves.
        assert!(bounds.min.x < 1.0);
    }

    #[test]
    fn auto_weights_prefer_the_nearest_bone() {
        let mut scene = Scene::default();
        let (id, _, _) = skinned_bar(&mut scene);
        let Some(Node { kind: NodeKind::Shape(shape), .. }) = scene.find(id) else { panic!("no shape") };
        let skin = shape.skin.as_ref().unwrap();
        let first = &skin.weights[0][0]; // anchor at x = 0, next to the upper bone
        let last = &skin.weights[0][2]; // anchor at x = 20, next to the lower bone's tip
        assert!(first[0] > first[1] && last[1] > last[0]);
        assert!((first.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn painting_moves_weight_and_keeps_the_sum_at_one() {
        let mut scene = Scene::default();
        let (id, _, _) = skinned_bar(&mut scene);
        assert!(scene.paint_skin(id, Vec2::new(0.0, 2.0), 5.0, 1, 1.0));
        let Some(Node { kind: NodeKind::Shape(shape), .. }) = scene.find(id) else { panic!("no shape") };
        let w = &shape.skin.as_ref().unwrap().weights[0][3]; // the corner at (0, 2), under the brush
        assert!(w[1] > 0.9, "paint did not take: {w:?}");
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn unbinding_restores_the_plain_shape() {
        let mut scene = Scene::default();
        let (id, _, lower) = skinned_bar(&mut scene);
        scene.set_bone_world(lower, Vec2::new(10.0, 0.0), 1.0, None);
        assert!(scene.unbind_skin(id));
        let b = scene.world_bounds(id).unwrap();
        assert!(b.max.distance(Vec2::new(20.0, 2.0)) < 1e-3);
    }
}
