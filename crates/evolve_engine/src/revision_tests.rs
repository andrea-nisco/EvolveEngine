//! Tests of change tracking (`revision.rs`): every way of changing a scene must change the stamps of what it changed.
//!
//! The audit works from the content, not from the code: it fingerprints every node by kind of change (geometry,
//! transform, paint, structure, data, exactly as the table in `revision.rs`), runs an operation, and fails if the
//! fingerprint of a node changed while its stamp did not, or if the scene's own stamps missed it. So a new way of
//! changing a scene that forgets its stamps is caught by adding it to the list below, and a bump that is removed from
//! an existing operation fails the test of that operation.

use crate::animation::{NodePose, Pose};
use crate::boolean::BoolOp;
use crate::geometry::{Affine2, Vec2};
use crate::path::Path;
use crate::physics::{Body, BodyKind};
use crate::revision::{Changes, Revs, Stamp};
use crate::runtime::{Input, Runtime};
use crate::scene::{Node, NodeId, NodeKind, Scene, Shape};
use crate::style::{Color, Paint, Stroke};
use std::collections::HashMap;

const KINDS: [Changes; 5] = [Changes::GEOMETRY, Changes::TRANSFORM, Changes::PAINT, Changes::STRUCTURE, Changes::DATA];

/// What a node holds for each kind of change, as text. Must follow the table in `revision.rs`.
fn fingerprint(node: &Node) -> [String; 5] {
    let geometry = match &node.kind {
        NodeKind::Shape(shape) => format!(
            "{:?}",
            (&shape.path, shape.fill_rule, &shape.skin, shape.fill.is_some(), shape.stroke.as_ref().map(|s| (s.join, s.cap, s.miter_limit, s.width.is_finite() && s.width > 0.0)))
        ),
        NodeKind::Camera(camera) => format!("{:?}", camera.size),
        NodeKind::Group(_) | NodeKind::Instance(_) => String::new(),
    };
    let paint = match &node.kind {
        NodeKind::Shape(shape) => format!("{:?}", (&shape.fill, shape.stroke.as_ref().map(|s| (&s.paint, s.width)))),
        NodeKind::Instance(instance) => format!("{:?}", (instance.fill, instance.stroke)),
        NodeKind::Camera(camera) => format!("{:?}", camera.background),
        NodeKind::Group(_) => String::new(),
    };
    let structure = match &node.kind {
        NodeKind::Group(children) => format!("group {:?} clip {}", children.iter().map(|c| c.id).collect::<Vec<_>>(), node.clip),
        NodeKind::Instance(instance) => format!("instance of {:?} clip {}", instance.symbol, node.clip),
        NodeKind::Shape(_) => format!("shape clip {}", node.clip),
        NodeKind::Camera(_) => format!("camera clip {}", node.clip),
    };
    [
        geometry,
        format!("{:?}", node.transform),
        format!("{:?} {:?} {}", paint, node.opacity, node.visible),
        structure,
        format!("{:?}", (&node.name, node.locked, &node.script, node.body, node.bone)),
    ]
}

/// Every node of a scene (and of its symbols' artwork) with its stamps and its fingerprints.
struct Snapshot {
    nodes: HashMap<(u64, u64), (Revs, [String; 5])>,
    order: Vec<(u64, u64)>,
    scene: Revs,
}

impl Snapshot {
    fn take(scene: &Scene) -> Snapshot {
        fn walk(node: &Node, symbol: u64, out: &mut Snapshot) {
            let key = (symbol, node.id.0);
            out.nodes.insert(key, (node.revs.0, fingerprint(node)));
            out.order.push(key);
            node.children().iter().for_each(|child| walk(child, symbol, out));
        }
        let mut snapshot = Snapshot { nodes: HashMap::new(), order: Vec::new(), scene: scene.revision() };
        scene.nodes.iter().for_each(|node| walk(node, 0, &mut snapshot));
        for symbol in &scene.symbols {
            walk(&symbol.root, symbol.id.0, &mut snapshot);
        }
        snapshot
    }

    /// Checks `scene` against what was taken before: content that changed must have changed its stamp, in the node and
    /// in the scene.
    fn verify(&self, scene: &Scene) -> Result<Changes, String> {
        let after = Snapshot::take(scene);
        let mut content_changed = Changes::NONE;
        for (key, (revs_before, print_before)) in &self.nodes {
            let Some((revs_after, print_after)) = after.nodes.get(key) else { continue };
            for (i, kind) in KINDS.iter().enumerate() {
                if print_before[i] != print_after[i] {
                    content_changed = content_changed | *kind;
                    if !revs_after.changed_since(revs_before).contains(*kind) {
                        return Err(format!(
                            "node {key:?}: its {kind:?} content changed but its stamp did not\n  before: {}\n  after:  {}",
                            print_before[i], print_after[i]
                        ));
                    }
                }
            }
        }
        // One stamp must never stand for two different contents, in any two nodes, old or new (a renderer may key a
        // cache by the stamp alone). A copy that changes something must therefore take a new stamp for it. Structure
        // is left out: it lists child ids, which differ in a copy, so it is only compared for the same node.
        for (i, kind) in KINDS.iter().enumerate().filter(|(_, kind)| **kind != Changes::STRUCTURE) {
            let mut seen: HashMap<Stamp, (&String, (u64, u64))> = HashMap::new();
            for (key, (revs, print)) in self.nodes.iter().chain(after.nodes.iter()) {
                let stamp = match i {
                    0 => revs.geometry,
                    1 => revs.transform,
                    2 => revs.paint,
                    _ => revs.data,
                };
                if let Some((other_print, other_key)) = seen.insert(stamp, (&print[i], *key)) {
                    if other_print != &print[i] {
                        return Err(format!(
                            "nodes {other_key:?} and {key:?} share one {kind:?} stamp but differ in content
  {other_print}
  {}",
                            print[i]
                        ));
                    }
                }
            }
        }
        if self.order != after.order {
            content_changed = content_changed | Changes::STRUCTURE;
        }
        let scene_changes = after.scene.changed_since(&self.scene);
        if !scene_changes.contains(content_changed) {
            return Err(format!("the scene's stamps show {scene_changes:?} but the content changed in {content_changed:?}"));
        }
        Ok(scene_changes)
    }
}

/// Runs `operation` and checks that every change it made was stamped. Returns the kinds of change the scene reports.
fn audited(scene: &mut Scene, what: &str, operation: impl FnOnce(&mut Scene)) -> Changes {
    let before = Snapshot::take(scene);
    operation(scene);
    before.verify(scene).unwrap_or_else(|error| panic!("{what}: {error}"))
}

fn shape_at(scene: &mut Scene, name: &str, at: Vec2, size: f32) -> NodeId {
    let mut shape = Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(size)));
    shape.fill = Some(Paint::linear(Vec2::ZERO, Vec2::new(size, 0.0), Color::rgb(255, 0, 0), Color::rgb(0, 0, 255)));
    shape.stroke = Some(Stroke::solid(Color::rgb(10, 20, 30), 2.0));
    let mut node = scene.new_node(name, NodeKind::Shape(shape));
    node.transform = Affine2::from_translation(at);
    scene.add(node, None)
}

struct World {
    scene: Scene,
    a: NodeId,
    b: NodeId,
    c: NodeId,
    group: NodeId,
    inner: NodeId,
    upper: NodeId,
    lower: NodeId,
    skinned: NodeId,
    target: NodeId,
}

/// A scene with a camera, nested groups, shapes with gradients and strokes, a two-bone arm with a skinned shape and an
/// IK target, so that every kind of operation has something to work on.
fn world() -> World {
    let mut scene = Scene::with_camera(Vec2::new(800.0, 600.0));
    let a = shape_at(&mut scene, "a", Vec2::new(10.0, 10.0), 20.0);
    let b = shape_at(&mut scene, "b", Vec2::new(60.0, 10.0), 20.0);
    let c = shape_at(&mut scene, "c", Vec2::new(110.0, 10.0), 20.0);
    let group = scene.group(&[b, c]).expect("a group");
    let inner_shape = shape_at(&mut scene, "inner", Vec2::new(30.0, 60.0), 15.0);
    let inner = scene.group(&[inner_shape]).expect("a group");
    scene.move_node(inner, Some(group), usize::MAX);

    let upper = scene.add_bone(None, Vec2::new(200.0, 300.0), 0.0, 40.0);
    let lower = scene.add_bone(Some(upper), Vec2::new(240.0, 300.0), 0.0, 40.0);
    let outline = [(200.0, 295.0), (240.0, 295.0), (280.0, 295.0), (280.0, 305.0), (240.0, 305.0), (200.0, 305.0)];
    let anchors = outline.iter().map(|&(x, y)| crate::path::Anchor::corner(Vec2::new(x, y))).collect();
    let arm = Shape::new(Path { subpaths: vec![crate::path::SubPath { anchors, closed: true }] });
    let skinned = scene.new_node("arm", NodeKind::Shape(arm));
    let skinned = scene.add(skinned, None);
    assert!(scene.bind_skin(skinned, &[upper, lower]));
    let marker = scene.new_node("target", NodeKind::Group(Vec::new()));
    let target = scene.add(marker, None);
    World { scene, a, b, c, group, inner, upper, lower, skinned, target }
}

// --- What the stamps promise ----------------------------------------------------------------

#[test]
fn reading_a_scene_changes_nothing() {
    let w = world();
    let before = w.scene.revision();
    let _ = (w.scene.find(w.a), w.scene.world_transform(w.inner), w.scene.world_bounds(w.group), w.scene.hit_test(Vec2::new(15.0, 15.0), 2.0));
    let _ = (w.scene.all_ids(), w.scene.camera_view(), w.scene.selection_bounds(&[w.a, w.b]), w.scene.top_level_selection(&[w.a, w.group]));
    let _ = crate::render::to_skia_path(&Path::rect(Vec2::ZERO, Vec2::ONE));
    assert_eq!(w.scene.changed_since(&before), Changes::NONE);
    // Looking at a node through the guard is free too; only writing through it counts.
    let mut scene = w.scene;
    let before = scene.revision();
    {
        let node = scene.find_mut(w.a).unwrap();
        let _ = (node.opacity, node.transform, node.name.len());
    }
    assert_eq!(scene.changed_since(&before), Changes::NONE);
}

#[test]
fn precise_setters_stamp_only_what_they_change() {
    let mut w = world();
    let id = w.a;
    let kinds = audited(&mut w.scene, "set_transform", |s| s.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(5.0, 5.0))));
    assert_eq!(kinds, Changes::TRANSFORM);
    let kinds = audited(&mut w.scene, "set_opacity", |s| s.find_mut(id).unwrap().set_opacity(0.5));
    assert_eq!(kinds, Changes::PAINT);
    let kinds = audited(&mut w.scene, "set_visible", |s| s.find_mut(id).unwrap().set_visible(false));
    assert_eq!(kinds, Changes::PAINT);
    let kinds = audited(&mut w.scene, "name_mut", |s| *s.find_mut(id).unwrap().name_mut() = "renamed".into());
    assert_eq!(kinds, Changes::DATA);
    let kinds = audited(&mut w.scene, "locked_mut", |s| *s.find_mut(id).unwrap().locked_mut() = true);
    assert_eq!(kinds, Changes::DATA);
    let kinds = audited(&mut w.scene, "set_body", |s| s.find_mut(id).unwrap().set_body(Some(Body::new(BodyKind::Static))));
    assert_eq!(kinds, Changes::DATA);
    let kinds = audited(&mut w.scene, "set_script", |s| s.find_mut(id).unwrap().set_script(Some("fn on_update(me, dt) {}".into())));
    assert_eq!(kinds, Changes::DATA);
    let kinds = audited(&mut w.scene, "path_mut", |s| {
        s.find_mut(id).unwrap().path_mut().unwrap().subpaths[0].anchors[0].pos = Vec2::new(-4.0, -4.0);
    });
    assert_eq!(kinds, Changes::GEOMETRY);
    let kinds = audited(&mut w.scene, "set_clip", |s| s.find_mut(w.group).unwrap().set_clip(true));
    assert_eq!(kinds, Changes::STRUCTURE);
    let kinds = audited(&mut w.scene, "bone_mut", |s| s.find_mut(w.upper).unwrap().bone_mut().unwrap().length = 55.0);
    assert_eq!(kinds, Changes::GEOMETRY | Changes::DATA);
}

#[test]
fn solid_color_setters_only_touch_paint_and_refuse_gradients() {
    let mut w = world();
    let id = w.a; // a gradient fill and a solid stroke
    let before = w.scene.revision();
    assert!(!w.scene.find_mut(id).unwrap().set_solid_fill_color(Color::BLACK), "the fill is a gradient");
    assert_eq!(w.scene.changed_since(&before), Changes::NONE, "a refused change stamps nothing");
    let kinds = audited(&mut w.scene, "set_solid_stroke_color", |s| assert!(s.find_mut(id).unwrap().set_solid_stroke_color(Color::rgb(1, 2, 3))));
    assert_eq!(kinds, Changes::PAINT);
    let kinds = audited(&mut w.scene, "set_solid_fill_color", |s| {
        s.find_mut(id).unwrap().kind = NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::ONE))); // a solid fill now
        assert!(s.find_mut(id).unwrap().set_solid_fill_color(Color::rgb(9, 9, 9)));
    });
    assert!(kinds.contains(Changes::PAINT));
}

#[test]
fn the_width_of_a_stroke_is_paint_while_it_stays_visible_and_geometry_when_it_appears_or_vanishes() {
    let mut w = world();
    let id = w.a; // a stroke of width 2
    let kinds = audited(&mut w.scene, "set_stroke_width 2 -> 5", |s| assert!(s.find_mut(id).unwrap().set_stroke_width(5.0)));
    assert_eq!(kinds, Changes::PAINT);
    let kinds = audited(&mut w.scene, "set_stroke_width 5 -> 0.25", |s| assert!(s.find_mut(id).unwrap().set_stroke_width(0.25)));
    assert_eq!(kinds, Changes::PAINT);
    let kinds = audited(&mut w.scene, "set_stroke_width -> 0", |s| assert!(s.find_mut(id).unwrap().set_stroke_width(0.0)));
    assert_eq!(kinds, Changes::PAINT | Changes::GEOMETRY, "no stroke to draw any more");
    let kinds = audited(&mut w.scene, "set_stroke_width 0 -> 3", |s| assert!(s.find_mut(id).unwrap().set_stroke_width(3.0)));
    assert_eq!(kinds, Changes::PAINT | Changes::GEOMETRY, "a stroke appears: other triangles");
    // The width it has: nothing changes, and nothing is stamped.
    let before = w.scene.revision();
    assert!(w.scene.find_mut(id).unwrap().set_stroke_width(3.0));
    assert_eq!(w.scene.changed_since(&before), Changes::NONE);
    // Refused: not a number, negative, no stroke, not a shape.
    for refused in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(!w.scene.find_mut(id).unwrap().set_stroke_width(refused), "{refused}");
    }
    let mut plain = Shape::new(Path::rect(Vec2::ZERO, Vec2::ONE));
    plain.stroke = None;
    let no_stroke = w.scene.new_node("plain", NodeKind::Shape(plain));
    let no_stroke = w.scene.add(no_stroke, None);
    let before = w.scene.revision();
    assert!(!w.scene.find_mut(no_stroke).unwrap().set_stroke_width(2.0));
    assert!(!w.scene.find_mut(w.group).unwrap().set_stroke_width(2.0));
    assert_eq!(w.scene.changed_since(&before), Changes::NONE, "a refused change stamps nothing");
    let stroke_width = |scene: &Scene| match &scene.find(id).unwrap().kind {
        NodeKind::Shape(shape) => shape.stroke.as_ref().unwrap().width,
        _ => unreachable!(),
    };
    assert_eq!(stroke_width(&w.scene), 3.0, "a refused width leaves the stroke as it was");
}

#[test]
fn writing_through_the_guard_without_a_precise_method_stamps_everything() {
    let mut w = world();
    let id = w.a;
    let kinds = audited(&mut w.scene, "node.opacity = ..", |s| s.find_mut(id).unwrap().opacity = 0.25);
    assert_eq!(kinds, Changes::ALL);
    let kinds = audited(&mut w.scene, "node.kind = ..", |s| {
        if let NodeKind::Shape(shape) = &mut s.find_mut(id).unwrap().kind {
            shape.fill_rule = crate::style::FillRule::EvenOdd;
        }
    });
    assert_eq!(kinds, Changes::ALL);
}

#[test]
fn shape_mut_stamps_geometry_and_paint_when_called() {
    let mut w = world();
    let id = w.a;
    let kinds = audited(&mut w.scene, "shape_mut", |s| s.shape_mut(id).unwrap().stroke.as_mut().unwrap().width = 9.0);
    assert_eq!(kinds, Changes::GEOMETRY | Changes::PAINT);
    let before = w.scene.revision();
    assert!(w.scene.shape_mut(w.group).is_none());
    assert_eq!(w.scene.changed_since(&before), Changes::NONE, "a group is not a shape, nothing is stamped");
}

#[test]
fn every_model_operation_stamps_what_it_changes() {
    let mut w = world();
    let (a, b, c, group, inner) = (w.a, w.b, w.c, w.group, w.inner);

    audited(&mut w.scene, "add", |s| {
        let node = s.new_node("new", NodeKind::Group(Vec::new()));
        s.add(node, None);
    });
    audited(&mut w.scene, "add into a group", |s| {
        let node = s.new_node("child", NodeKind::Group(Vec::new()));
        s.add(node, Some(group));
    });
    audited(&mut w.scene, "remove", |s| {
        let id = s.nodes.last().unwrap().id;
        assert!(s.remove(id).is_some());
    });
    // A group that is moved and scaled, so a node moved into it has to change its own transform to stay in place.
    w.scene.find_mut(group).unwrap().set_transform(Affine2::from_scale_angle_translation(Vec2::splat(2.0), 0.3, Vec2::new(50.0, 20.0)));
    let kinds = audited(&mut w.scene, "move_node into a group", |s| assert!(s.move_node(a, Some(group), 0)));
    assert!(kinds.contains(Changes::TRANSFORM | Changes::STRUCTURE), "{kinds:?}");
    audited(&mut w.scene, "move_node out of a group", |s| assert!(s.move_node(a, None, usize::MAX)));
    audited(&mut w.scene, "shift_order", |s| s.shift_order(a, false));
    let mut made = None;
    audited(&mut w.scene, "group", |s| made = s.group(&[a, b]));
    let made = made.expect("a group");
    // A group that is turned and scaled, so its children change their own transforms when it is dissolved.
    w.scene.find_mut(made).unwrap().set_transform(Affine2::from_scale_angle_translation(Vec2::splat(1.5), 0.2, Vec2::new(5.0, 9.0)));
    let kinds = audited(&mut w.scene, "ungroup", |s| assert_eq!(s.ungroup(made).len(), 2));
    assert!(kinds.contains(Changes::TRANSFORM | Changes::STRUCTURE), "{kinds:?}");
    audited(&mut w.scene, "duplicate", |s| assert!(s.duplicate(c).is_some()));
    audited(&mut w.scene, "center_origin on a shape", |s| assert!(s.center_origin(a)));
    audited(&mut w.scene, "center_origin on a group", |s| assert!(s.center_origin(inner)));
    // The compound path is made first, so the snapshot taken for the audit already holds its stamp: the copies made by
    // release_compound start from that stamp and must not keep it for their one-contour paths.
    w.scene.find_mut(a).unwrap().path_mut().unwrap().subpaths.extend(Path::ellipse(Vec2::new(50.0, 50.0), Vec2::splat(4.0)).subpaths);
    audited(&mut w.scene, "release_compound", |s| assert_eq!(s.release_compound(a).len(), 2));
    audited(&mut w.scene, "combine", |s| assert!(s.combine(&[a, c]).is_some()));

    let mut w2 = world();
    let (x, y) = (shape_at(&mut w2.scene, "x", Vec2::new(300.0, 300.0), 40.0), shape_at(&mut w2.scene, "y", Vec2::new(310.0, 310.0), 40.0));
    audited(&mut w2.scene, "boolean", |s| assert!(s.boolean(&[x, y], BoolOp::Union).is_some()));
    audited(&mut w2.scene, "make_clip_group", |s| {
        let (p, q) = (s.nodes.iter().find(|n| n.id == w2.a).map(|n| n.id).unwrap(), w2.c);
        assert!(s.make_clip_group(&[p, q]).is_some());
    });
}

#[test]
fn symbols_stamp_the_scene_and_their_nodes() {
    let mut w = world();
    let (a, b) = (w.a, w.b);
    let mut made = None;
    audited(&mut w.scene, "make_symbol", |s| made = s.make_symbol(&[a], "Box"));
    let (symbol, instance) = made.expect("a symbol");
    audited(&mut w.scene, "place_instance", |s| assert!(s.place_instance(symbol, Vec2::new(300.0, 300.0)).is_some()));
    audited(&mut w.scene, "an instance's override", |s| {
        if let NodeKind::Instance(i) = &mut s.find_mut(instance).unwrap().kind {
            i.fill = Some(Color::rgb(0, 255, 0));
        }
    });
    audited(&mut w.scene, "update_symbol", |s| assert!(s.update_symbol(symbol, &[b]).is_some()));
    audited(&mut w.scene, "symbol_copy", |s| {
        let copy = s.symbol_copy(symbol).expect("a copy of the master");
        s.add(copy, None);
    });
    audited(&mut w.scene, "detach_instance", |s| assert!(s.detach_instance(instance).is_some()));
    let remaining: Vec<NodeId> = w.scene.all_ids().into_iter().filter(|&id| matches!(w.scene.find(id).map(|n| &n.kind), Some(NodeKind::Instance(_)))).collect();
    audited(&mut w.scene, "detach the rest", |s| remaining.iter().for_each(|&id| assert!(s.detach_instance(id).is_some())));
    let kinds = audited(&mut w.scene, "rename_symbol", |s| assert!(s.rename_symbol(symbol, "Crate")));
    assert!(kinds.contains(Changes::DATA));
    let before = w.scene.revision();
    assert!(w.scene.rename_symbol(symbol, "Crate"));
    assert_eq!(w.scene.changed_since(&before), Changes::NONE, "renaming to the same name is no change");
    audited(&mut w.scene, "delete_symbol", |s| assert!(s.delete_symbol(symbol)));
}

#[test]
fn clipboard_operations_are_stamped() {
    let mut w = world();
    let (a, group) = (w.a, w.group);
    // Nodes inside a moved group, so that copying (which writes their world transform) really changes the transform.
    w.scene.find_mut(group).unwrap().set_transform(Affine2::from_translation(Vec2::new(100.0, 50.0)));
    let inner = w.inner;
    let clipboard = w.scene.copy_nodes(&[a, inner]).expect("something to copy");
    // The clipboard is not part of the scene, so the audit does not see it: check its stamps by hand.
    for (copied, id) in clipboard.nodes.iter().zip([a, inner]) {
        let original = w.scene.find(id).unwrap();
        if copied.transform != original.transform {
            assert_ne!(copied.revs.transform, original.revs.transform, "copy_nodes: the copy of {id:?} has another transform but kept its stamp");
        }
    }
    let mut pasted = Vec::new();
    audited(&mut w.scene, "paste_nodes", |s| pasted = s.paste_nodes(&clipboard, Vec2::new(30.0, 30.0)));
    assert_eq!(pasted.len(), 2);
    for (&id, copied) in pasted.iter().zip(&clipboard.nodes) {
        let node = w.scene.find(id).unwrap();
        assert_ne!(node.transform, copied.transform);
        assert_ne!(node.revs.transform, copied.revs.transform, "paste_nodes: a moved copy needs its own transform stamp");
    }

    let mut with_symbol = world();
    let (symbol, _) = with_symbol.scene.make_symbol(&[with_symbol.a], "Box").unwrap();
    let clipboard = with_symbol.scene.copy_nodes(&with_symbol.scene.nodes.iter().map(|n| n.id).collect::<Vec<_>>()).unwrap();
    let mut other = world();
    audited(&mut other.scene, "paste with symbols", |s| assert!(!s.paste_nodes(&clipboard, Vec2::ZERO).is_empty()));
    audited(&mut other.scene, "spawn a symbol", |s| {
        s.symbols.first().map(|sym| sym.name.clone()).and_then(|name| s.spawn(&name, Vec2::new(5.0, 5.0))).expect("spawned");
    });
    let _ = symbol;
    audited(&mut w.scene, "spawn a template", |s| {
        s.find_mut(a).unwrap().set_visible(false);
        assert!(s.spawn("a", Vec2::new(70.0, 70.0)).is_some());
    });
}

#[test]
fn rigging_operations_are_stamped() {
    let mut w = world();
    let (upper, lower, skinned, target) = (w.upper, w.lower, w.skinned, w.target);
    audited(&mut w.scene, "add_bone", |s| {
        s.add_bone(Some(lower), Vec2::new(280.0, 300.0), 0.0, 20.0);
    });
    let kinds = audited(&mut w.scene, "set_bone_world", |s| s.set_bone_world(upper, Vec2::new(200.0, 300.0), 0.4, None));
    assert_eq!(kinds, Changes::TRANSFORM, "turning a bone only changes its transform");
    audited(&mut w.scene, "set_bone_world with a length", |s| s.set_bone_world(upper, Vec2::new(200.0, 300.0), 0.4, Some(33.0)));
    audited(&mut w.scene, "create_ik", |s| assert!(s.create_ik(lower, target, 2).is_some()));
    // Move the target far away so that solving has to turn the bones.
    w.scene.find_mut(target).unwrap().set_transform(Affine2::from_translation(Vec2::new(210.0, 380.0)));
    let kinds = audited(&mut w.scene, "solve_ik", |s| s.solve_ik());
    assert_eq!(kinds, Changes::TRANSFORM, "solving only turns bones");
    audited(&mut w.scene, "paint_skin", |s| assert!(s.paint_skin(skinned, Vec2::new(200.0, 300.0), 60.0, 1, 0.5)));
    audited(&mut w.scene, "unbind_skin", |s| assert!(s.unbind_skin(skinned)));
    audited(&mut w.scene, "bind_skin", |s| assert!(s.bind_skin(skinned, &[upper, lower])));
    let mut copy = w.scene.clone();
    audited(&mut copy, "copy_ik_bones_to", |s| w.scene.copy_ik_bones_to(s));
}

#[test]
fn physics_and_poses_stamp_only_transforms_and_paint() {
    let mut w = world();
    let (a, b) = (w.a, w.b);
    let kinds = audited(&mut w.scene, "translate_world", |s| s.translate_world(a, Vec2::new(7.0, 0.0)));
    assert_eq!(kinds, Changes::TRANSFORM);

    let mut pose = Pose::new();
    pose.insert(a, NodePose { transform: Some(Affine2::from_translation(Vec2::new(1.0, 2.0))), opacity: Some(0.4), ..Default::default() });
    let kinds = audited(&mut w.scene, "apply_pose: transform and opacity", |s| s.apply_pose(&pose));
    assert_eq!(kinds, Changes::TRANSFORM | Changes::PAINT, "a pose that moves things must not make a renderer rebuild shapes");

    let mut pose = Pose::new();
    pose.insert(b, NodePose { stroke: Some(Color::rgb(7, 7, 7)), fill: Some(Color::rgb(8, 8, 8)), ..Default::default() });
    let kinds = audited(&mut w.scene, "apply_pose: colors", |s| s.apply_pose(&pose));
    assert!(!kinds.intersects(Changes::GEOMETRY | Changes::TRANSFORM | Changes::STRUCTURE), "{kinds:?}");

    let mut pose = Pose::new();
    pose.insert(b, NodePose { path: Some(Path::rect(Vec2::ZERO, Vec2::splat(33.0))), ..Default::default() });
    let kinds = audited(&mut w.scene, "apply_pose: a morphed path", |s| s.apply_pose(&pose));
    assert!(kinds.contains(Changes::GEOMETRY));
}

#[test]
fn a_running_game_stamps_what_scripts_and_physics_change() {
    let mut scene = Scene::default();
    let mut shape = Shape::new(Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)));
    shape.fill = Some(Paint::Solid(Color::rgb(255, 0, 0)));
    shape.stroke = Some(Stroke::solid(Color::BLACK, 1.0));
    let mut node = scene.new_node("actor", NodeKind::Shape(shape));
    node.script = Some(
        "fn on_update(me, dt) { move_by(me, 10.0 * dt, 0); set_rotation(me, 30.0); set_scale(me, 1.5, 1.5); \
         set_opacity(me, 0.5); set_visible(me, true); set_fill(me, 1, 2, 3); set_stroke(me, 4, 5, 6); set_stroke_width(me, 2.5); }"
            .into(),
    );
    let id = scene.add(node, None);
    let mut ball = scene.new_node("ball", NodeKind::Shape(Shape::new(Path::ellipse(Vec2::new(0.0, -50.0), Vec2::splat(5.0)))));
    ball.body = Some(Body::new(BodyKind::Dynamic));
    let ball = scene.add(ball, None);

    let mut runtime = Runtime::new(&scene);
    let before = Snapshot::take(&runtime.scene());
    for _ in 0..10 {
        runtime.update(1.0 / 60.0, Input::default());
    }
    let after = runtime.scene();
    let kinds = before.verify(&after).unwrap_or_else(|error| panic!("running the game: {error}"));
    assert!(kinds.contains(Changes::TRANSFORM | Changes::PAINT), "{kinds:?}");
    assert!(!kinds.intersects(Changes::GEOMETRY | Changes::STRUCTURE), "scripts and physics must not touch geometry or structure: {kinds:?}");
    assert!(after.find(id).unwrap().revs.transform != scene.find(id).unwrap().revs.transform);
    assert!(after.find(ball).unwrap().revs.transform != scene.find(ball).unwrap().revs.transform, "gravity moved the ball");
    assert_eq!(after.find(ball).unwrap().revs.geometry, scene.find(ball).unwrap().revs.geometry);
}

// --- Stamps, copies and comparisons -------------------------------------------------------------

#[test]
fn a_copy_has_the_same_stamps_until_one_of_them_changes() {
    let w = world();
    let copy = w.scene.clone();
    assert_eq!(copy.revision(), w.scene.revision());
    assert!(copy.same_content(&w.scene));
    let (mut left, mut right) = (w.scene.clone(), w.scene.clone());
    left.find_mut(w.a).unwrap().set_transform(Affine2::from_translation(Vec2::new(1.0, 1.0)));
    right.find_mut(w.a).unwrap().set_transform(Affine2::from_translation(Vec2::new(2.0, 2.0)));
    // Two different changes to copies of the same scene must never look like the same version.
    assert_ne!(left.find(w.a).unwrap().revs.transform, right.find(w.a).unwrap().revs.transform);
    assert_ne!(left.revision(), right.revision());
    assert_ne!(left.revision(), w.scene.revision());
    assert!(!left.same_content(&right));
}

#[test]
fn the_same_change_made_to_two_copies_is_still_the_same_content() {
    let w = world();
    let (mut left, mut right) = (w.scene.clone(), w.scene.clone());
    let moved = Affine2::from_translation(Vec2::new(3.0, 4.0));
    left.find_mut(w.a).unwrap().set_transform(moved);
    right.find_mut(w.a).unwrap().set_transform(moved);
    assert_ne!(left.revision(), right.revision(), "separate stamps, because they were made separately");
    assert!(left.same_content(&right), "but the content is equal and same_content says so, like `==`");
    assert!(left == right);
}

#[test]
fn same_content_notices_changes_outside_the_node_tree() {
    let w = world();
    let mut scene = w.scene.clone();
    assert!(scene.same_content(&w.scene));
    scene.name = "Other".into();
    assert!(!scene.same_content(&w.scene));
    let mut scene = w.scene.clone();
    scene.clips.push(crate::animation::Clip::new("Clip"));
    assert!(!scene.same_content(&w.scene), "clips are compared directly, whatever the stamps say");
    let mut scene = w.scene.clone();
    let _ = scene.create_ik(w.lower, w.target, 2);
    assert!(!scene.same_content(&w.scene));
}

#[test]
fn stamps_are_not_saved_and_do_not_take_part_in_comparing() {
    let w = world();
    let json = serde_json::to_string(&w.scene).unwrap();
    assert!(!json.contains("revs"), "stamps must not reach the project file");
    let loaded: Scene = serde_json::from_str(&json).unwrap();
    assert!(loaded == w.scene, "equal content is equal, whatever the stamps");
    assert_ne!(loaded.revision(), w.scene.revision(), "a loaded scene is a new version");
    assert!(loaded.same_content(&w.scene), "and same_content falls back to a real comparison to say so");
    assert_ne!(loaded.find(w.a).unwrap().revs.geometry, w.scene.find(w.a).unwrap().revs.geometry);
}

#[test]
fn restoring_an_old_copy_restores_its_stamps() {
    let mut w = world();
    let snapshot = w.scene.clone();
    w.scene.find_mut(w.a).unwrap().set_opacity(0.1);
    assert_ne!(w.scene.revision(), snapshot.revision());
    let changed_in = w.scene.changed_since(&snapshot.revision());
    assert_eq!(changed_in, Changes::PAINT);
    let newer = w.scene.clone();
    w.scene = snapshot.clone(); // an undo
    assert_eq!(w.scene.revision(), snapshot.revision());
    assert_eq!(w.scene.changed_since(&newer.revision()), Changes::PAINT, "going back is seen as a change of the same kind");
    assert_eq!(w.scene.find(w.a).unwrap().revs, snapshot.find(w.a).unwrap().revs);
}

#[test]
fn stamps_are_unique_across_threads() {
    let handles: Vec<_> = (0..4).map(|_| std::thread::spawn(|| (0..1000).map(|_| crate::revision::Stamp::fresh()).collect::<Vec<_>>())).collect();
    let mut all: Vec<_> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
    let count = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), count);
}

/// The audit itself must be able to fail: a change that skips its stamp is reported.
#[test]
fn the_audit_catches_a_write_that_skips_the_stamps() {
    let mut w = world();
    let a = w.a;
    for (what, write) in [
        ("transform", Box::new(move |s: &mut Scene| s.nodes.iter_mut().find(|n| n.id == a).unwrap().transform = Affine2::from_translation(Vec2::splat(99.0))) as Box<dyn FnOnce(&mut Scene)>),
        ("name", Box::new(move |s: &mut Scene| s.nodes.iter_mut().find(|n| n.id == a).unwrap().name = "sneaky".into())),
        ("opacity", Box::new(move |s: &mut Scene| s.nodes.iter_mut().find(|n| n.id == a).unwrap().opacity = 0.3)),
        ("path", Box::new(move |s: &mut Scene| {
            if let NodeKind::Shape(shape) = &mut s.nodes.iter_mut().find(|n| n.id == a).unwrap().kind {
                shape.path.subpaths[0].anchors[0].pos = Vec2::splat(-50.0);
            }
        })),
        ("a new top-level node", Box::new(|s: &mut Scene| {
            let node = s.new_node("sneaky", NodeKind::Group(Vec::new()));
            s.nodes.push(node);
        })),
    ] {
        let before = Snapshot::take(&w.scene);
        write(&mut w.scene);
        assert!(before.verify(&w.scene).is_err(), "a direct write to the {what} went unnoticed by the audit");
    }
}
