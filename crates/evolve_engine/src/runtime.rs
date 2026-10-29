//! The game runtime: runs a copy of a scene with its scripts, physics and animation.
//!
//! The editor's Play mode and exported games both use this. The runtime owns its own scene copy,
//! so playing never changes the project being edited.

use crate::geometry::{Rect, Vec2};
use crate::path::Polyline;
use crate::physics::{BodyKind, Contact, ContactKind, PhysicsSettings, PhysicsWorld};
use crate::project::Project;
use crate::scene::{NodeId, NodeKind, Scene};
use crate::script;
use rhai::{Dynamic, Engine, FuncArgs, Scope, AST};
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Largest time step simulated at once; longer frames are slowed down instead of skipping physics.
const MAX_STEP: f32 = 0.05;

/// What the player is doing this frame. Key names are `"A"`..`"Z"`, `"Num0"`..`"Num9"`, `"Space"`,
/// `"Enter"`, `"Left"`, `"Right"`, `"Up"`, `"Down"`, and so on.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub keys_down: HashSet<String>,
    /// Keys that went down this frame.
    pub keys_pressed: HashSet<String>,
    /// Pointer position in world space.
    pub mouse: Vec2,
    pub mouse_down: bool,
    /// The right mouse button is held, went down in this frame, went up in this frame.
    pub mouse_right_down: bool,
    pub mouse_right_pressed: bool,
    pub mouse_right_released: bool,
    /// How far the mouse wheel moved in this frame (positive: up, away from the player). Zero when the pointer is not over the game.
    pub wheel: f32,
}

/// One line of the game's console.
#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: f32,
    pub text: String,
    pub error: bool,
}

pub(crate) struct PlayingClip {
    pub clip: usize,
    pub time: f32,
    pub looping: bool,
}

/// State shared between the runtime and the functions scripts call.
pub(crate) struct World {
    pub scene: Scene,
    pub physics: PhysicsWorld,
    pub input: Input,
    pub vars: HashMap<(i64, String), Dynamic>,
    pub log: Vec<LogLine>,
    pub time: f32,
    pub playing: Option<PlayingClip>,
    /// Set when objects were created while running, so new scripts get compiled.
    pub nodes_added: bool,
    /// Values scripts keep across scenes (`set_global`), unlike per-object variables.
    pub globals: HashMap<String, Dynamic>,
    /// Names of all scenes in play order, and the index of the running one.
    pub scene_names: Vec<String>,
    pub current_scene: usize,
    /// A scene change asked for by a script; carried out at the end of the frame.
    pub scene_request: Option<usize>,
    rng: u64,
}

impl World {
    fn new(scene: Scene) -> Self {
        Self {
            scene,
            physics: PhysicsWorld::default(),
            input: Input::default(),
            vars: HashMap::new(),
            log: Vec::new(),
            time: 0.0,
            playing: None,
            nodes_added: false,
            globals: HashMap::new(),
            scene_names: Vec::new(),
            current_scene: 0,
            scene_request: None,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    pub fn log_line(&mut self, text: &str, error: bool) {
        self.log.push(LogLine { time: self.time, text: text.to_string(), error });
    }

    /// Starts a clip by name; a clip already playing is replaced.
    pub fn play_clip(&mut self, name: &str, looping: bool) {
        match self.scene.clips.iter().position(|c| c.name == name) {
            Some(clip) => self.playing = Some(PlayingClip { clip, time: 0.0, looping }),
            None => self.log_line(&format!("play: there is no clip named \"{name}\""), true),
        }
    }

    /// Asks for the scene called `name` to run after this frame.
    pub fn request_scene_named(&mut self, name: &str) {
        match self.scene_names.iter().position(|n| n == name) {
            Some(index) => self.scene_request = Some(index),
            None => self.log_line(&format!("load_scene: there is no scene named \"{name}\""), true),
        }
    }

    /// Asks for the scene after the current one in the project's order.
    pub fn request_next_scene(&mut self) {
        if self.current_scene + 1 < self.scene_names.len() {
            self.scene_request = Some(self.current_scene + 1);
        } else {
            self.log_line("next_scene: this is the last scene", true);
        }
    }

    /// Uniform random number in 0..1 (xorshift).
    pub fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn step_animation(&mut self, dt: f32) {
        let Some(playing) = self.playing.as_mut() else { return };
        let Some(duration) = self.scene.clips.get(playing.clip).map(|c| c.duration.max(1e-3)) else {
            self.playing = None;
            return;
        };
        playing.time += dt;
        let finished = playing.time >= duration && !playing.looping;
        if playing.time >= duration {
            playing.time = if playing.looping { playing.time % duration } else { duration };
        }
        let (clip, time) = (playing.clip, playing.time);
        let pose = self.scene.clips[clip].pose_at(&self.scene, time);
        self.scene.apply_pose(&pose);
        if finished {
            self.playing = None;
        }
    }
}

struct ScriptInstance {
    node: NodeId,
    name: String,
    ast: AST,
    /// Handler name -> number of parameters it declares.
    handlers: HashMap<String, usize>,
    /// Set after a runtime error; the script is not called again.
    failed: bool,
    /// `on_init` has run (it waits until the node is visible for the first time).
    initialized: bool,
}

/// A body outline or circle for drawing collision debug overlays.
#[derive(Clone, Debug)]
pub struct DebugBody {
    pub id: NodeId,
    pub kind: BodyKind,
    pub outlines: Vec<Polyline>,
    /// Dynamic bodies collide as this circle (center, radius).
    pub circle: Option<(Vec2, f32)>,
    /// Out of view, it was not stepped in the last frame (see [`crate::physics::PhysicsSettings`]).
    pub waiting: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub shapes: usize,
    pub bodies: usize,
    pub scripts: usize,
    pub frames: u64,
}

pub struct Runtime {
    world: Rc<RefCell<World>>,
    engine: Engine,
    scripts: Vec<ScriptInstance>,
    /// Nodes whose script has been compiled already.
    registered: HashSet<NodeId>,
    frames: u64,
    /// Every scene of the project, so scripts can switch between them. Each is started fresh from here.
    scenes: Vec<Scene>,
}

impl Runtime {
    /// Starts a game from a single scene (there are no other scenes to switch to).
    pub fn new(scene: &Scene) -> Self {
        Self::start(&Project::single(scene.clone()), 0)
    }

    /// Starts the game of `project` in its scene number `index`. Scripts that fail to compile are reported in the log.
    pub fn start(project: &Project, index: usize) -> Self {
        let index = index.min(project.scenes.len().saturating_sub(1));
        let mut world = World::new(project.scenes[index].clone());
        world.physics.settings = project.physics;
        world.scene_names = project.scenes.iter().map(|s| s.name.clone()).collect();
        world.current_scene = index;
        let world = Rc::new(RefCell::new(world));
        let mut engine = Engine::new();
        script::register_api(&mut engine, &world);
        let mut runtime = Self { world, engine, scripts: Vec::new(), registered: HashSet::new(), frames: 0, scenes: project.scenes.clone() };
        runtime.register_new_scripts();
        runtime
    }

    pub fn physics_settings(&self) -> PhysicsSettings {
        self.world.borrow().physics.settings
    }

    /// Changes the project's physics settings while the game runs; they hold from the next frame on, in every scene.
    pub fn set_physics_settings(&mut self, settings: PhysicsSettings) {
        self.world.borrow_mut().physics.settings = settings;
    }

    /// Replaces the running scene by scene number `index`, started from scratch. The console, the
    /// clock, the globals, the random sequence and the project's physics settings carry over; everything else is new.
    fn switch_to(&mut self, index: usize) {
        let Some(scene) = self.scenes.get(index) else { return };
        {
            let mut world = self.world.borrow_mut();
            let old = std::mem::replace(&mut *world, World::new(scene.clone()));
            world.log = old.log;
            world.time = old.time;
            world.globals = old.globals;
            world.rng = old.rng;
            world.physics.settings = old.physics.settings;
            world.input = old.input;
            world.scene_names = old.scene_names;
            world.current_scene = index;
        }
        self.scripts.clear();
        self.registered.clear();
        self.register_new_scripts();
    }

    /// Name of the scene that is running.
    pub fn scene_name(&self) -> String {
        let world = self.world.borrow();
        world.scene_names.get(world.current_scene).cloned().unwrap_or_default()
    }

    /// Compiles the scripts of nodes that have not been seen yet (at start, and after `spawn`).
    fn register_new_scripts(&mut self) {
        let found: Vec<(NodeId, String, String)> = {
            let world = self.world.borrow();
            let mut found = Vec::new();
            world.scene.for_each_node(|node| {
                if let (false, Some(script)) = (self.registered.contains(&node.id), &node.script) {
                    found.push((node.id, node.name.clone(), script.clone()));
                }
            });
            found
        };
        for (id, name, source) in found {
            self.registered.insert(id);
            match self.engine.compile(&source) {
                Ok(ast) => {
                    let handlers = ast.iter_functions().map(|f| (f.name.to_string(), f.params.len())).collect();
                    self.scripts.push(ScriptInstance { node: id, name, ast, handlers, failed: false, initialized: false });
                }
                Err(error) => self.world.borrow_mut().log_line(&format!("script of '{name}' does not compile: {error}"), true),
            }
        }
    }

    /// A script runs only while its node exists and is visible (hidden objects are inactive templates).
    fn is_active(&self, index: usize) -> bool {
        self.world.borrow().scene.is_shown(self.scripts[index].node)
    }

    /// Advances the game by `dt` seconds.
    pub fn update(&mut self, dt: f32, input: Input) {
        let dt = dt.clamp(0.0, MAX_STEP);
        let pressed: Vec<String> = input.keys_pressed.iter().cloned().collect();
        let spawned = {
            let mut world = self.world.borrow_mut();
            world.input = input;
            world.time += dt;
            std::mem::take(&mut world.nodes_added)
        };
        if spawned {
            self.register_new_scripts();
        }

        for i in 0..self.scripts.len() {
            if !self.is_active(i) {
                continue;
            }
            let me = self.scripts[i].node.0 as i64;
            if !self.scripts[i].initialized {
                self.scripts[i].initialized = true;
                self.call(i, "on_init", (me,), 1);
            }
            self.call(i, "on_update", (me, dt as f64), 2);
            for key in &pressed {
                self.call(i, "on_input", (me, key.clone()), 2);
            }
        }

        let contacts = {
            let mut world = self.world.borrow_mut();
            let world = &mut *world;
            world.physics.step(&mut world.scene, dt)
        };
        self.dispatch_contacts(&contacts);

        {
            let mut world = self.world.borrow_mut();
            world.step_animation(dt);
            world.scene.solve_ik();
        }
        self.frames += 1;

        // A scene change asked for by a script takes effect between frames.
        let request = self.world.borrow_mut().scene_request.take();
        if let Some(index) = request {
            self.switch_to(index);
        }
    }

    fn call(&mut self, index: usize, function: &str, args: impl FuncArgs, parameters: usize) {
        let script = &self.scripts[index];
        if script.failed || script.handlers.get(function) != Some(&parameters) {
            return;
        }
        if !self.world.borrow().scene.is_shown(script.node) {
            return; // destroyed or hidden
        }
        if let Err(error) = self.engine.call_fn::<Dynamic>(&mut Scope::new(), &script.ast, function, args) {
            let message = format!("{function} of '{}' failed: {error}", script.name);
            self.world.borrow_mut().log_line(&message, true);
            self.scripts[index].failed = true;
        }
    }

    fn dispatch_contacts(&mut self, contacts: &[Contact]) {
        for contact in contacts {
            // Both bodies hear about the contact, each with the other one as `other`.
            for (me, other) in [(contact.a, contact.b), (contact.b, contact.a)] {
                let Some(index) = self.scripts.iter().position(|s| s.node == me) else { continue };
                let (me, other) = (me.0 as i64, other.0 as i64);
                match (contact.trigger, contact.kind) {
                    (false, ContactKind::Begin) => self.call(index, "on_collision", (me, other), 2),
                    (true, kind) => self.call(index, "on_trigger", (me, other, kind == ContactKind::Begin), 3),
                    (false, ContactKind::End) => {}
                }
            }
        }
    }

    // --- Inspecting the running game -------------------------------------------------------

    /// The scene as it currently looks (draw this).
    pub fn scene(&self) -> Ref<'_, Scene> {
        Ref::map(self.world.borrow(), |w| &w.scene)
    }

    pub fn log(&self) -> Ref<'_, Vec<LogLine>> {
        Ref::map(self.world.borrow(), |w| &w.log)
    }

    pub fn stats(&self) -> Stats {
        let world = self.world.borrow();
        let (mut shapes, mut bodies) = (0, 0);
        world.scene.for_each_node(|node| {
            shapes += matches!(node.kind, NodeKind::Shape(_)) as usize;
            bodies += node.body.is_some() as usize;
        });
        Stats { shapes, bodies, scripts: self.scripts.len(), frames: self.frames }
    }

    /// Collision geometry of all bodies, for debug drawing.
    pub fn debug_bodies(&self) -> Vec<DebugBody> {
        let world = self.world.borrow();
        let scene = &world.scene;
        let mut with_body = Vec::new();
        scene.for_each_node(|node| {
            if let Some(body) = node.body {
                with_body.push((node.id, body));
            }
        });
        with_body
            .into_iter()
            .map(|(id, body)| {
                let circle = (body.kind == BodyKind::Dynamic).then(|| {
                    let bounds = scene.world_bounds(id)?;
                    Some((bounds.center(), (bounds.size().x + bounds.size().y) * 0.25))
                });
                let outlines = if body.kind == BodyKind::Dynamic { Vec::new() } else { scene.world_outlines(id) };
                DebugBody { id, kind: body.kind, outlines, circle: circle.flatten(), waiting: world.physics.is_waiting(id) }
            })
            .collect()
    }

    /// Where physics bodies count as in view, for drawing it; `None` when every body is stepped every frame.
    pub fn debug_physics_area(&self) -> Option<Rect> {
        let world = self.world.borrow();
        world.physics.active_area(&world.scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Path;
    use crate::physics::Body;
    use crate::scene::Shape;

    fn add(scene: &mut Scene, name: &str, path: Path, body: Option<Body>, script: Option<&str>) -> NodeId {
        let mut node = scene.new_node(name, NodeKind::Shape(Shape::new(path)));
        node.body = body;
        node.script = script.map(str::to_string);
        scene.add(node, None)
    }

    fn run(runtime: &mut Runtime, frames: usize, input: Input) {
        for _ in 0..frames {
            runtime.update(1.0 / 60.0, input.clone());
        }
    }

    fn keys(down: &[&str]) -> Input {
        Input { keys_down: down.iter().map(|k| k.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn scripts_get_init_and_update_calls_and_can_print() {
        let mut scene = Scene::default();
        let id = add(
            &mut scene,
            "mover",
            Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)),
            None,
            Some("fn on_init(me) { print(\"ready\"); } fn on_update(me, dt) { move_by(me, 60.0 * dt, 0); }"),
        );
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 60, Input::default());
        let x = runtime.scene().world_bounds(id).unwrap().center().x;
        assert!((x - 60.0).abs() < 1.0, "moved {x}");
        assert!(runtime.log().iter().any(|l| l.text == "ready" && !l.error));
        assert_eq!(scene.world_bounds(id).unwrap().center().x, 0.0, "the edited scene is untouched");
    }

    #[test]
    fn keyboard_input_reaches_scripts() {
        let mut scene = Scene::default();
        let id = add(
            &mut scene,
            "player",
            Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)),
            None,
            Some("fn on_update(me, dt) { if key_down(\"Right\") { move_by(me, 100.0 * dt, 0.0); } }"),
        );
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 30, keys(&[]));
        run(&mut runtime, 30, keys(&["Right"]));
        let x = runtime.scene().world_bounds(id).unwrap().center().x;
        assert!((x - 50.0).abs() < 1.0, "moved {x}");
    }

    #[test]
    fn collisions_and_triggers_call_their_handlers() {
        let mut scene = Scene::default();
        add(&mut scene, "floor", Path::rect(Vec2::new(-100.0, 100.0), Vec2::new(100.0, 120.0)), Some(Body::new(BodyKind::Static)), None);
        add(
            &mut scene,
            "zone",
            Path::rect(Vec2::new(-20.0, 40.0), Vec2::new(20.0, 70.0)),
            Some(Body::new(BodyKind::Trigger)),
            Some("fn on_trigger(me, other, entered) { print(if entered { \"in\" } else { \"out\" }); }"),
        );
        add(
            &mut scene,
            "ball",
            Path::ellipse(Vec2::new(0.0, 0.0), Vec2::splat(8.0)),
            Some(Body::new(BodyKind::Dynamic)),
            Some("fn on_collision(me, other) { set_var(me, \"hits\", get_var(me, \"hits\") + 1); print(\"hit\"); }"),
        );
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 180, Input::default());
        let log: Vec<String> = runtime.log().iter().map(|l| l.text.clone()).collect();
        assert!(log.contains(&"in".to_string()) && log.contains(&"out".to_string()), "{log:?}");
        assert!(log.contains(&"hit".to_string()), "{log:?}");
    }

    #[test]
    fn script_errors_are_logged_and_stop_only_that_script() {
        let mut scene = Scene::default();
        add(&mut scene, "broken", Path::rect(Vec2::ZERO, Vec2::splat(5.0)), None, Some("fn on_update(me, dt) { no_such_function(); }"));
        add(&mut scene, "bad syntax", Path::rect(Vec2::ZERO, Vec2::splat(5.0)), None, Some("fn on_update( {"));
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 5, Input::default());
        let errors = runtime.log().iter().filter(|l| l.error).count();
        assert_eq!(errors, 2, "one compile error and one runtime error (not repeated every frame)");
    }

    #[test]
    fn spawned_copies_run_their_own_scripts_while_the_hidden_template_stays_inert() {
        let mut scene = Scene::default();
        let template = add(
            &mut scene,
            "Bullet",
            Path::rect(Vec2::splat(-2.0), Vec2::splat(2.0)),
            None,
            Some("fn on_update(me, dt) { move_by(me, 60.0 * dt, 0.0); }"),
        );
        scene.find_mut(template).unwrap().visible = false;
        add(
            &mut scene,
            "Gun",
            Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)),
            None,
            Some("fn on_init(me) { let a = instantiate(\"Bullet\", 10, 10); let b = instantiate(\"Bullet\", 10, 50.5); print(\"ids \" + a + \" \" + b); print(instantiate(\"Nothing\", 0, 0)); }"),
        );
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 61, Input::default());
        let world = runtime.scene();
        let bullets: Vec<NodeId> = world.all_ids().into_iter().filter(|&id| world.find(id).is_some_and(|n| n.name == "Bullet" && n.visible)).collect();
        assert_eq!(bullets.len(), 2, "log: {:?}", runtime.log().iter().map(|l| l.text.clone()).collect::<Vec<_>>());
        for id in bullets {
            let x = world.world_transform(id).unwrap().translation.x;
            assert!((x - 70.0).abs() < 2.0, "a spawned bullet flew from x=10 to about 70, got {x}");
        }
        assert_eq!(world.world_transform(template).unwrap().translation.x, 0.0, "the hidden template did not move");
        drop(world);
        assert!(runtime.log().iter().any(|l| l.text == "-1"), "spawning an unknown name returns -1");
    }

    #[test]
    fn runaway_loops_are_stopped() {
        let mut scene = Scene::default();
        add(&mut scene, "loop", Path::rect(Vec2::ZERO, Vec2::splat(5.0)), None, Some("fn on_update(me, dt) { loop { } }"));
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 1, Input::default());
        assert!(runtime.log().iter().any(|l| l.error));
    }

    #[test]
    fn scripts_can_ray_cast_play_clips_and_find_nodes() {
        let mut scene = Scene::default();
        add(&mut scene, "wall", Path::rect(Vec2::new(50.0, -20.0), Vec2::new(60.0, 20.0)), None, None);
        let mut clip = crate::animation::Clip::new("fade");
        let actor = add(
            &mut scene,
            "actor",
            Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)),
            None,
            Some(
                "fn on_init(me) {
                    let hit = raycast(20.0, 0.0, 1.0, 0.0, 500.0);
                    set_var(me, \"d\", hit.distance);
                    print(\"found wall: \" + find(\"wall\") + \" distance \" + hit.distance);
                    play(\"fade\");
                }",
            ),
        );
        clip.set_key(actor, crate::animation::Property::Opacity, 0.0, crate::animation::Value::Float(1.0), None);
        clip.set_key(actor, crate::animation::Property::Opacity, 1.0, crate::animation::Value::Float(0.0), None);
        scene.clips.push(clip);
        let mut runtime = Runtime::new(&scene);
        run(&mut runtime, 30, Input::default());
        assert!(runtime.log().iter().any(|l| l.text.contains("distance 30")), "{:?}", runtime.log().iter().map(|l| &l.text).collect::<Vec<_>>());
        let opacity = runtime.scene().find(actor).unwrap().opacity;
        assert!(opacity < 0.9 && opacity > 0.1, "the clip is fading the actor: {opacity}");
    }

    /// Three scenes. The first one's script runs `first_script`; scene 2 and 3 each print their name.
    fn three_scenes(first_script: &str) -> Project {
        let mut project = Project::new();
        project.add_scene();
        project.add_scene();
        for i in 0..3 {
            let script = if i == 0 { first_script.to_string() } else { "fn on_init(me) { print(\"in \" + scene_name() + \" score \" + get_global(\"score\")); }".to_string() };
            add(&mut project.scenes[i], &format!("actor {i}"), Path::rect(Vec2::splat(-5.0), Vec2::splat(5.0)), None, Some(&script));
        }
        project
    }

    fn printed(runtime: &Runtime) -> Vec<String> {
        runtime.log().iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn next_scene_moves_through_the_list_and_globals_carry_over() {
        let project = three_scenes("fn on_init(me) { set_global(\"score\", 42); } fn on_update(me, dt) { next_scene(); }");
        let mut runtime = Runtime::start(&project, 0);
        assert_eq!(runtime.scene_name(), "Scene 1");
        run(&mut runtime, 1, Input::default()); // scene 1 asks for the next scene...
        assert_eq!(runtime.scene_name(), "Scene 2", "...and it starts at the end of that frame");
        assert!(runtime.scene().find_by_name("actor 1").is_some() && runtime.scene().find_by_name("actor 0").is_none());
        run(&mut runtime, 1, Input::default());
        assert!(printed(&runtime).contains(&"in Scene 2 score 42".to_string()), "{:?}", printed(&runtime));
    }

    #[test]
    fn load_scene_finds_scenes_by_name_and_reports_unknown_ones() {
        let project = three_scenes("fn on_update(me, dt) { if time() > 0.0 { load_scene(\"Scene 3\"); } }");
        let mut runtime = Runtime::start(&project, 0);
        run(&mut runtime, 2, Input::default());
        assert_eq!(runtime.scene_name(), "Scene 3");

        let project = three_scenes("fn on_init(me) { load_scene(\"Nowhere\"); }");
        let mut runtime = Runtime::start(&project, 0);
        run(&mut runtime, 2, Input::default());
        assert_eq!(runtime.scene_name(), "Scene 1", "stays in the scene");
        assert!(runtime.log().iter().any(|l| l.error && l.text.contains("Nowhere")));
    }

    #[test]
    fn restart_scene_starts_it_from_scratch_and_the_last_scene_has_no_next() {
        let project = three_scenes("fn on_update(me, dt) { move_by(me, 1.0, 0.0); if pos_x(me) > 2.5 { restart_scene(); } }");
        let mut runtime = Runtime::start(&project, 0);
        run(&mut runtime, 3, Input::default());
        let (id, x) = {
            let scene = runtime.scene();
            let id = scene.find_by_name("actor 0").unwrap();
            (id, scene.world_transform(id).unwrap().translation.x)
        };
        assert_eq!(x, 0.0, "the restart put the actor back at its start ({id:?})");

        let project = three_scenes("fn on_init(me) { }");
        let mut runtime = Runtime::start(&project, 2);
        assert_eq!(runtime.scene_name(), "Scene 3");
        runtime.world.borrow_mut().request_next_scene();
        run(&mut runtime, 1, Input::default());
        assert_eq!(runtime.scene_name(), "Scene 3");
        assert!(runtime.log().iter().any(|l| l.error && l.text.contains("last scene")));
    }

    #[test]
    fn the_debug_view_shows_the_physics_area_and_which_bodies_wait() {
        // The default camera frames (0, 0) to (1280, 720); with a margin of 0.5 the area reaches 640 and 360 further out.
        let mut scene = Scene::default();
        let dynamic = Some(Body::new(BodyKind::Dynamic));
        let near = add(&mut scene, "near", Path::ellipse(Vec2::new(640.0, 360.0), Vec2::splat(10.0)), dynamic, None);
        let far = add(&mut scene, "far", Path::ellipse(Vec2::new(9000.0, 360.0), Vec2::splat(10.0)), dynamic, None);
        let mut project = Project::single(scene);
        assert_eq!(Runtime::start(&project, 0).debug_physics_area(), None, "the rule is off");

        project.physics.slow_offscreen = true;
        let mut runtime = Runtime::start(&project, 0);
        assert_eq!(runtime.debug_physics_area(), Some(Rect { min: Vec2::new(-640.0, -360.0), max: Vec2::new(1920.0, 1080.0) }));
        let mut far_waited = 0;
        for _ in 0..4 {
            run(&mut runtime, 1, Input::default());
            let bodies = runtime.debug_bodies();
            let waiting = |id: NodeId| bodies.iter().find(|b| b.id == id).unwrap().waiting;
            assert!(!waiting(near), "in view, it never waits");
            far_waited += waiting(far) as usize;
        }
        assert_eq!(far_waited, 3, "out of view, it waits 3 frames out of 4");
    }

    #[test]
    fn the_projects_physics_settings_hold_in_every_scene() {
        let mut project = three_scenes("fn on_update(me, dt) { next_scene(); }");
        project.physics = PhysicsSettings { slow_offscreen: true, margin: 0.25, offscreen_interval: 8 };
        let mut runtime = Runtime::start(&project, 0);
        assert_eq!(runtime.physics_settings(), project.physics);
        run(&mut runtime, 1, Input::default());
        assert_eq!(runtime.scene_name(), "Scene 2");
        assert_eq!(runtime.physics_settings(), project.physics, "still after a scene change");

        // A change while the game runs holds in the next scenes too.
        let mut runtime = Runtime::start(&project, 0);
        let changed = PhysicsSettings { offscreen_interval: 2, ..project.physics };
        runtime.set_physics_settings(changed);
        run(&mut runtime, 1, Input::default());
        assert_eq!(runtime.scene_name(), "Scene 2");
        assert_eq!(runtime.physics_settings(), changed);
    }
}
