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
//! Scenes: `load_scene("Level 2")`, `next_scene()`, `restart_scene()` and `scene_name()`.
//! `set_global("score", 10)` and `get_global("score")` keep values across scenes.

use crate::animation::{compose, decompose};
use crate::geometry::Vec2;
use crate::runtime::World;
use crate::scene::{NodeId, NodeKind};
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
fn num(value: &Dynamic) -> f32 {
    value.as_float().ok().or_else(|| value.as_int().ok().map(|i| i as f64)).unwrap_or(0.0) as f32
}

fn node(id: i64) -> NodeId {
    NodeId(id.max(0) as u64)
}

fn channel(value: &Dynamic) -> u8 {
    num(value).round().clamp(0.0, 255.0) as u8
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
        if let Some(n) = w.scene.find_mut(node(id)) {
            let (position, _, scale) = decompose(&n.transform);
            n.transform = compose(position, num(&degrees).to_radians(), scale);
        }
    });
    api!("scale_x", |w, id: i64| w.scene.find(node(id)).map_or(1.0, |n| decompose(&n.transform).2.x as f64));
    api!("scale_y", |w, id: i64| w.scene.find(node(id)).map_or(1.0, |n| decompose(&n.transform).2.y as f64));
    api!("set_scale", |w, id: i64, sx: Dynamic, sy: Dynamic| {
        if let Some(n) = w.scene.find_mut(node(id)) {
            let (position, rotation, _) = decompose(&n.transform);
            n.transform = compose(position, rotation, Vec2::new(num(&sx), num(&sy)));
        }
    });

    // --- Appearance ------------------------------------------------------------------------
    api!("opacity", |w, id: i64| w.scene.find(node(id)).map_or(0.0, |n| n.opacity as f64));
    api!("set_opacity", |w, id: i64, value: Dynamic| {
        if let Some(n) = w.scene.find_mut(node(id)) {
            n.opacity = num(&value).clamp(0.0, 1.0);
        }
    });
    api!("set_visible", |w, id: i64, visible: bool| {
        if let Some(n) = w.scene.find_mut(node(id)) {
            n.visible = visible;
        }
    });
    api!("set_fill", |w, id: i64, r: Dynamic, g: Dynamic, b: Dynamic| {
        if let Some(n) = w.scene.find_mut(node(id)) {
            if let NodeKind::Shape(shape) = &mut n.kind {
                shape.fill = Some(Paint::Solid(Color::rgb(channel(&r), channel(&g), channel(&b))));
            }
        }
    });
    api!("set_stroke", |w, id: i64, r: Dynamic, g: Dynamic, b: Dynamic| {
        if let Some(NodeKind::Shape(shape)) = w.scene.find_mut(node(id)).map(|n| &mut n.kind) {
            if let Some(stroke) = shape.stroke.as_mut() {
                stroke.paint = Paint::Solid(Color::rgb(channel(&r), channel(&g), channel(&b)));
            }
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
}
