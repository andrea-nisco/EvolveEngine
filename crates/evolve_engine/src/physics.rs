//! Collision queries and lightweight 2D physics on vector outlines.
//!
//! Queries (ray casts, overlaps) work on the exact outlines of shapes. Dynamics are deliberately
//! simple: a *dynamic* body is a circle (sized from its shape) that falls, bounces and slides
//! against *static* bodies, which can have any outline, concave ones included. *Trigger* bodies
//! only report when a dynamic body enters or leaves them.

use crate::geometry::{closest_point_on_segment, ray_segment_distance, Affine2, Vec2};
use crate::path::Polyline;
use crate::scene::{Node, NodeId, NodeKind, Scene};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Outlines are flattened this finely (world units) for collision.
const OUTLINE_TOLERANCE: f32 = 0.25;
/// Impacts slower than this (units/second) do not bounce, so resting bodies stay still.
const REST_SPEED: f32 = 30.0;
const SOLVER_PASSES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    /// Never moves; dynamic bodies collide with it.
    Static,
    /// A circle moved by gravity and collisions.
    Dynamic,
    /// Does not block anything; reports overlaps.
    Trigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub kind: BodyKind,
    /// 0 = no bounce, 1 = a perfectly elastic bounce.
    pub restitution: f32,
    /// 0 = slippery, 1 = sticky.
    pub friction: f32,
    pub gravity_scale: f32,
}

impl Body {
    pub fn new(kind: BodyKind) -> Self {
        Self { kind, restitution: 0.4, friction: 0.2, gravity_scale: 1.0 }
    }
}

// --- Queries on outlines ---------------------------------------------------------------------

/// Where a ray hit an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The node hit: the body that owns the outline when `bodies_only`, else the shape.
    pub node: NodeId,
    pub point: Vec2,
    /// Unit normal of the outline, facing the ray's origin.
    pub normal: Vec2,
    pub distance: f32,
}

/// True if `p` is inside the closed `polyline` (even-odd rule).
fn polyline_contains(polyline: &Polyline, p: Vec2) -> bool {
    let mut inside = false;
    for (a, b) in polyline.edges() {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// How a circle overlaps a set of outlines: the push-out direction and depth.
fn circle_overlap(center: Vec2, radius: f32, outlines: &[Polyline]) -> Option<(Vec2, f32)> {
    let mut deepest: Option<(Vec2, f32)> = None;
    let mut consider = |normal: Vec2, depth: f32| {
        if deepest.is_none_or(|(_, d)| depth > d) {
            deepest = Some((normal, depth));
        }
    };
    for outline in outlines {
        let edges = if outline.closed { outline.points.len() } else { outline.points.len().saturating_sub(1) };
        let nearest = outline
            .edges()
            .take(edges)
            .map(|(a, b)| {
                let p = closest_point_on_segment(center, a, b);
                (p, p.distance(center), (b - a).perp())
            })
            .min_by(|x, y| x.1.total_cmp(&y.1));
        let Some((point, distance, edge_normal)) = nearest else { continue };
        if outline.closed && polyline_contains(outline, center) {
            // The center is inside the solid: leave through the nearest edge.
            let out = (point - center).try_normalize().unwrap_or_else(|| edge_normal.normalize_or_zero());
            consider(out, distance + radius);
        } else if distance < radius {
            let out = (center - point).try_normalize().unwrap_or_else(|| edge_normal.normalize_or_zero());
            consider(out, radius - distance);
        }
    }
    deepest
}

impl Scene {
    /// World-space outlines of all shapes at or below `id` (skins and symbols included).
    pub fn world_outlines(&self, id: NodeId) -> Vec<Polyline> {
        fn walk(scene: &Scene, node: &Node, world: Affine2, out: &mut Vec<Polyline>) {
            match &node.kind {
                NodeKind::Shape(shape) => {
                    out.extend(shape.effective_path(scene, world).transformed(&world).flatten(OUTLINE_TOLERANCE));
                }
                NodeKind::Group(children) => children.iter().for_each(|c| walk(scene, c, world * c.transform, out)),
                NodeKind::Instance(instance) => {
                    if let Some(symbol) = scene.symbol(instance.symbol) {
                        walk(scene, &symbol.root, world * symbol.root.transform, out);
                    }
                }
                NodeKind::Camera(_) => {}
            }
        }
        let mut outlines = Vec::new();
        if let (Some(node), Some(world)) = (self.find(id), self.world_transform(id)) {
            walk(self, node, world, &mut outlines);
        }
        outlines
    }

    /// The physics body of `id` or of its nearest ancestor that has one, with that node's id.
    pub fn body_of(&self, id: NodeId) -> Option<(NodeId, Body)> {
        let chain = self.ancestry(id)?;
        chain.iter().rev().find_map(|&n| Some((n, self.find(n)?.body?)))
    }

    /// The nearest outline crossed by a ray. `direction` need not be normalized. With `bodies_only`,
    /// only shapes that belong to a physics body count. Visible shapes only; symbol instances are skipped.
    pub fn raycast(&self, origin: Vec2, direction: Vec2, max_distance: f32, bodies_only: bool) -> Option<RayHit> {
        let direction = direction.try_normalize()?;
        let mut best: Option<RayHit> = None;
        for id in self.all_ids() {
            let Some(node) = self.find(id) else { continue };
            if !matches!(node.kind, NodeKind::Shape(_)) || !self.is_shown(id) {
                continue;
            }
            let owner = match (bodies_only, self.body_of(id)) {
                (true, None) => continue,
                (true, Some((body_node, _))) => body_node,
                (false, _) => id,
            };
            for outline in self.world_outlines(id) {
                let edges = if outline.closed { outline.points.len() } else { outline.points.len().saturating_sub(1) };
                for (a, b) in outline.edges().take(edges) {
                    let Some(distance) = ray_segment_distance(origin, direction, a, b) else { continue };
                    if distance > max_distance || best.is_some_and(|h| h.distance <= distance) {
                        continue;
                    }
                    let mut normal = (b - a).perp().normalize_or_zero();
                    if normal.dot(direction) > 0.0 {
                        normal = -normal;
                    }
                    best = Some(RayHit { node: owner, point: origin + direction * distance, normal, distance });
                }
            }
        }
        best
    }

    /// True if the outlines of the shapes under `a` and `b` touch or one contains the other.
    pub fn outlines_overlap(&self, a: NodeId, b: NodeId) -> bool {
        let (oa, ob) = (self.world_outlines(a), self.world_outlines(b));
        let segments = |o: &Polyline| -> Vec<(Vec2, Vec2)> {
            let n = if o.closed { o.points.len() } else { o.points.len().saturating_sub(1) };
            o.edges().take(n).collect()
        };
        for (pa, pb) in oa.iter().flat_map(|x| ob.iter().map(move |y| (x, y))) {
            let (sa, sb) = (segments(pa), segments(pb));
            let crossing = sa.iter().any(|&(p, q)| sb.iter().any(|&(r, s)| segments_intersect(p, q, r, s)));
            let contained = pa.points.first().is_some_and(|&p| pb.closed && polyline_contains(pb, p))
                || pb.points.first().is_some_and(|&p| pa.closed && polyline_contains(pa, p));
            if crossing || contained {
                return true;
            }
        }
        false
    }

    /// True if the node exists and it and all its ancestors are visible.
    pub fn is_shown(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|chain| chain.iter().all(|&n| self.find(n).is_some_and(|node| node.visible)))
    }

    /// Moves a node by `delta` in world space, whatever its parents' transforms are.
    pub fn translate_world(&mut self, id: NodeId, delta: Vec2) {
        let (Some(world), parent_inverse) = (self.world_transform(id), self.parent_world(id).inverse()) else { return };
        if let Some(node) = self.find_mut(id) {
            node.transform = parent_inverse * Affine2::from_translation(delta) * world;
        }
    }
}

/// True if segment `p`-`q` crosses segment `r`-`s`.
pub fn segments_intersect(p: Vec2, q: Vec2, r: Vec2, s: Vec2) -> bool {
    let (d1, d2) = (q - p, s - r);
    let denominator = d1.perp_dot(d2);
    if denominator.abs() < 1e-12 {
        return false;
    }
    let t = (r - p).perp_dot(d2) / denominator;
    let u = (r - p).perp_dot(d1) / denominator;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

// --- Dynamics --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactKind {
    Begin,
    End,
}

/// A contact that started or ended during a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    /// The dynamic body.
    pub a: NodeId,
    /// What it touched (a static body, another dynamic body or a trigger).
    pub b: NodeId,
    pub kind: ContactKind,
    pub trigger: bool,
}

/// Velocities and contact bookkeeping for a running game.
#[derive(Clone, Debug)]
pub struct PhysicsWorld {
    /// Units per second squared; +y is down.
    pub gravity: Vec2,
    velocities: HashMap<NodeId, Vec2>,
    touching: HashSet<(NodeId, NodeId, bool)>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self { gravity: Vec2::new(0.0, 900.0), velocities: HashMap::new(), touching: HashSet::new() }
    }
}

struct Dynamic {
    id: NodeId,
    body: Body,
    center: Vec2,
    radius: f32,
    velocity: Vec2,
}

impl PhysicsWorld {
    pub fn velocity(&self, id: NodeId) -> Vec2 {
        self.velocities.get(&id).copied().unwrap_or(Vec2::ZERO)
    }

    pub fn set_velocity(&mut self, id: NodeId, velocity: Vec2) {
        self.velocities.insert(id, velocity);
    }

    /// Adds to the velocity (masses are all equal to 1).
    pub fn add_impulse(&mut self, id: NodeId, impulse: Vec2) {
        *self.velocities.entry(id).or_default() += impulse;
    }

    /// Advances all dynamic bodies by `dt` seconds and returns the contacts that began or ended.
    pub fn step(&mut self, scene: &mut Scene, dt: f32) -> Vec<Contact> {
        let ids = scene.all_ids();
        let bodies: Vec<(NodeId, Body)> = ids
            .iter()
            .filter_map(|&id| scene.find(id).filter(|n| n.visible).and_then(|n| n.body.map(|b| (id, b))))
            .collect();
        let obstacles: Vec<(NodeId, BodyKind, Vec<Polyline>)> = bodies
            .iter()
            .filter(|(_, b)| b.kind != BodyKind::Dynamic)
            .map(|&(id, b)| (id, b.kind, scene.world_outlines(id)))
            .collect();

        let mut dynamics: Vec<Dynamic> = bodies
            .iter()
            .filter(|(_, b)| b.kind == BodyKind::Dynamic)
            .filter_map(|&(id, body)| {
                let bounds = scene.world_bounds(id)?;
                let radius = (bounds.size().x + bounds.size().y) * 0.25;
                let velocity = self.velocity(id) + self.gravity * body.gravity_scale * dt;
                Some(Dynamic { id, body, center: bounds.center() + velocity * dt, radius, velocity })
            })
            .collect();
        let starts: Vec<Vec2> = dynamics.iter().map(|d| d.center - d.velocity * dt).collect();

        let mut now: HashSet<(NodeId, NodeId, bool)> = HashSet::new();
        for d in &mut dynamics {
            for _ in 0..SOLVER_PASSES {
                for (other, kind, outlines) in &obstacles {
                    let Some((normal, depth)) = circle_overlap(d.center, d.radius, outlines) else { continue };
                    if *kind == BodyKind::Trigger {
                        now.insert((d.id, *other, true));
                        continue;
                    }
                    d.center += normal * depth;
                    bounce(d, normal);
                    now.insert((d.id, *other, false));
                }
            }
        }
        resolve_dynamic_pairs(&mut dynamics, &mut now);

        for (d, start) in dynamics.iter().zip(&starts) {
            self.velocities.insert(d.id, d.velocity);
            scene.translate_world(d.id, d.center - *start);
        }

        let mut contacts: Vec<Contact> = now
            .difference(&self.touching)
            .map(|&(a, b, trigger)| Contact { a, b, kind: ContactKind::Begin, trigger })
            .chain(self.touching.difference(&now).map(|&(a, b, trigger)| Contact { a, b, kind: ContactKind::End, trigger }))
            .collect();
        contacts.sort_by_key(|c| (c.a, c.b));
        self.touching = now;
        contacts
    }
}

/// Reflects the velocity of `d` off a surface with the given normal, applying restitution and friction.
fn bounce(d: &mut Dynamic, normal: Vec2) {
    let into_surface = d.velocity.dot(normal);
    if into_surface >= 0.0 {
        return;
    }
    let restitution = if -into_surface < REST_SPEED { 0.0 } else { d.body.restitution };
    let tangent = d.velocity - normal * into_surface;
    d.velocity = tangent * (1.0 - d.body.friction.clamp(0.0, 1.0) * 0.05) - normal * into_surface * restitution;
}

/// Circle-circle collisions between dynamic bodies (equal masses).
fn resolve_dynamic_pairs(dynamics: &mut [Dynamic], now: &mut HashSet<(NodeId, NodeId, bool)>) {
    for i in 0..dynamics.len() {
        for j in i + 1..dynamics.len() {
            let (left, right) = dynamics.split_at_mut(j);
            let (a, b) = (&mut left[i], &mut right[0]);
            let offset = b.center - a.center;
            let reach = a.radius + b.radius;
            let distance = offset.length();
            if distance >= reach {
                continue;
            }
            let normal = if distance > 1e-6 { offset / distance } else { Vec2::X };
            let push = (reach - distance) * 0.5;
            a.center -= normal * push;
            b.center += normal * push;
            let approach = (b.velocity - a.velocity).dot(normal);
            if approach < 0.0 {
                let restitution = (a.body.restitution + b.body.restitution) * 0.5;
                let impulse = -(1.0 + restitution) * approach * 0.5;
                a.velocity -= normal * impulse;
                b.velocity += normal * impulse;
            }
            now.insert((a.id, b.id, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path;
    use crate::scene::Shape;

    fn add_shape(scene: &mut Scene, path: Path, body: Option<Body>) -> NodeId {
        let mut node = scene.new_node("shape", NodeKind::Shape(Shape::new(path)));
        node.body = body;
        scene.add(node, None)
    }

    fn floor_and_ball(restitution: f32) -> (Scene, NodeId, NodeId) {
        let mut scene = Scene::default();
        let floor = add_shape(&mut scene, Path::rect(Vec2::new(-100.0, 100.0), Vec2::new(300.0, 140.0)), Some(Body::new(BodyKind::Static)));
        let mut body = Body::new(BodyKind::Dynamic);
        body.restitution = restitution;
        let ball = add_shape(&mut scene, Path::ellipse(Vec2::new(50.0, 0.0), Vec2::splat(10.0)), Some(body));
        (scene, floor, ball)
    }

    fn simulate(scene: &mut Scene, world: &mut PhysicsWorld, seconds: f32) -> Vec<Contact> {
        let mut all = Vec::new();
        for _ in 0..(seconds * 60.0) as usize {
            all.extend(world.step(scene, 1.0 / 60.0));
        }
        all
    }

    #[test]
    fn a_ball_falls_and_comes_to_rest_on_the_floor() {
        let (mut scene, _, ball) = floor_and_ball(0.0);
        let mut world = PhysicsWorld::default();
        simulate(&mut scene, &mut world, 3.0);
        let center = scene.world_bounds(ball).unwrap().center();
        assert!((center.y - 90.0).abs() < 1.5, "rests on the floor top (y=100) with radius 10, got {center:?}");
        assert!(world.velocity(ball).length() < 20.0);
    }

    #[test]
    fn a_bouncy_ball_rebounds_upwards() {
        let (mut scene, _, ball) = floor_and_ball(0.8);
        let mut world = PhysicsWorld::default();
        let mut rebounded = false;
        for _ in 0..180 {
            world.step(&mut scene, 1.0 / 60.0);
            rebounded |= world.velocity(ball).y < -100.0;
        }
        assert!(rebounded);
    }

    #[test]
    fn solid_contacts_are_reported_when_they_begin() {
        let (mut scene, floor, ball) = floor_and_ball(0.0);
        let mut world = PhysicsWorld::default();
        let contacts = simulate(&mut scene, &mut world, 2.0);
        let begins = contacts.iter().filter(|c| c.kind == ContactKind::Begin && c.a == ball && c.b == floor && !c.trigger).count();
        assert_eq!(begins, 1, "a resting ball keeps its contact: {contacts:?}");
    }

    #[test]
    fn a_trigger_reports_enter_and_exit() {
        let mut scene = Scene::default();
        let zone = add_shape(&mut scene, Path::rect(Vec2::new(0.0, 50.0), Vec2::new(100.0, 100.0)), Some(Body::new(BodyKind::Trigger)));
        let mut body = Body::new(BodyKind::Dynamic);
        body.gravity_scale = 0.0;
        let ball = add_shape(&mut scene, Path::ellipse(Vec2::new(50.0, 0.0), Vec2::splat(5.0)), Some(body));
        let mut world = PhysicsWorld::default();
        world.set_velocity(ball, Vec2::new(0.0, 60.0)); // down through the zone
        let contacts = simulate(&mut scene, &mut world, 3.0);
        let triggers: Vec<_> = contacts.iter().filter(|c| c.trigger && c.b == zone).map(|c| c.kind).collect();
        assert_eq!(triggers, vec![ContactKind::Begin, ContactKind::End]);
    }

    #[test]
    fn two_dynamic_balls_bounce_off_each_other() {
        let mut scene = Scene::default();
        let mut body = Body::new(BodyKind::Dynamic);
        body.gravity_scale = 0.0;
        body.restitution = 1.0;
        let left = add_shape(&mut scene, Path::ellipse(Vec2::new(0.0, 0.0), Vec2::splat(10.0)), Some(body));
        let right = add_shape(&mut scene, Path::ellipse(Vec2::new(60.0, 0.0), Vec2::splat(10.0)), Some(body));
        let mut world = PhysicsWorld::default();
        world.set_velocity(left, Vec2::new(100.0, 0.0));
        simulate(&mut scene, &mut world, 1.0);
        assert!(world.velocity(right).x > 50.0 && world.velocity(left).x < 50.0);
    }

    #[test]
    fn a_ball_cannot_fall_through_a_thin_concave_wall() {
        // A U-shaped trough: the ball must stay inside it.
        let mut scene = Scene::default();
        let trough = Path {
            subpaths: vec![crate::path::SubPath {
                anchors: [(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0)]
                    .iter()
                    .map(|&(x, y)| crate::path::Anchor::corner(Vec2::new(x, y)))
                    .collect(),
                closed: false,
            }],
        };
        add_shape(&mut scene, trough, Some(Body::new(BodyKind::Static)));
        let ball = add_shape(&mut scene, Path::ellipse(Vec2::new(50.0, 20.0), Vec2::splat(8.0)), Some(Body::new(BodyKind::Dynamic)));
        let mut world = PhysicsWorld::default();
        simulate(&mut scene, &mut world, 3.0);
        let center = scene.world_bounds(ball).unwrap().center();
        assert!(center.y < 100.0 && center.x > 0.0 && center.x < 100.0, "ball escaped: {center:?}");
    }

    #[test]
    fn rays_hit_the_nearest_outline_and_report_the_normal() {
        let mut scene = Scene::default();
        let near = add_shape(&mut scene, Path::rect(Vec2::new(20.0, -5.0), Vec2::new(30.0, 5.0)), None);
        add_shape(&mut scene, Path::rect(Vec2::new(60.0, -5.0), Vec2::new(70.0, 5.0)), None);
        let hit = scene.raycast(Vec2::ZERO, Vec2::new(2.0, 0.0), 1000.0, false).unwrap();
        assert_eq!(hit.node, near);
        assert!((hit.distance - 20.0).abs() < 1e-3);
        assert!(hit.normal.distance(Vec2::new(-1.0, 0.0)) < 1e-3);
        assert!(scene.raycast(Vec2::ZERO, Vec2::new(2.0, 0.0), 10.0, false).is_none());
        assert!(scene.raycast(Vec2::ZERO, Vec2::new(0.0, 1.0), 100.0, false).is_none());
    }

    #[test]
    fn rays_can_be_limited_to_physics_bodies() {
        let mut scene = Scene::default();
        add_shape(&mut scene, Path::rect(Vec2::new(20.0, -5.0), Vec2::new(30.0, 5.0)), None);
        let wall = add_shape(&mut scene, Path::rect(Vec2::new(60.0, -5.0), Vec2::new(70.0, 5.0)), Some(Body::new(BodyKind::Static)));
        let hit = scene.raycast(Vec2::ZERO, Vec2::X, 1000.0, true).unwrap();
        assert_eq!(hit.node, wall);
    }

    #[test]
    fn outline_overlap_detects_crossing_and_containment_but_not_gaps() {
        let mut scene = Scene::default();
        let a = add_shape(&mut scene, Path::rect(Vec2::ZERO, Vec2::splat(10.0)), None);
        let crossing = add_shape(&mut scene, Path::rect(Vec2::new(5.0, 5.0), Vec2::new(15.0, 15.0)), None);
        let inside = add_shape(&mut scene, Path::rect(Vec2::new(2.0, 2.0), Vec2::new(4.0, 4.0)), None);
        let apart = add_shape(&mut scene, Path::rect(Vec2::new(20.0, 0.0), Vec2::new(30.0, 10.0)), None);
        assert!(scene.outlines_overlap(a, crossing));
        assert!(scene.outlines_overlap(a, inside));
        assert!(!scene.outlines_overlap(a, apart));
    }
}
