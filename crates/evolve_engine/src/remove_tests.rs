//! `Scene::remove_all`: one walk that must leave the scene a node-by-node `remove` would leave, and stamp what it changed.

use crate::geometry::Vec2;
use crate::hit_tests::{random_scene, Dice};
use crate::scene::{NodeId, NodeKind, Scene};

/// The ids of a random part of the scene's nodes, groups and shapes, nested ones included, in no particular order.
fn some_nodes(scene: &Scene, dice: &mut Dice, share: usize) -> Vec<NodeId> {
    let mut ids: Vec<NodeId> = scene.all_ids().into_iter().filter(|_| dice.below(100) < share).collect();
    // Out of document order, and with an id twice and one that is not in the scene.
    ids.reverse();
    if let Some(&first) = ids.first() {
        ids.push(first);
    }
    ids.push(NodeId(9_999_999));
    ids
}

#[test]
fn removing_many_nodes_at_once_leaves_the_scene_removing_them_one_by_one_leaves() {
    let mut dice = Dice(5);
    for round in 0..20 {
        let scene = random_scene(&mut dice);
        let ids = some_nodes(&scene, &mut dice, [1, 5, 20, 60][round % 4]);
        let mut at_once = scene.clone();
        let removed = at_once.remove_all(&ids);
        let mut one_by_one = scene.clone();
        for &id in &ids {
            one_by_one.remove(id);
        }
        assert!(at_once == one_by_one, "round {round}: not the scene that removing them one by one leaves");
        let gone = scene.all_ids().len() - at_once.all_ids().len();
        assert!(gone >= removed, "round {round}: {removed} taken out of their parents, {gone} nodes gone with what was inside them");
        assert_eq!(gone == 0, removed == 0, "round {round}");
    }
}

#[test]
fn a_node_inside_a_listed_node_goes_with_it_and_unknown_ids_are_skipped() {
    let mut scene = Scene::default();
    let inner = scene.new_node("inner", NodeKind::Shape(crate::scene::Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::ONE))));
    let inner_id = inner.id;
    let outer = scene.new_node("outer", NodeKind::Group(vec![inner]));
    let outer_id = scene.add(outer, None);
    let other = scene.new_node("other", NodeKind::Shape(crate::scene::Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::ONE))));
    let other_id = scene.add(other, None);
    assert_eq!(scene.remove_all(&[inner_id, outer_id, NodeId(777)]), 1, "the inner one went with the outer one: one taken out of its parent");
    assert_eq!(scene.all_ids(), vec![other_id]);
    assert_eq!(scene.remove_all(&[]), 0);
    assert_eq!(scene.remove_all(&[outer_id, inner_id]), 0, "nothing of them is left");
}

#[test]
fn every_parent_that_lost_a_child_is_stamped_once_and_the_journal_names_it() {
    let mut scene = Scene::default();
    let mut groups = Vec::new();
    let mut shapes = Vec::new();
    for g in 0..3 {
        let children: Vec<_> = (0..4)
            .map(|i| {
                let node = scene.new_node(format!("s{g}{i}"), NodeKind::Shape(crate::scene::Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::ONE))));
                shapes.push(node.id);
                node
            })
            .collect();
        let group = scene.new_node(format!("g{g}"), NodeKind::Group(children));
        groups.push(scene.add(group, None));
    }
    let before = scene.revision();
    let group_before = scene.find(groups[0]).unwrap().revs.structure;
    let untouched_before = scene.find(groups[2]).unwrap().revs.structure;
    // Two children of the first group and one of the second go.
    assert_eq!(scene.remove_all(&[shapes[0], shapes[1], shapes[5]]), 3);
    assert_ne!(scene.revision().structure, before.structure, "the scene's structure stamp moved");
    assert_ne!(scene.find(groups[0]).unwrap().revs.structure, group_before);
    assert_eq!(scene.find(groups[2]).unwrap().revs.structure, untouched_before, "a group that kept its children is not stamped");
    let changed = scene.nodes_changed_since(&before).expect("the journal reaches back");
    let named = |id: NodeId| changed.iter().any(|c| c.0 == id);
    assert!(named(groups[0]) && named(groups[1]) && !named(groups[2]), "{changed:?}");
    assert_eq!(scene.find(groups[0]).unwrap().children().len(), 2);
    assert_eq!(scene.find(groups[1]).unwrap().children().len(), 3);
}

#[test]
fn removing_thousands_of_nodes_is_quick() {
    let mut scene = Scene::default();
    let ids: Vec<NodeId> = (0..20_000)
        .map(|i| {
            let node = scene.new_node(format!("s{i}"), NodeKind::Shape(crate::scene::Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::ONE))));
            scene.add(node, None)
        })
        .collect();
    let started = std::time::Instant::now();
    assert_eq!(scene.remove_all(&ids[..10_000]), 10_000);
    assert_eq!(scene.all_ids().len(), 10_000);
    // One by one this took over a second in a release build (a search and a shift of the list for each); a walk takes milliseconds.
    // The limit is loose enough for a debug build on a slow machine.
    assert!(started.elapsed().as_secs_f64() < 2.0, "{:?}", started.elapsed());
}

/// The same tree: ids, names, flags and shapes equal, transforms equal to within rounding (the two ways multiply the same matrices in
/// the same order, but the test should not depend on that).
fn same_tree(a: &[crate::scene::Node], b: &[crate::scene::Node], path: &str) -> Result<(), String> {
    if a.len() != b.len() {
        return Err(format!("{path}: {} nodes against {}", a.len(), b.len()));
    }
    for (x, y) in a.iter().zip(b) {
        let here = format!("{path}/{}", x.id.0);
        if (x.id, &x.name, x.visible, x.locked, x.opacity, x.clip) != (y.id, &y.name, y.visible, y.locked, y.opacity, y.clip) {
            return Err(format!("{here}: not the same node"));
        }
        let (m, n) = (x.transform, y.transform);
        let off = (m.matrix2.x_axis - n.matrix2.x_axis).abs().max_element().max((m.matrix2.y_axis - n.matrix2.y_axis).abs().max_element()).max((m.translation - n.translation).abs().max_element());
        if off > 1e-3 {
            return Err(format!("{here}: transforms differ by {off}: {m:?} against {n:?}"));
        }
        match (&x.kind, &y.kind) {
            (NodeKind::Group(p), NodeKind::Group(q)) => same_tree(p, q, &here)?,
            (p, q) if p == q => {}
            _ => return Err(format!("{here}: not the same content")),
        }
    }
    Ok(())
}

#[test]
fn grouping_many_nodes_at_once_gives_the_tree_that_moving_them_one_by_one_gives() {
    let mut dice = Dice(9);
    for round in 0..24 {
        let scene = random_scene(&mut dice);
        let ids = some_nodes(&scene, &mut dice, [2, 6, 20, 60][round % 4]);
        let (mut fast, mut slow) = (scene.clone(), scene.clone());
        let (a, b) = (fast.group(&ids), slow.group_one_by_one(&ids));
        assert_eq!(a, b, "round {round}: the same group, or none");
        if let Err(why) = same_tree(&fast.nodes, &slow.nodes, "") {
            panic!("round {round}: {why}");
        }
        if a.is_some() {
            // The grouped nodes keep their places on the screen, whatever they were inside before.
            for id in scene.top_level_selection(&ids) {
                let (before, after) = (scene.world_transform(id).unwrap(), fast.world_transform(id).unwrap());
                let off = (before.matrix2.x_axis - after.matrix2.x_axis).abs().max_element().max((before.translation - after.translation).abs().max_element());
                assert!(off < 1e-2, "round {round}: node {id:?} moved on the screen by {off}");
            }
        }
    }
}

#[test]
fn grouping_thousands_of_nodes_is_quick_and_stamps_what_changed() {
    let mut scene = Scene::default();
    let ids: Vec<NodeId> = (0..20_000)
        .map(|i| {
            let node = scene.new_node(format!("s{i}"), NodeKind::Shape(crate::scene::Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::ONE))));
            scene.add(node, None)
        })
        .collect();
    let before = scene.revision();
    let node_before = scene.find(ids[7]).unwrap().revs.transform;
    let started = std::time::Instant::now();
    let group = scene.group(&ids[..10_000]).expect("a group");
    assert!(started.elapsed().as_secs_f64() < 2.0, "{:?}", started.elapsed());
    assert_eq!(scene.find(group).unwrap().children().len(), 10_000);
    assert_eq!(scene.nodes.len(), 10_001, "the group and the other half");
    assert_ne!(scene.revision().transform, before.transform, "the scene's transform stamp moved");
    assert_ne!(scene.revision().structure, before.structure, "and its structure stamp");
    assert_ne!(scene.find(ids[7]).unwrap().revs.transform, node_before, "a node that was regrouped has a new transform stamp (a renderer that drew it before must not trust its row)");
    assert!(scene.nodes_changed_since(&before).is_none_or(|changed| changed.iter().any(|c| c.0 == NodeId(0)) || changed.iter().any(|c| c.0 == ids[7])), "the journal says that something changed");
}
