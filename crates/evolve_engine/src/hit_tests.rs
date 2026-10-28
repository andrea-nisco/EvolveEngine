//! Hit tests with the boxes of control points (`scene.rs`, `HitCache`): they must give the answer the exact walk gives, for every
//! kind of shape and transform, and after every kind of edit (a stale box would hide a shape that is under the point).

use crate::geometry::{Affine2, Vec2};
use crate::path::{Anchor, Path, SubPath};
use crate::scene::{NodeId, NodeKind, Scene, Shape};
use crate::style::{Color, LineCap, LineJoin, Paint, Stroke};

/// A random number generator that needs nothing (the tests must give the same scenes every time).
pub(crate) struct Dice(pub(crate) u64);

impl Dice {
    pub(crate) fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }

    fn unit(&mut self) -> f32 {
        self.below(1 << 20) as f32 / (1 << 20) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// A random transform: a move to somewhere in `lo..hi` on both axes, a turn, a scale (sometimes mirrored) and sometimes a skew.
fn transform(dice: &mut Dice, lo: f32, hi: f32) -> Affine2 {
    let at = Vec2::new(dice.range(lo, hi), dice.range(lo, hi));
    let turn = dice.range(0.0, std::f32::consts::TAU);
    let (sx, sy) = (dice.range(0.3, 3.0) * if dice.below(10) == 0 { -1.0 } else { 1.0 }, dice.range(0.3, 3.0));
    let m = Affine2::from_scale_angle_translation(Vec2::new(sx, sy), turn, at);
    if dice.below(8) == 0 {
        Affine2::from_cols(m.matrix2.x_axis + m.matrix2.y_axis * 0.4, m.matrix2.y_axis, m.translation)
    } else {
        m
    }
}

fn random_shape(dice: &mut Dice) -> Shape {
    let size = dice.range(2.0, 9.0);
    let curve = |dice: &mut Dice| Path {
        subpaths: vec![SubPath {
            anchors: (0..4)
                .map(|i| Anchor {
                    pos: Vec2::new(i as f32 * size * 0.4, dice.range(-size, size)),
                    handle_in: Vec2::new(-size * 0.3, dice.range(-size, size)),
                    handle_out: Vec2::new(size * 0.3, dice.range(-size, size)),
                })
                .collect(),
            closed: dice.below(2) == 0,
        }],
    };
    let line = |dice: &mut Dice| Path {
        subpaths: vec![SubPath { anchors: (0..5).map(|i| Anchor::corner(Vec2::new(i as f32 * size * 0.3, dice.range(0.0, size)))).collect(), closed: false }],
    };
    let mut shape = match dice.below(5) {
        0 => Shape::new(Path::rect(Vec2::ZERO, Vec2::new(size, size * 0.6))),
        1 => Shape::new(Path::ellipse(Vec2::new(size * 0.5, size * 0.5), Vec2::new(size * 0.5, size * 0.3))),
        2 => Shape::new(curve(dice)),
        3 => Shape::new(line(dice)),
        _ => Shape::new(Path::ellipse(Vec2::ZERO, Vec2::splat(size * 0.2))),
    };
    let width = [0.0, 0.2, 1.0, 3.0, 14.0][dice.below(5)];
    shape.fill = (dice.below(3) != 0).then_some(Paint::Solid(Color::rgb(200, 30, 30)));
    shape.stroke = (width > 0.0 || dice.below(2) == 0).then(|| Stroke {
        paint: Paint::Solid(Color::BLACK),
        width,
        join: [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][dice.below(3)],
        cap: [LineCap::Butt, LineCap::Round, LineCap::Square][dice.below(3)],
        miter_limit: 4.0,
    });
    shape
}

/// A scene with shapes everywhere in 0..400, in groups with their own transforms, some of them clip groups, some shapes hidden or locked,
/// and instances of a symbol.
pub(crate) fn random_scene(dice: &mut Dice) -> Scene {
    let mut scene = Scene::default();
    for g in 0..30 {
        let mut children = Vec::new();
        for _ in 0..10 {
            let mut node = scene.new_node("s", NodeKind::Shape(random_shape(dice)));
            node.transform = transform(dice, -20.0, 60.0);
            node.visible = dice.below(20) != 0;
            node.locked = dice.below(20) == 0;
            children.push(node);
        }
        let mut group = scene.new_node("g", NodeKind::Group(children));
        group.transform = transform(dice, 0.0, 380.0);
        // Every fifth group clips its content with its top child.
        group.clip = g % 5 == 0;
        scene.add(group, None);
    }
    let parts: Vec<NodeId> = (0..3)
        .map(|_| {
            let mut node = scene.new_node("part", NodeKind::Shape(random_shape(dice)));
            node.transform = transform(dice, 0.0, 0.0);
            scene.add(node, None)
        })
        .collect();
    let (symbol, _) = scene.make_symbol(&parts, "Art").expect("a symbol");
    for _ in 0..6 {
        let id = scene.place_instance(symbol, Vec2::ZERO).expect("an instance");
        scene.find_mut(id).unwrap().set_transform(transform(dice, 0.0, 400.0));
    }
    scene
}

/// Asks the same question both ways at random places and says how many gave a hit.
fn compare(scene: &Scene, dice: &mut Dice, queries: usize, what: &str) -> usize {
    let mut hits = 0;
    for _ in 0..queries {
        let p = Vec2::new(dice.range(-30.0, 430.0), dice.range(-30.0, 430.0));
        let tolerance = [0.5, 2.0, 8.0][dice.below(3)];
        let (fast, exact) = (scene.hit_test(p, tolerance), scene.hit_test_exact(p, tolerance));
        assert_eq!(fast, exact, "{what}: at {p:?} with tolerance {tolerance}");
        hits += fast.is_some() as usize;
    }
    hits
}

#[test]
fn a_hit_test_with_boxes_gives_the_answer_of_the_exact_walk_for_every_shape_transform_and_edit() {
    let mut dice = Dice(7);
    for round in 0..4 {
        let mut scene = random_scene(&mut dice);
        let hits = compare(&scene, &mut dice, 1500, &format!("round {round}, a new scene"));
        assert!(hits > 100 && hits < 1400, "the queries are neither all hits nor all misses: {hits} of 1500");
        // Edits of every kind, each followed by queries: a box that is not made again after an edit would hide a shape.
        for step in 0..60 {
            let ids = scene.all_ids();
            let id = ids[dice.below(ids.len())];
            let is_shape = matches!(scene.find(id).map(|n| &n.kind), Some(NodeKind::Shape(_)));
            match dice.below(6) {
                0 => scene.find_mut(id).unwrap().set_transform(transform(&mut dice, 0.0, 400.0)),
                1 if is_shape => {
                    // An outline edit: move a point a long way, so that the box it was measured for is wrong now.
                    if let Some(path) = scene.find_mut(id).unwrap().path_mut() {
                        if let Some(anchor) = path.subpaths.get_mut(0).and_then(|s| s.anchors.get_mut(0)) {
                            anchor.pos += Vec2::new(dice.range(-150.0, 150.0), dice.range(-150.0, 150.0));
                        }
                    }
                }
                2 => {
                    let visible = scene.find(id).unwrap().visible;
                    scene.find_mut(id).unwrap().set_visible(!visible);
                }
                3 if is_shape => {
                    // A different shape in the same node.
                    scene.find_mut(id).unwrap().kind = NodeKind::Shape(random_shape(&mut dice));
                }
                4 => {
                    scene.remove(id);
                }
                _ => {
                    // A copy of the scene (what undo makes) is asked as well as the scene: it starts with nothing remembered.
                    let copy = scene.clone();
                    compare(&copy, &mut dice, 100, &format!("round {round}, step {step}, a copy"));
                }
            }
            compare(&scene, &mut dice, 100, &format!("round {round}, step {step}"));
        }
    }
}

#[test]
fn a_shape_with_a_point_that_is_not_a_number_or_no_points_is_still_tested_exactly() {
    let mut scene = Scene::default();
    let mut nan = Shape::new(Path { subpaths: vec![SubPath { anchors: vec![Anchor::corner(Vec2::ZERO), Anchor::corner(Vec2::new(f32::NAN, 5.0)), Anchor::corner(Vec2::new(10.0, 10.0))], closed: true }] });
    nan.stroke = None;
    let mut empty = Shape::new(Path::default());
    empty.stroke = Some(Stroke::solid(Color::BLACK, 4.0));
    for shape in [nan, empty] {
        let node = scene.new_node("odd", NodeKind::Shape(shape));
        scene.add(node, None);
    }
    for (x, y) in [(1.0, 1.0), (5.0, 5.0), (50.0, 50.0), (-3.0, 2.0)] {
        let p = Vec2::new(x, y);
        assert_eq!(scene.hit_test(p, 1.0), scene.hit_test_exact(p, 1.0), "at {p:?}");
    }
}

#[test]
fn a_wide_stroke_is_hit_far_from_the_points_of_its_path_and_a_thin_shape_is_not_hit_beyond_the_tolerance() {
    let mut scene = Scene::default();
    let mut line = Shape::new(Path { subpaths: vec![SubPath { anchors: vec![Anchor::corner(Vec2::new(0.0, 0.0)), Anchor::corner(Vec2::new(100.0, 0.0))], closed: false }] });
    line.fill = None;
    line.stroke = Some(Stroke::solid(Color::BLACK, 20.0));
    let node = scene.new_node("line", NodeKind::Shape(line));
    let id = scene.add(node, None);
    assert_eq!(scene.hit_test(Vec2::new(50.0, 9.0), 1.0), Some(vec![id]), "inside half of the width");
    assert_eq!(scene.hit_test(Vec2::new(50.0, 12.5), 1.0), None, "outside it and the tolerance");
    assert_eq!(scene.hit_test(Vec2::new(50.0, 10.8), 1.0), Some(vec![id]), "just inside the tolerance");
    // Moving the shape makes the box wrong; the next test must not use the old one.
    scene.find_mut(id).unwrap().set_transform(Affine2::from_translation(Vec2::new(0.0, 300.0)));
    assert_eq!(scene.hit_test(Vec2::new(50.0, 5.0), 1.0), None);
    assert_eq!(scene.hit_test(Vec2::new(50.0, 305.0), 1.0), Some(vec![id]));
    // Changing the outline makes the stored box wrong too.
    scene.find_mut(id).unwrap().path_mut().unwrap().subpaths[0].anchors[1].pos = Vec2::new(100.0, 200.0);
    assert_eq!(scene.hit_test(Vec2::new(50.0, 400.0), 1.0), Some(vec![id]), "the line now runs from (0, 300) to (100, 500)");
    assert_eq!(scene.hit_test(Vec2::new(50.0, 305.0), 1.0), None, "and is no longer where it was");
}

/// A skinned shape is bent by bones: its outline is not its path, so the box of the path says nothing about where it is.
#[test]
fn a_skinned_shape_is_hit_where_it_is_bent_to_and_not_only_where_its_bind_pose_is() {
    let mut scene = Scene::default();
    let upper = scene.add_bone(None, Vec2::ZERO, 0.0, 10.0);
    let lower = scene.add_bone(Some(upper), Vec2::new(10.0, 0.0), 0.0, 10.0);
    let bar = Shape::new(Path::rect(Vec2::new(0.0, -2.0), Vec2::new(20.0, 2.0)));
    let node = scene.new_node("bar", NodeKind::Shape(bar));
    let id = scene.add(node, None);
    assert!(scene.bind_skin(id, &[upper, lower]));
    scene.set_bone_world(lower, Vec2::new(10.0, 0.0), std::f32::consts::FRAC_PI_2, None);
    let mut hits_above_the_bind_pose = 0;
    for y in 3..9 {
        for x in [9.0, 10.0, 11.0] {
            let p = Vec2::new(x, y as f32);
            let (fast, exact) = (scene.hit_test(p, 0.5), scene.hit_test_exact(p, 0.5));
            assert_eq!(fast, exact, "at {p:?}");
            hits_above_the_bind_pose += (fast == Some(vec![id])) as usize;
        }
    }
    assert!(hits_above_the_bind_pose >= 3, "the bent bar reaches up past the box of its bind pose: {hits_above_the_bind_pose} hits");
}

/// Not a check, a measurement: `cargo test --release -p evolve_engine how_long_a_click -- --ignored --nocapture`. 21,000 shapes of
/// the kinds above, the point asked about spread over them, the exact walk against the one with boxes (the first call measures them).
#[test]
#[ignore]
fn how_long_a_click_takes_on_a_big_scene() {
    let mut dice = Dice(11);
    let mut scene = Scene::default();
    for _ in 0..210 {
        let mut children = Vec::new();
        for _ in 0..100 {
            let mut node = scene.new_node("s", NodeKind::Shape(random_shape(&mut dice)));
            node.transform = transform(&mut dice, 0.0, 2000.0);
            children.push(node);
        }
        let group = scene.new_node("g", NodeKind::Group(children));
        scene.add(group, None);
    }
    let points: Vec<Vec2> = (0..200).map(|_| Vec2::new(dice.range(0.0, 2000.0), dice.range(0.0, 2000.0))).collect();
    let time = |f: &dyn Fn(Vec2) -> Option<Vec<NodeId>>| {
        let mut times: Vec<f64> = points
            .iter()
            .map(|p| {
                let started = std::time::Instant::now();
                std::hint::black_box(f(*p));
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        (times[times.len() / 2], times[times.len() * 95 / 100])
    };
    let first = std::time::Instant::now();
    scene.hit_test(Vec2::new(1.0, 1.0), 2.0);
    let first = first.elapsed().as_secs_f64() * 1000.0;
    let exact = time(&|p| scene.hit_test_exact(p, 2.0));
    let boxed = time(&|p| scene.hit_test(p, 2.0));
    eprintln!("21,000 shapes: exact walk median {:.2} ms (p95 {:.2}); with boxes median {:.3} ms (p95 {:.3}); the first call, which measures them, {:.2} ms", exact.0, exact.1, boxed.0, boxed.1, first);
}
