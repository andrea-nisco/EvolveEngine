//! The scripting API: what game scripts (written in Rhai) can see and do.
//!
//! A script is a few event handlers attached to a node. Every handler receives the node's id as
//! its first argument (`me`); nodes are plain integer ids. Handlers, all optional:
//!
//! ```text
//! fn on_init(me) { }                      // once, when the game starts
//! fn on_update(me, dt) { }                // every frame
//! fn on_input(me, key) { }                // when a key goes down, e.g. "Space", "Left", "A"
//! fn on_collision(me, other) { }          // a dynamic body hit something solid
//! fn on_trigger(me, other, entered) { }   // a body entered (true) or left (false) a trigger
//! ```
//!
//! Input: `key_down("A")`, `key_pressed("Space")`, `mouse_x()`, `mouse_y()` (the pointer in the world), `mouse_down()`, and for the right
//! button `mouse_right_down()`, `mouse_right_pressed()`, `mouse_right_released()`, and `mouse_wheel()` (how far the wheel moved in this
//! frame, positive up). The camera: `main_camera()` (the id of the node whose frame the player sees, or -1), `camera_width()`,
//! `camera_height()` and `set_camera_height(h)` (the frame's height in world units; its width follows, the aspect ratio stays) or
//! `set_camera_size(w, h)`. Move the camera with `set_pos` / `move_by` on its node.
//!
//! Appearance: `set_opacity`, `set_visible`, `set_fill(me, r, g, b)`, `set_stroke(me, r, g, b)` and `set_stroke_width(me, width)` (0 to
//! 10,000; a small change of width costs the renderer no new triangles, so a game can restyle thousands of borders in a frame).
//!
//! Scenes: `load_scene("Level 2")`, `next_scene()`, `restart_scene()` and `scene_name()`.
//! `set_global("score", 10)` and `get_global("score")` keep values across scenes.

use crate::animation::{compose, decompose};
use crate::geometry::Vec2;
use crate::runtime::World;
use crate::scene::{NodeId, NodeKind, Shape};
use crate::style::{Color, Paint};
use rhai::{Dynamic, Engine, Map};
use std::cell::RefCell;
use std::rc::Rc;

/// Starting point inserted when a node gets its first script.
pub const SCRIPT_TEMPLATE: &str = "\
// Handlers are optional. `me` is this node's id.

fn on_init(me) {
    print(\"hello from \" + me);
}

fn on_update(me, dt) {
    // Example: arrow keys move the node.
    let speed = 200.0;
    if key_down(\"Left\")  { move_by(me, -speed * dt, 0.0); }
    if key_down(\"Right\") { move_by(me,  speed * dt, 0.0); }
}
";

/// Checks that a script parses. Unknown functions are only found when the script runs.
pub fn check_script(source: &str) -> Result<(), String> {
    Engine::new().compile(source).map(|_| ()).map_err(|e| e.to_string())
}

/// Any number a script passes (int or float) as a float.
/// The widest stroke a script can ask for.
const MAX_STROKE_WIDTH: f32 = 10_000.0;

fn num(value: &Dynamic) -> f32 {
    value.as_float().ok().or_else(|| value.as_int().ok().map(|i| i as f64)).unwrap_or(0.0) as f32
}

fn node(id: i64) -> NodeId {
    NodeId(id.max(0) as u64)
}

fn channel(value: &Dynamic) -> u8 {
    num(value).round().clamp(0.0, 255.0) as u8
}

/// The smallest and the largest side a script can give the camera's frame (world units): a frame that is not a number, zero or enormous
/// would give a picture that is nothing at all, or the whole renderer's arithmetic.
const MIN_CAMERA_SIDE: f32 = 0.001;
const MAX_CAMERA_SIDE: f32 = 1.0e7;

/// The size of the main camera's frame (the default one without a camera).
fn camera_size(scene: &crate::scene::Scene) -> Vec2 {
    scene.camera_view().size
}

/// Gives the main camera's frame a size (clamped); nothing if the scene has no camera.
fn set_camera_size(scene: &mut crate::scene::Scene, size: Vec2) {
    let Some(id) = scene.main_camera().filter(|_| size.x.is_finite() && size.y.is_finite()) else { return };
    let size = Vec2::new(size.x.clamp(MIN_CAMERA_SIDE, MAX_CAMERA_SIDE), size.y.clamp(MIN_CAMERA_SIDE, MAX_CAMERA_SIDE));
    if let Some(mut node) = scene.find_mut(id) {
        node.set_camera_size(size);
    }
}

/// Registers every function scripts can call. `world` is shared with the runtime.
pub(crate) fn register_api(engine: &mut Engine, world: &Rc<RefCell<World>>) {
    engine.set_max_operations(500_000).set_max_call_levels(32);

    macro_rules! api {
        ($name:literal, |$w:ident $(, $arg:ident : $ty:ty)*| $body:expr) => {{
            let shared = Rc::clone(world);
            engine.register_fn($name, move |$($arg: $ty),*| {
                #[allow(unused_mut)]
                let mut $w = shared.borrow_mut();
                $body
            });
        }};
    }

    // --- Finding nodes ---------------------------------------------------------------------
    api!("find", |w, name: String| {
        w.scene.find_by_name(&name).map_or(-1, |id| id.0 as i64)
    });
    api!("exists", |w, id: i64| w.scene.find(node(id)).is_some());
    // instantiate("name", x, y): copies the object (or places the symbol) with that name and returns
    // the new id, or -1. (`spawn` would be nicer, but it is a reserved word in Rhai.)
    api!("instantiate", |w, name: String, x: Dynamic, y: Dynamic| {
        let created = w.scene.spawn(&name, Vec2::new(num(&x), num(&y)));
        w.nodes_added = true;
        created.map_or(-1, |id| id.0 as i64)
    });
    api!("destroy", |w, id: i64| {
        w.scene.remove(node(id));
    });

    // --- Transform -------------------------------------------------------------------------
    api!("pos_x", |w, id: i64| w.scene.world_transform(node(id)).map_or(0.0, |t| t.translation.x as f64));
    api!("pos_y", |w, id: i64| w.scene.world_transform(node(id)).map_or(0.0, |t| t.translation.y as f64));
    api!("set_pos", |w, id: i64, x: Dynamic, y: Dynamic| {
        if let Some(origin) = w.scene.world_transform(node(id)).map(|t| t.translation) {
            w.scene.translate_world(node(id), Vec2::new(num(&x), num(&y)) - origin);
        }
    });
    api!("move_by", |w, id: i64, dx: Dynamic, dy: Dynamic| {
        w.scene.translate_world(node(id), Vec2::new(num(&dx), num(&dy)));
    });
    api!("rotation", |w, id: i64| w.scene.find(node(id)).map_or(0.0, |n| decompose(&n.transform).1.to_degrees() as f64));
    api!("set_rotation", |w, id: i64, degrees: Dynamic| {
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            let (position, _, scale) = decompose(&n.transform);
            n.set_transform(compose(position, num(&degrees).to_radians(), scale));
        }
    });
    api!("scale_x", |w, id: i64| w.scene.find(node(id)).map_or(1.0, |n| decompose(&n.transform).2.x as f64));
    api!("scale_y", |w, id: i64| w.scene.find(node(id)).map_or(1.0, |n| decompose(&n.transform).2.y as f64));
    api!("set_scale", |w, id: i64, sx: Dynamic, sy: Dynamic| {
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            let (position, rotation, _) = decompose(&n.transform);
            n.set_transform(compose(position, rotation, Vec2::new(num(&sx), num(&sy))));
        }
    });

    // --- Appearance ------------------------------------------------------------------------
    api!("opacity", |w, id: i64| w.scene.find(node(id)).map_or(0.0, |n| n.opacity as f64));
    api!("set_opacity", |w, id: i64, value: Dynamic| {
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            n.set_opacity(num(&value).clamp(0.0, 1.0));
        }
    });
    api!("set_visible", |w, id: i64, visible: bool| {
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            n.set_visible(visible);
        }
    });
    api!("set_fill", |w, id: i64, r: Dynamic, g: Dynamic, b: Dynamic| {
        let color = Color::rgb(channel(&r), channel(&g), channel(&b));
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            // A color change on a solid fill (the usual case, maybe every frame) only touches the shape's paint.
            if !n.set_solid_fill_color(color) {
                if let NodeKind::Shape(shape) = &mut n.kind {
                    shape.fill = Some(Paint::Solid(color));
                }
            }
        }
    });
    api!("set_stroke", |w, id: i64, r: Dynamic, g: Dynamic, b: Dynamic| {
        let color = Color::rgb(channel(&r), channel(&g), channel(&b));
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            if !n.set_solid_stroke_color(color) {
                if let NodeKind::Shape(Shape { stroke: Some(stroke), .. }) = &mut n.kind {
                    stroke.paint = Paint::Solid(color);
                }
            }
        }
    });

    api!("set_stroke_width", |w, id: i64, width: Dynamic| {
        if let Some(mut n) = w.scene.find_mut(node(id)) {
            n.set_stroke_width(num(&width).clamp(0.0, MAX_STROKE_WIDTH));
        }
    });

    // --- Physics ---------------------------------------------------------------------------
    api!("velocity_x", |w, id: i64| w.physics.velocity(node(id)).x as f64);
    api!("velocity_y", |w, id: i64| w.physics.velocity(node(id)).y as f64);
    api!("set_velocity", |w, id: i64, vx: Dynamic, vy: Dynamic| {
        w.physics.set_velocity(node(id), Vec2::new(num(&vx), num(&vy)));
    });
    api!("add_impulse", |w, id: i64, ix: Dynamic, iy: Dynamic| {
        w.physics.add_impulse(node(id), Vec2::new(num(&ix), num(&iy)));
    });
    api!("set_gravity", |w, gx: Dynamic, gy: Dynamic| {
        w.physics.gravity = Vec2::new(num(&gx), num(&gy));
    });
    api!("overlaps", |w, a: i64, b: i64| w.scene.outlines_overlap(node(a), node(b)));
    // raycast(origin x, y, direction x, y, max distance) -> #{ hit, id, x, y, nx, ny, distance }
    api!("raycast", |w, ox: Dynamic, oy: Dynamic, dx: Dynamic, dy: Dynamic, max: Dynamic| {
        let hit = w.scene.raycast(Vec2::new(num(&ox), num(&oy)), Vec2::new(num(&dx), num(&dy)), num(&max), false);
        let mut result = Map::new();
        result.insert("hit".into(), Dynamic::from(hit.is_some()));
        if let Some(h) = hit {
            result.insert("id".into(), Dynamic::from(h.node.0 as i64));
            result.insert("x".into(), Dynamic::from(h.point.x as f64));
            result.insert("y".into(), Dynamic::from(h.point.y as f64));
            result.insert("nx".into(), Dynamic::from(h.normal.x as f64));
            result.insert("ny".into(), Dynamic::from(h.normal.y as f64));
            result.insert("distance".into(), Dynamic::from(h.distance as f64));
        }
        result
    });

    // --- Input -----------------------------------------------------------------------------
    api!("key_down", |w, key: String| w.input.keys_down.contains(&key));
    api!("key_pressed", |w, key: String| w.input.keys_pressed.contains(&key));
    api!("mouse_x", |w| w.input.mouse.x as f64);
    api!("mouse_y", |w| w.input.mouse.y as f64);
    api!("mouse_down", |w| w.input.mouse_down);
    api!("mouse_right_down", |w| w.input.mouse_right_down);
    api!("mouse_right_pressed", |w| w.input.mouse_right_pressed);
    api!("mouse_right_released", |w| w.input.mouse_right_released);
    api!("mouse_wheel", |w| w.input.wheel as f64);

    // --- The camera ----------------------------------------------------------------------------
    // The frame the player sees is the main camera's `size`. Zooming is changing it; its position is the camera node's.
    api!("main_camera", |w| w.scene.main_camera().map_or(-1, |id| id.0 as i64));
    api!("camera_width", |w| camera_size(&w.scene).x as f64);
    api!("camera_height", |w| camera_size(&w.scene).y as f64);
    api!("set_camera_height", |w, height: Dynamic| {
        let size = camera_size(&w.scene);
        let height = num(&height);
        set_camera_size(&mut w.scene, Vec2::new(height * size.x / size.y.max(MIN_CAMERA_SIDE), height));
    });
    api!("set_camera_size", |w, width: Dynamic, height: Dynamic| {
        set_camera_size(&mut w.scene, Vec2::new(num(&width), num(&height)));
    });

    // --- Animation -------------------------------------------------------------------------
    api!("play", |w, clip: String, looping: bool| {
        w.play_clip(&clip, looping);
    });
    api!("play", |w, clip: String| {
        w.play_clip(&clip, false);
    });
    api!("stop_animation", |w| {
        w.playing = None;
    });

    // --- Per-node variables and misc ---------------------------------------------------------
    api!("get_var", |w, id: i64, name: String| w.vars.get(&(id, name)).cloned().unwrap_or_else(|| Dynamic::from(0_i64)));
    api!("set_var", |w, id: i64, name: String, value: Dynamic| {
        w.vars.insert((id, name), value);
    });
    api!("get_global", |w, name: String| w.globals.get(&name).cloned().unwrap_or_else(|| Dynamic::from(0_i64)));
    api!("set_global", |w, name: String, value: Dynamic| {
        w.globals.insert(name, value);
    });

    // --- Scenes ----------------------------------------------------------------------------
    // The switch happens at the end of the frame. Objects, per-object variables and animation start
    // fresh in the new scene; globals, the console and the clock carry over.
    api!("load_scene", |w, name: String| w.request_scene_named(&name));
    api!("next_scene", |w| w.request_next_scene());
    api!("restart_scene", |w| {
        w.scene_request = Some(w.current_scene);
    });
    api!("scene_name", |w| w.scene_names.get(w.current_scene).cloned().unwrap_or_default());
    api!("time", |w| w.time as f64);
    api!("random", |w| w.random() as f64);

    // `print` goes to the editor's console.
    let shared = Rc::clone(world);
    engine.on_print(move |text| shared.borrow_mut().log_line(text, false));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_errors_are_reported_with_a_position() {
        assert!(check_script(SCRIPT_TEMPLATE).is_ok());
        let error = check_script("fn on_update(me, dt) { let x = ; }").unwrap_err();
        assert!(error.contains("line"), "{error}");
    }

    // --- The right button, the wheel and the camera ---------------------------------------------------

    use crate::runtime::{Input, Runtime};
    use crate::scene::Scene;

    /// A scene whose camera node (800 x 400 frame) has `script`, and a runtime for it.
    fn with_camera_script(script: &str) -> Runtime {
        let mut scene = Scene::with_camera(Vec2::new(800.0, 400.0));
        let camera = scene.main_camera().expect("a camera");
        scene.find_mut(camera).unwrap().script = Some(script.to_string());
        Runtime::new(&scene)
    }

    fn lines(runtime: &Runtime) -> Vec<String> {
        runtime.log().iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn a_script_sets_the_width_of_a_stroke_within_bounds_and_leaves_what_is_not_a_number_alone() {
        use crate::revision::Changes;
        use crate::style::{Color, Stroke};
        let mut scene = Scene::with_camera(Vec2::new(800.0, 400.0));
        let mut shape = Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::splat(10.0)));
        shape.stroke = Some(Stroke::solid(Color::BLACK, 2.0));
        let border = scene.new_node("border", NodeKind::Shape(shape));
        let border = scene.add(border, None);
        let mut plain = Shape::new(crate::path::Path::rect(Vec2::ZERO, Vec2::splat(10.0)));
        plain.stroke = None;
        let plain = scene.new_node("plain", NodeKind::Shape(plain));
        let plain = scene.add(plain, None);
        let camera = scene.main_camera().expect("a camera");
        let stroke_of = |runtime: &Runtime, id: NodeId| match &runtime.scene().find(id).unwrap().kind {
            NodeKind::Shape(shape) => shape.stroke.as_ref().map(|stroke| stroke.width),
            _ => unreachable!(),
        };
        // (what the script asks for, the width the border has after one frame, from 2)
        for (call, width, kinds) in [
            ("3.5", 3.5, Changes::PAINT),
            ("3", 3.0, Changes::PAINT),
            ("0.25", 0.25, Changes::PAINT),
            ("2.0", 2.0, Changes::NONE),
            ("-4.0", 0.0, Changes::PAINT | Changes::GEOMETRY),
            ("0", 0.0, Changes::PAINT | Changes::GEOMETRY),
            ("1.0e30", 10_000.0, Changes::PAINT),
            ("0.0 / 0.0", 2.0, Changes::NONE),
        ] {
            let mut game = scene.clone();
            // The same call on a shape with no stroke, and on a node that is not there: nothing happens to them.
            game.find_mut(camera).unwrap().script = Some(format!("fn on_update(me, dt) {{ set_stroke_width({}, {call}); set_stroke_width({}, 5.0); set_stroke_width(987654, 5.0); }}", border.0, plain.0));
            let mut runtime = Runtime::new(&game);
            let before = runtime.scene().revision();
            runtime.update(1.0 / 60.0, Input::default());
            assert_eq!(stroke_of(&runtime, border), Some(width), "set_stroke_width({call})");
            assert_eq!(stroke_of(&runtime, plain), None, "a shape with no stroke gets none");
            assert_eq!(runtime.scene().changed_since(&before), kinds, "set_stroke_width({call})");
        }
    }

    #[test]
    fn scripts_see_the_right_button_and_the_wheel() {
        let mut runtime = with_camera_script(
            "fn on_update(me, dt) {
                 if mouse_right_pressed() { print(\"pressed\"); }
                 if mouse_right_down() { print(\"down\"); }
                 if mouse_right_released() { print(\"released\"); }
                 if mouse_wheel() != 0.0 { print(\"wheel \" + mouse_wheel()); }
                 if mouse_down() { print(\"left\"); }
             }",
        );
        runtime.update(0.016, Input::default());
        assert!(lines(&runtime).is_empty(), "nothing is pressed: {:?}", lines(&runtime));
        runtime.update(0.016, Input { mouse_right_down: true, mouse_right_pressed: true, wheel: -1.0, ..Default::default() });
        runtime.update(0.016, Input { mouse_right_down: true, ..Default::default() });
        runtime.update(0.016, Input { mouse_right_released: true, wheel: 2.0, ..Default::default() });
        runtime.update(0.016, Input { mouse_down: true, ..Default::default() });
        assert_eq!(lines(&runtime), ["pressed", "down", "wheel -1.0", "down", "released", "wheel 2.0", "left"]);
    }

    #[test]
    fn scripts_find_the_camera_and_change_the_size_of_its_frame_keeping_its_shape() {
        let mut runtime = with_camera_script(
            "fn on_update(me, dt) {
                 print(\"camera \" + (main_camera() == me) + \" \" + camera_width() + \" \" + camera_height());
                 set_camera_height(200.0);
             }",
        );
        runtime.update(0.016, Input::default());
        runtime.update(0.016, Input::default());
        assert_eq!(lines(&runtime), ["camera true 800.0 400.0", "camera true 400.0 200.0"]);
        assert_eq!(runtime.scene().camera_view().size, Vec2::new(400.0, 200.0), "the frame is half as high and half as wide");
        // A frame of any shape can be given too.
        let mut runtime = with_camera_script("fn on_update(me, dt) { set_camera_size(300, 100); }");
        runtime.update(0.016, Input::default());
        assert_eq!(runtime.scene().camera_view().size, Vec2::new(300.0, 100.0));
    }

    #[test]
    fn a_size_that_is_not_a_number_or_is_absurd_cannot_ruin_the_camera() {
        let mut runtime = with_camera_script(
            "fn on_update(me, dt) {
                 let nothing = 0.0 / 0.0;
                 set_camera_height(nothing);
                 set_camera_size(nothing, 100.0);
                 set_camera_size(100.0, 1.0 / 0.0);
             }",
        );
        runtime.update(0.016, Input::default());
        assert_eq!(runtime.scene().camera_view().size, Vec2::new(800.0, 400.0), "NaN and infinity change nothing");
        for (script, side) in [("set_camera_height(0.0)", 0.001), ("set_camera_height(-5.0)", 0.001), ("set_camera_height(1.0e30)", 1.0e7)] {
            let mut runtime = with_camera_script(&format!("fn on_update(me, dt) {{ {script}; }}"));
            runtime.update(0.016, Input::default());
            let size = runtime.scene().camera_view().size;
            assert!(size.is_finite() && size.y == side && size.x > 0.0, "{script}: {size:?}");
        }
    }

    #[test]
    fn setting_the_size_the_camera_has_leaves_the_scene_as_it_was_for_everyone_who_watches_it() {
        let mut runtime = with_camera_script("fn on_update(me, dt) { set_camera_height(400.0); set_camera_size(800.0, 400.0); }");
        let before = runtime.scene().revision();
        runtime.update(0.016, Input::default());
        assert_eq!(runtime.scene().revision(), before, "a game that sets its zoom in every frame does not make a renderer redo its work");
        let mut runtime = with_camera_script("fn on_update(me, dt) { set_camera_height(300.0); }");
        let before = runtime.scene().revision();
        runtime.update(0.016, Input::default());
        assert_eq!(runtime.scene().changed_since(&before), crate::revision::Changes::GEOMETRY, "a new size is a change to the camera and nothing else");
    }

    #[test]
    fn a_scene_without_a_camera_gives_no_camera_to_find_and_none_to_change() {
        let mut scene = Scene::default();
        let node = scene.new_node("script", NodeKind::Group(Vec::new()));
        let id = scene.add(node, None);
        scene.find_mut(id).unwrap().script = Some("fn on_update(me, dt) { print(\"main \" + main_camera()); set_camera_height(10.0); }".to_string());
        let mut runtime = Runtime::new(&scene);
        runtime.update(0.016, Input::default());
        assert_eq!(lines(&runtime), ["main -1"]);
        assert!(runtime.log().iter().all(|l| !l.error));
    }
}
