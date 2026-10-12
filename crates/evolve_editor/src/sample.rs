//! The project shown when the editor starts: a title scene that leads to a small game scene.
//! Together they exercise gradients, curves, strokes, groups, symbols, animation, a rig,
//! physics, scripts and switching between scenes.

use evolve_engine::animation::{Clip, Easing, Property, Value};
use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_engine::physics::{Body, BodyKind};
use evolve_engine::project::Project;
use evolve_engine::scene::{NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, LineCap, LineJoin, Paint, Stroke};
/// The scene the editor opens first (the Sunset scene, which shows the most).
pub const OPEN_AT_START: usize = 1;

/// A title screen followed by the sunset game. The game starts at the title.
pub fn project() -> Project {
    Project { scenes: vec![title(), scene()], start: 0 }
}

/// The title screen: a turning star and a button. Space or a click on the button goes to the next scene.
fn title() -> Scene {
    let size = Vec2::new(1280.0, 720.0);
    let mut scene = Scene::with_camera(size);
    scene.name = "Title".into();

    let mut sky = Shape::new(Path::rect(Vec2::ZERO, size));
    sky.fill = Some(Paint::linear(Vec2::ZERO, Vec2::new(0.0, size.y), Color::rgb(18, 22, 58), Color::rgb(96, 52, 120)));
    sky.stroke = None;
    add(&mut scene, "Sky", sky);

    let mut star = Shape::new(star_path(Vec2::new(640.0, 260.0), 120.0, 50.0, 5));
    star.fill = Some(Paint::linear(Vec2::new(520.0, 140.0), Vec2::new(760.0, 380.0), Color::rgb(255, 224, 102), Color::rgb(255, 140, 66)));
    let mut outline = Stroke::solid(Color::rgb(120, 50, 20), 8.0);
    outline.join = LineJoin::Round;
    star.stroke = Some(outline);
    let star = add(&mut scene, "Star", star);
    scene.center_origin(star);
    set_script(&mut scene, star, "fn on_init(me) { play(\"Spin\", true); }");
    let mut spin = Clip::new("Spin");
    spin.duration = 8.0;
    spin.set_key(star, Property::Rotation, 0.0, Value::Float(0.0), None);
    spin.set_key(star, Property::Rotation, 8.0, Value::Float(std::f32::consts::TAU), None);
    spin.tracks[0].keys[0].easing = Easing::LINEAR;
    scene.clips.push(spin);

    let mut button = Shape::new(Path::rect(Vec2::new(480.0, 478.0), Vec2::new(800.0, 562.0)));
    button.fill = Some(Paint::Solid(Color::rgb(80, 200, 170)));
    button.stroke = Some(Stroke::solid(Color::rgb(20, 90, 80), 4.0));
    let button = add(&mut scene, "Start button", button);
    scene.center_origin(button);
    set_script(
        &mut scene,
        button,
        r#"fn on_init(me) {
    print("Title: press Space, or click the button, to start.");
}

fn on_update(me, dt) {
    let over = abs(mouse_x() - pos_x(me)) < 160.0 && abs(mouse_y() - pos_y(me)) < 42.0;
    set_opacity(me, if over { 1.0 } else { 0.8 });
    // A click is the moment the mouse goes down over the button.
    if over && mouse_down() && get_var(me, "was_down") == 0 { next_scene(); }
    set_var(me, "was_down", if mouse_down() { 1 } else { 0 });
}

fn on_input(me, key) {
    if key == "Space" { next_scene(); }
}"#,
    );

    let play = Path { subpaths: vec![SubPath { anchors: [(622.0, 494.0), (622.0, 546.0), (668.0, 520.0)].map(|(x, y)| Anchor::corner(Vec2::new(x, y))).to_vec(), closed: true }] };
    let mut icon = Shape::new(play);
    icon.fill = Some(Paint::Solid(Color::WHITE));
    icon.stroke = None;
    add(&mut scene, "Play icon", icon);
    scene
}

/// The game scene: hills, a tentacle, a ball you can roll. Escape goes back to the title.
pub fn scene() -> Scene {
    let size = Vec2::new(1280.0, 720.0);
    let mut scene = Scene::with_camera(size);
    scene.name = "Sunset".into();

    // Sky.
    let mut sky = Shape::new(Path::rect(Vec2::ZERO, size));
    sky.fill = Some(Paint::linear(Vec2::ZERO, Vec2::new(0.0, size.y), Color::rgb(35, 52, 110), Color::rgb(255, 170, 120)));
    sky.stroke = None;
    add(&mut scene, "Sky", sky);

    // Sun.
    let mut sun = Shape::new(Path::ellipse(Vec2::new(900.0, 400.0), Vec2::splat(110.0)));
    sun.fill = Some(Paint::radial(Vec2::new(900.0, 400.0), 110.0, Color::rgb(255, 245, 200), Color::rgba(255, 190, 90, 0)));
    sun.stroke = None;
    let sun = add(&mut scene, "Sun", sun);
    scene.center_origin(sun); // so it can pulse around its own middle

    // Hills: smooth closed curves. The end points have a single inward handle so they stay inside the screen.
    let hill = |points: &[(f32, f32)], color: Color| {
        let last = points.len() - 1;
        let mut anchors: Vec<Anchor> = points
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| {
                let mut anchor = Anchor::smooth(Vec2::new(x, y), Vec2::new(90.0, 0.0));
                if i == 0 {
                    anchor.handle_in = Vec2::ZERO;
                }
                if i == last {
                    anchor.handle_out = Vec2::ZERO;
                }
                anchor
            })
            .collect();
        anchors.push(Anchor::corner(Vec2::new(1280.0, 720.0)));
        anchors.push(Anchor::corner(Vec2::new(0.0, 720.0)));
        let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: true }] });
        shape.fill = Some(Paint::Solid(color));
        shape.stroke = None;
        shape
    };
    let far = add(&mut scene, "Far hills", hill(&[(0.0, 470.0), (300.0, 400.0), (620.0, 480.0), (960.0, 410.0), (1280.0, 470.0)], Color::rgb(72, 60, 120)));
    let near = add(&mut scene, "Near hills", hill(&[(0.0, 560.0), (360.0, 500.0), (700.0, 580.0), (1000.0, 520.0), (1280.0, 570.0)], Color::rgb(34, 28, 70)));
    // The hills are solid ground for the game.
    set_body(&mut scene, far, BodyKind::Static);
    set_body(&mut scene, near, BodyKind::Static);
    // The sun is a trigger zone that dims while something passes through it.
    set_body(&mut scene, sun, BodyKind::Trigger);
    set_script(
        &mut scene,
        sun,
        r#"fn on_trigger(me, other, entered) {
    set_opacity(me, if entered { 0.4 } else { 1.0 });
    print(if entered { "the ball is passing through the sun" } else { "the ball left the sun" });
}"#,
    );

    // A star-shaped badge with rounded joins, placed in a group.
    let star = star_path(Vec2::ZERO, 70.0, 30.0, 5);
    let mut badge = Shape::new(star);
    badge.fill = Some(Paint::linear(Vec2::new(-70.0, -70.0), Vec2::new(70.0, 70.0), Color::rgb(255, 224, 102), Color::rgb(255, 140, 66)));
    let mut outline = Stroke::solid(Color::rgb(120, 50, 20), 6.0);
    outline.join = LineJoin::Round;
    outline.cap = LineCap::Round;
    badge.stroke = Some(outline);
    let badge = scene.new_node("Star", NodeKind::Shape(badge));
    let mut group = scene.new_node("Badge", NodeKind::Group(vec![badge]));
    group.transform = Affine2::from_translation(Vec2::new(300.0, 190.0)) * Affine2::from_angle(0.2);
    let badge_id = scene.add(group, None);

    // The badge becomes a symbol; two more instances reuse its geometry with their own colors.
    let mut stars = Vec::new();
    if let Some((symbol, first)) = scene.make_symbol(&[badge_id], "Star badge") {
        stars.push(first);
        for (x, y, scale, fill) in [(640.0, 130.0, 0.7, Color::rgb(120, 200, 255)), (980.0, 220.0, 0.45, Color::rgb(255, 120, 160))] {
            if let Some(id) = scene.place_instance(symbol, Vec2::new(x, y)) {
                if let Some(node) = scene.find_mut(id) {
                    node.transform = Affine2::from_translation(Vec2::new(x, y)) * Affine2::from_scale(Vec2::splat(scale));
                    if let NodeKind::Instance(instance) = &mut node.kind {
                        instance.fill = Some(fill);
                    }
                }
                stars.push(id);
            }
        }
    }

    let tentacle_target = add_tentacle(&mut scene);
    scene.clips.push(idle_clip(&scene, sun, &stars, tentacle_target));
    add_ball(&mut scene);
    scene
}

fn set_body(scene: &mut Scene, id: NodeId, kind: BodyKind) {
    if let Some(node) = scene.find_mut(id) {
        node.body = Some(Body::new(kind));
    }
}

fn set_script(scene: &mut Scene, id: NodeId, source: &str) {
    if let Some(node) = scene.find_mut(id) {
        node.script = Some(source.to_string());
    }
}

/// A ball for the game: arrow keys roll it, Space jumps, R resets it, Escape returns to the title. Run the game with F5.
fn add_ball(scene: &mut Scene) {
    let mut shape = Shape::new(Path::ellipse(Vec2::ZERO, Vec2::splat(20.0)));
    shape.fill = Some(Paint::Solid(Color::rgb(255, 112, 92)));
    shape.stroke = Some(Stroke::solid(Color::rgb(120, 40, 40), 3.0));
    let mut node = scene.new_node("Ball", NodeKind::Shape(shape));
    node.transform = Affine2::from_translation(Vec2::new(900.0, 40.0));
    let mut body = Body::new(BodyKind::Dynamic);
    body.restitution = 0.55;
    node.body = Some(body);
    node.script = Some(
        r#"fn on_init(me) {
    set_global("visits", get_global("visits") + 1);
    print("Arrow keys roll the ball, Space jumps, R resets it, Escape goes back to the title (visit " + get_global("visits") + ").");
    play("Idle", true); // the sun, stars and tentacle keep moving
}

fn on_update(me, dt) {
    let speed = 260.0;
    if key_down("Left")  { set_velocity(me, -speed, velocity_y(me)); }
    if key_down("Right") { set_velocity(me,  speed, velocity_y(me)); }
    if pos_y(me) > 800.0 { set_pos(me, 900.0, 40.0); set_velocity(me, 0.0, 0.0); }
}

fn on_input(me, key) {
    if key == "Space" { add_impulse(me, 0.0, -450.0); }
    if key == "R"     { set_pos(me, 900.0, 40.0); set_velocity(me, 0.0, 0.0); }
    if key == "Escape" { load_scene("Title"); }
}

fn on_collision(me, other) {
    set_var(me, "bounces", get_var(me, "bounces") + 1);
}"#
        .to_string(),
    );
    scene.add(node, None);
}

/// A skinned, IK-driven tentacle growing out of the hills. Returns the node the tip follows.
fn add_tentacle(scene: &mut Scene) -> NodeId {
    // Three 60-unit bones pointing up from the base at (180, 690).
    let up = -std::f32::consts::FRAC_PI_2;
    let base = scene.add_bone(None, Vec2::new(180.0, 690.0), up, 60.0);
    let middle = scene.add_bone(Some(base), Vec2::new(180.0, 630.0), up, 60.0);
    let tip = scene.add_bone(Some(middle), Vec2::new(180.0, 570.0), up, 60.0);

    // A tapering outline with an anchor every 15 units, so it can bend smoothly.
    let half_width = |k: usize| 16.0 - k as f32 * 0.9;
    let point = |k: usize, side: f32| Vec2::new(180.0 + side * half_width(k), 690.0 - k as f32 * 15.0);
    let anchors = (0..=12)
        .map(|k| point(k, -1.0))
        .chain((0..=12).rev().map(|k| point(k, 1.0)))
        .map(Anchor::corner)
        .collect();
    let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: true }] });
    shape.fill = Some(Paint::Solid(Color::rgb(80, 200, 170)));
    let mut outline = Stroke::solid(Color::rgb(20, 90, 80), 3.0);
    outline.join = LineJoin::Round;
    shape.stroke = Some(outline);
    let node = scene.new_node("Tentacle", NodeKind::Shape(shape));
    let tentacle = scene.add(node, None);
    scene.bind_skin(tentacle, &[base, middle, tip]);

    // The tip follows this small marker; animating the marker waves the tentacle.
    let mut marker = Shape::new(Path::ellipse(Vec2::ZERO, Vec2::splat(9.0)));
    marker.fill = Some(Paint::Solid(Color::rgba(255, 170, 60, 200)));
    marker.stroke = None;
    let mut node = scene.new_node("Tentacle target", NodeKind::Shape(marker));
    node.transform = Affine2::from_translation(Vec2::new(180.0, 505.0));
    let target = scene.add(node, None);
    scene.create_ik(tip, target, 3);
    target
}

/// A looping 3 second clip: the sun pulses, one star rocks, one bobs and one spins,
/// and the tentacle's target sways from side to side.
fn idle_clip(scene: &Scene, sun: NodeId, stars: &[NodeId], tentacle_target: NodeId) -> Clip {
    let mut clip = Clip::new("Idle");
    clip.duration = 3.0;
    let start = |id: NodeId, property: Property| scene.find(id).and_then(|n| property.read(n));

    let mut key = |id: NodeId, property: Property, time: f32, value: Value| {
        let before = start(id, property);
        clip.set_key(id, property, time, value, before);
    };
    for (time, size) in [(0.0, 1.0), (1.5, 1.15), (3.0, 1.0)] {
        key(sun, Property::Scale, time, Value::Vec2(Vec2::splat(size)));
    }
    if let Some(Value::Vec2(home)) = start(tentacle_target, Property::Position) {
        for (time, sway) in [(0.0, (0.0, 0.0)), (0.75, (80.0, 15.0)), (1.5, (0.0, 0.0)), (2.25, (-80.0, 15.0)), (3.0, (0.0, 0.0))] {
            key(tentacle_target, Property::Position, time, Value::Vec2(home + Vec2::new(sway.0, sway.1)));
        }
    }
    if let [rocker, bobber, spinner] = *stars {
        for (time, angle) in [(0.0, 0.0), (1.5, 0.4), (3.0, 0.0)] {
            key(rocker, Property::Rotation, time, Value::Float(angle));
        }
        if let Some(Value::Vec2(home)) = start(bobber, Property::Position) {
            for (time, lift) in [(0.0, 0.0), (1.5, -40.0), (3.0, 0.0)] {
                key(bobber, Property::Position, time, Value::Vec2(home + Vec2::new(0.0, lift)));
            }
        }
        key(spinner, Property::Rotation, 0.0, Value::Float(0.0));
        key(spinner, Property::Rotation, 3.0, Value::Float(std::f32::consts::TAU));
        if let Some(track) = clip.tracks.last_mut() {
            track.keys[0].easing = Easing::LINEAR; // a constant spin
        }
    }
    clip
}

fn add(scene: &mut Scene, name: &str, shape: Shape) -> NodeId {
    let node = scene.new_node(name, NodeKind::Shape(shape));
    scene.add(node, None)
}

fn star_path(center: Vec2, outer: f32, inner: f32, points: usize) -> Path {
    let anchors = (0..points * 2)
        .map(|i| {
            let radius = if i % 2 == 0 { outer } else { inner };
            let angle = std::f32::consts::PI * i as f32 / points as f32 - std::f32::consts::FRAC_PI_2;
            Anchor::corner(center + Vec2::from_angle(angle) * radius)
        })
        .collect();
    Path { subpaths: vec![SubPath { anchors, closed: true }] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::render;
    use evolve_engine::tiny_skia::Pixmap;

    #[test]
    fn the_sample_game_runs_without_errors_and_the_ball_lands_on_the_hills() {
        use evolve_engine::runtime::{Input, Runtime};
        let scene = scene();
        let ball = scene.all_ids().into_iter().find(|&id| scene.find(id).is_some_and(|n| n.name == "Ball")).unwrap();
        let mut runtime = Runtime::new(&scene);
        for _ in 0..240 {
            runtime.update(1.0 / 60.0, Input::default());
        }
        let log = runtime.log();
        assert!(log.iter().all(|l| !l.error), "script errors: {:?}", log.iter().filter(|l| l.error).collect::<Vec<_>>());
        assert!(log.iter().any(|l| l.text.contains("passing through the sun")), "the ball should fall through the sun trigger");
        drop(log);
        let center = runtime.scene().world_bounds(ball).unwrap().center();
        assert!(center.y > 300.0 && center.y < 600.0, "the ball should rest on the hills, not fall through or float: {center:?}");
    }

    fn press(key: &str) -> evolve_engine::runtime::Input {
        evolve_engine::runtime::Input { keys_pressed: [key.to_string()].into(), ..Default::default() }
    }

    #[test]
    fn the_title_leads_to_the_game_and_escape_leads_back() {
        use evolve_engine::runtime::{Input, Runtime};
        let project = project();
        assert_eq!((project.scenes[0].name.as_str(), project.scenes[1].name.as_str(), project.start), ("Title", "Sunset", 0));
        let mut runtime = Runtime::start(&project, 0);
        runtime.update(1.0 / 60.0, Input::default());
        assert_eq!(runtime.scene_name(), "Title");
        runtime.update(1.0 / 60.0, press("Space"));
        assert_eq!(runtime.scene_name(), "Sunset");
        runtime.update(1.0 / 60.0, Input::default());
        runtime.update(1.0 / 60.0, press("Escape"));
        assert_eq!(runtime.scene_name(), "Title");
        runtime.update(1.0 / 60.0, press("Space"));
        runtime.update(1.0 / 60.0, Input::default());
        let log: Vec<String> = runtime.log().iter().map(|l| l.text.clone()).collect();
        assert!(log.iter().any(|l| l.contains("visit 1")) && log.iter().any(|l| l.contains("visit 2")), "globals count the visits: {log:?}");
        assert!(runtime.log().iter().all(|l| !l.error), "script errors: {log:?}");
    }

    #[test]
    fn clicking_the_start_button_starts_the_game() {
        use evolve_engine::runtime::{Input, Runtime};
        let mut runtime = Runtime::start(&project(), 0);
        let away = Input { mouse: Vec2::new(100.0, 100.0), mouse_down: true, ..Default::default() };
        runtime.update(1.0 / 60.0, away);
        assert_eq!(runtime.scene_name(), "Title", "a click elsewhere does nothing");
        let click = Input { mouse: Vec2::new(640.0, 520.0), mouse_down: true, ..Default::default() };
        runtime.update(1.0 / 60.0, Input { mouse_down: false, ..click.clone() });
        runtime.update(1.0 / 60.0, click);
        assert_eq!(runtime.scene_name(), "Sunset");
    }

    #[test]
    fn the_sample_tentacle_is_visible() {
        let mut scene = scene();
        scene.solve_ik();
        // Drawn through a camera, like in the editor.
        let view = Affine2::from_scale(Vec2::splat(0.75));
        let mut pixmap = Pixmap::new(960, 540).unwrap();
        render::render(&scene, view, &mut pixmap);
        let at = view.transform_point2(Vec2::new(180.0, 650.0));
        let p = pixmap.pixel(at.x as u32, at.y as u32).unwrap();
        assert!(p.green() > 150 && p.red() < 120, "no teal body at the tentacle: {p:?}");
    }
}
