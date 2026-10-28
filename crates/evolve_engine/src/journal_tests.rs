//! Tests of the journal of changes (`revision.rs`, "Which nodes changed"): a renderer that has drawn a scene asks which nodes changed
//! since, and must get every one of them, or be told that the answer is not known.

use crate::geometry::{Affine2, Vec2};
use crate::path::Path;
use crate::revision::Changes;
use crate::scene::{NodeId, NodeKind, Scene, Shape};
use crate::style::{Color, Paint};

/// A scene of `n` shapes and their ids.
fn scene(n: usize) -> (Scene, Vec<NodeId>) {
    let mut scene = Scene::default();
    let ids = (0..n)
        .map(|i| {
            let mut shape = Shape::new(Path::rect(Vec2::new(i as f32, 0.0), Vec2::new(i as f32 + 1.0, 1.0)));
            shape.fill = Some(Paint::Solid(Color::rgb(10, 20, 30)));
            let node = scene.new_node(format!("s{i}"), NodeKind::Shape(shape));
            scene.add(node, None)
        })
        .collect();
    (scene, ids)
}

#[test]
fn the_journal_names_the_nodes_changed_and_what_kind_of_change_each_got() {
    let (mut scene, ids) = scene(6);
    let before = scene.revision();
    scene.find_mut(ids[1]).unwrap().set_transform(Affine2::from_translation(Vec2::new(5.0, 0.0)));
    scene.find_mut(ids[4]).unwrap().set_opacity(0.5);
    scene.find_mut(ids[1]).unwrap().set_transform(Affine2::from_translation(Vec2::new(6.0, 0.0)));
    scene.find_mut(ids[4]).unwrap().path_mut();
    let changed = scene.nodes_changed_since(&before).expect("the journal reaches back");
    assert_eq!(changed.len(), 2, "each node once, whatever the number of changes: {changed:?}");
    let kinds = |id: NodeId| changed.iter().find(|c| c.0 == id).map(|c| c.1);
    assert_eq!(kinds(ids[1]), Some(Changes::TRANSFORM));
    assert_eq!(kinds(ids[4]), Some(Changes::PAINT | Changes::GEOMETRY));
    assert_eq!(kinds(ids[0]), None, "a node nobody touched is not named");
    // Nothing since: an empty list (not "unknown").
    let now = scene.revision();
    assert_eq!(scene.nodes_changed_since(&now), Some(vec![]));
    // And only what came after the point asked about.
    scene.find_mut(ids[2]).unwrap().set_visible(false);
    assert_eq!(scene.nodes_changed_since(&now), Some(vec![(ids[2], Changes::PAINT)]));
}

#[test]
fn a_write_through_the_node_names_it_with_every_kind_and_a_change_to_the_scene_as_a_whole_is_unknown() {
    let (mut scene, ids) = scene(3);
    let before = scene.revision();
    scene.find_mut(ids[0]).unwrap().name = "renamed".into(); // a plain write: stamps everything, to be safe
    let changed = scene.nodes_changed_since(&before).unwrap();
    assert_eq!(changed, vec![(ids[0], Changes::ALL)]);
    // Adding, removing and reordering nodes change the list of children: no one node, so node 0.
    let before = scene.revision();
    let node = scene.new_node("new", NodeKind::Group(vec![]));
    scene.add(node, None);
    let changed = scene.nodes_changed_since(&before).unwrap();
    assert!(changed.iter().any(|(id, kinds)| *id == NodeId(0) && kinds.contains(Changes::STRUCTURE)), "{changed:?}");
    let before = scene.revision();
    scene.remove(ids[1]);
    let changed = scene.nodes_changed_since(&before).unwrap();
    assert!(changed.iter().any(|(_, kinds)| kinds.contains(Changes::STRUCTURE)), "{changed:?}");
}

#[test]
fn a_copy_of_a_scene_has_a_journal_of_its_own() {
    let (mut scene, ids) = scene(3);
    let before = scene.revision();
    scene.find_mut(ids[1]).unwrap().set_opacity(0.9);
    let mut copy = scene.clone();
    assert_ne!(copy.journal_epoch(), scene.journal_epoch(), "a copy must never be taken for the scene it was copied from");
    assert_eq!(copy.revision(), scene.revision(), "the same content has the same stamps");
    // The copy starts from now: what happened to the original before the copy is not in its journal.
    assert_eq!(scene.nodes_changed_since(&before), Some(vec![(ids[1], Changes::PAINT)]));
    assert_eq!(copy.nodes_changed_since(&before), None);
    let before = scene.revision();
    scene.find_mut(ids[0]).unwrap().set_transform(Affine2::from_translation(Vec2::X));
    copy.find_mut(ids[2]).unwrap().set_transform(Affine2::from_translation(Vec2::Y));
    assert_eq!(scene.nodes_changed_since(&before), Some(vec![(ids[0], Changes::TRANSFORM)]));
    let after_copy = copy.revision();
    copy.find_mut(ids[1]).unwrap().set_opacity(0.2);
    assert_eq!(copy.nodes_changed_since(&after_copy), Some(vec![(ids[1], Changes::PAINT)]));
}

#[test]
fn a_journal_that_overflowed_says_it_does_not_know_and_what_it_still_holds_is_right() {
    let (mut scene, ids) = scene(2);
    let old = scene.revision();
    for i in 0..40_000 {
        scene.find_mut(ids[0]).unwrap().set_transform(Affine2::from_translation(Vec2::new(i as f32, 0.0)));
    }
    assert_eq!(scene.nodes_changed_since(&old), None, "the oldest changes were dropped");
    let recent = scene.revision();
    scene.find_mut(ids[1]).unwrap().set_opacity(0.3);
    assert_eq!(scene.nodes_changed_since(&recent), Some(vec![(ids[1], Changes::PAINT)]));
}

// --- Cameras: nodes that draw nothing ---------------------------------------------------------------

#[test]
fn giving_a_camera_the_size_it_has_changes_nothing_and_another_size_changes_only_its_geometry() {
    let (mut scene, ids) = scene(3);
    let camera = scene.new_camera(Vec2::new(300.0, 200.0), Vec2::new(150.0, 100.0));
    let camera = scene.add(camera, None);
    let before = scene.revision();
    scene.find_mut(camera).unwrap().set_camera_size(Vec2::new(300.0, 200.0));
    assert_eq!(scene.revision(), before, "the same size: not a change");
    scene.find_mut(camera).unwrap().set_camera_size(Vec2::new(120.0, 80.0));
    assert_eq!(scene.camera_view().size, Vec2::new(120.0, 80.0));
    assert_eq!(scene.changed_since(&before), Changes::GEOMETRY);
    assert_eq!(scene.nodes_changed_since(&before), Some(vec![(camera, Changes::GEOMETRY)]));
    // A node that is not a camera is left alone.
    let before = scene.revision();
    scene.find_mut(ids[0]).unwrap().set_camera_size(Vec2::new(1.0, 1.0));
    assert_eq!(scene.revision(), before);
}

#[test]
fn only_cameras_changed_says_yes_for_camera_moves_and_no_for_anything_else_or_anything_unknown() {
    let (mut scene, ids) = scene(3);
    let camera = scene.new_camera(Vec2::new(300.0, 200.0), Vec2::new(150.0, 100.0));
    let camera = scene.add(camera, None);
    let (before, epoch) = (scene.revision(), scene.journal_epoch());
    assert!(scene.only_cameras_changed_since(&before, epoch), "nothing changed at all");
    scene.find_mut(camera).unwrap().set_transform(Affine2::from_translation(Vec2::new(10.0, 0.0)));
    scene.find_mut(camera).unwrap().set_camera_size(Vec2::new(120.0, 80.0));
    assert!(scene.only_cameras_changed_since(&before, epoch), "a camera that moved and was resized");
    // Another scene's journal says nothing about this one.
    assert!(!scene.only_cameras_changed_since(&before, crate::revision::Stamp::fresh()));
    // A shape too: the answer is no.
    scene.find_mut(ids[1]).unwrap().set_transform(Affine2::from_translation(Vec2::new(1.0, 0.0)));
    assert!(!scene.only_cameras_changed_since(&before, epoch));
    // A change that is not to one node, or to the tree, is no.
    let (mut other, _) = self::scene(2);
    let node = other.new_camera(Vec2::new(10.0, 10.0), Vec2::ZERO);
    other.add(node, None);
    let (before, epoch) = (other.revision(), other.journal_epoch());
    let node = other.new_node("late", NodeKind::Group(Vec::new()));
    other.add(node, None);
    assert!(!other.only_cameras_changed_since(&before, epoch), "a node was added");
    // A shape that became a camera is a change to what was drawn.
    let (mut other, ids) = self::scene(2);
    let (before, epoch) = (other.revision(), other.journal_epoch());
    other.find_mut(ids[0]).unwrap().kind = NodeKind::Camera(crate::game_camera::GameCamera::default());
    assert!(!other.only_cameras_changed_since(&before, epoch), "a shape was replaced by a camera");
}
