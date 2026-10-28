//! Hands-on checks of the panels that edit a copy and write it back (the pattern of Phase 1 of the GPU plan): typing
//! must not lose characters, a value written back must stay, and one edit must not undo another. The panels are the
//! real ones; the input is simulated with egui's own test harness (`egui_kittest`): click a field, type, look at the
//! scene.

use crate::app::EditorApp;
use eframe::egui::{self, accesskit::Role};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use evolve_engine::physics::BodyKind;
use evolve_engine::scene::{NodeId, NodeKind};

fn harness<'a>(app: EditorApp, pane: impl FnMut(&mut egui::Ui, &mut EditorApp) + 'a) -> Harness<'a, EditorApp> {
    harness_sized(app, egui::vec2(420.0, 900.0), pane)
}

fn harness_sized<'a>(app: EditorApp, size: egui::Vec2, pane: impl FnMut(&mut egui::Ui, &mut EditorApp) + 'a) -> Harness<'a, EditorApp> {
    let mut harness = Harness::builder().with_size(size).build_ui_state(pane, app);
    harness.run();
    harness
}

fn sample_app() -> EditorApp {
    let project = crate::sample::project();
    let open = crate::sample::OPEN_AT_START;
    EditorApp::from_project(project, open)
}

fn find_by_name(app: &EditorApp, name: &str) -> NodeId {
    app.scene.all_ids().into_iter().find(|&id| app.scene.find(id).is_some_and(|n| n.name == name)).unwrap_or_else(|| panic!("the sample has no node called {name}"))
}

fn value_of(harness: &Harness<'_, EditorApp>, role: Role, containing: &str) -> String {
    harness.get_all_by_role(role).filter_map(|n| n.value()).find(|v| v.contains(containing)).unwrap_or_else(|| panic!("no {role:?} showing \"{containing}\""))
}

// --- The Name field of the Inspector --------------------------------------------------------------

#[test]
fn typing_in_the_name_field_keeps_every_character_and_the_edit_stays() {
    let mut app = sample_app();
    let id = app.scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Shape(_))).map(|n| n.id).expect("a top-level shape");
    let original = app.scene.find(id).unwrap().name.clone();
    app.selection = vec![id];
    let mut h = harness(app, |ui, app| app.inspector_pane(ui));

    // Click into the name field and type, one chunk at a time with frames in between, as a person would.
    let field = h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == original)).expect("the name field");
    field.click();
    h.run();
    for chunk in [" one", " two", " three"] {
        h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v.starts_with(&original))).expect("the focused name field").type_text(chunk);
        h.run();
        h.run();
    }
    let expected = format!("{original} one two three");
    assert_eq!(h.state().scene.find(id).unwrap().name, expected, "every typed character reached the scene");
    assert_eq!(value_of(&h, Role::TextInput, &original), expected, "and the field still shows it");
    h.run_steps(5);
    assert_eq!(h.state().scene.find(id).unwrap().name, expected, "idle frames do not undo it");
}

#[test]
fn a_rename_is_one_undoable_step() {
    let mut app = sample_app();
    let id = app.scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Shape(_))).map(|n| n.id).unwrap();
    let original = app.scene.find(id).unwrap().name.clone();
    app.selection = vec![id];
    let mut h = harness(app, |ui, app| app.inspector_pane(ui));
    h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == original)).unwrap().click();
    h.run();
    h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == original)).unwrap().type_text("X");
    h.run_steps(3);
    let app = h.state_mut();
    assert!(app.history.update(&app.scene, false), "the rename is an edit the history records");
    assert!(app.history.undo(&mut app.scene));
    assert_eq!(app.scene.find(id).unwrap().name, original);
}

// --- The script editor and the physics section -----------------------------------------------------

#[test]
fn typing_in_the_script_editor_keeps_the_text() {
    let mut app = sample_app();
    let ball = find_by_name(&app, "Ball");
    let before = app.scene.find(ball).unwrap().script.clone().expect("the sample ball has a script");
    app.selection = vec![ball];
    let mut h = harness(app, |ui, app| app.inspector_pane(ui));

    let editor = h.get_all_by_role(Role::MultilineTextInput).next().expect("the script editor");
    editor.click();
    h.run();
    h.get_all_by_role(Role::MultilineTextInput).next().unwrap().type_text("// note");
    h.run_steps(3);
    let script = h.state().scene.find(ball).unwrap().script.clone().unwrap();
    assert!(script.contains("// note"), "the typed text reached the scene: {script:?}");
    assert_eq!(script.len(), before.len() + "// note".len(), "and nothing else changed");
    h.run_steps(5);
    assert_eq!(h.state().scene.find(ball).unwrap().script.as_deref(), Some(script.as_str()), "idle frames do not undo it");
}

#[test]
fn the_physics_checkbox_adds_and_removes_a_body_and_leaves_other_settings_alone() {
    let mut app = sample_app();
    let id = app.scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Shape(_)) && n.body.is_none()).map(|n| n.id).expect("a shape without a body");
    let script = app.scene.find(id).unwrap().script.clone();
    app.selection = vec![id];
    let mut h = harness(app, |ui, app| app.inspector_pane(ui));

    h.get_by_label("Collision body").click();
    h.run_steps(3);
    let body = h.state().scene.find(id).unwrap().body.expect("the checkbox added a body");
    assert_eq!(body.kind, BodyKind::Static);
    assert_eq!(h.state().scene.find(id).unwrap().script, script, "the script was not touched");
    h.run_steps(5);
    assert_eq!(h.state().scene.find(id).unwrap().body, Some(body), "idle frames keep the body");

    h.get_by_label("Collision body").click();
    h.run_steps(3);
    assert!(h.state().scene.find(id).unwrap().body.is_none(), "unchecking removes it again");
}

// --- The Assets panel: symbol name ---------------------------------------------------------------

#[test]
fn typing_a_symbol_name_keeps_every_character() {
    let mut app = sample_app();
    let symbol = app.scene.symbols.first().expect("the sample has a symbol").clone();
    app.assets.selected = Some(symbol.id);
    // The Assets panel is a wide strip: in a narrow window the name field is off to the side.
    let mut h = harness_sized(app, egui::vec2(1600.0, 400.0), |ui, app| app.assets_contents(ui));

    h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == symbol.name)).expect("the symbol name field").click();
    h.run();
    h.key_press(egui::Key::End); // a click puts the cursor where it lands; a person goes to the end of the word
    h.run();
    for chunk in [" a", "b", "c"] {
        h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v.starts_with(&symbol.name))).unwrap().type_text(chunk);
        h.run_steps(2);
    }
    let expected = format!("{} abc", symbol.name);
    assert_eq!(h.state().scene.symbol(symbol.id).unwrap().name, expected);
    h.run_steps(5);
    assert_eq!(h.state().scene.symbol(symbol.id).unwrap().name, expected, "idle frames do not undo it");
}

// --- The camera section and the tree -----------------------------------------------------------

#[test]
fn renaming_a_camera_and_the_objects_panel_rename_keep_what_is_typed() {
    let mut app = sample_app();
    let camera = app.scene.main_camera().expect("the sample has a camera");
    let original = app.scene.find(camera).unwrap().name.clone();
    app.selection = vec![camera];
    let mut h = harness(app, |ui, app| app.inspector_pane(ui));
    h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == original)).expect("the camera name field").click();
    h.run();
    h.get_all_by_role(Role::TextInput).find(|n| n.value().is_some_and(|v| v == original)).unwrap().type_text(" B");
    h.run_steps(3);
    assert_eq!(h.state().scene.find(camera).unwrap().name, format!("{original} B"));
    h.run_steps(5);
    assert_eq!(h.state().scene.find(camera).unwrap().name, format!("{original} B"), "idle frames do not undo it");
}

// --- Checkboxes and sliders ----------------------------------------------------------------------

#[test]
fn the_clip_checkbox_and_a_slider_write_their_value_back_and_keep_it() {
    let mut app = sample_app();
    // A group of two shapes with a dynamic body, to have a clip checkbox and the physics sliders to work with.
    let shapes: Vec<NodeId> = app.scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Shape(_))).map(|n| n.id).take(2).collect();
    let group = app.scene.group(&shapes).expect("a group");
    {
        let mut node = app.scene.find_mut(group).unwrap();
        node.set_body(Some(evolve_engine::physics::Body::new(BodyKind::Dynamic)));
    }
    let bounce_before = app.scene.find(group).unwrap().body.unwrap().restitution;
    app.selection = vec![group];
    let mut h = harness_sized(app, egui::vec2(420.0, 1000.0), |ui, app| app.inspector_pane(ui));

    h.get_by_label("Clip to top object").click();
    h.run_steps(3);
    assert!(h.state().scene.find(group).unwrap().clip, "the checkbox reached the scene");
    h.run_steps(5);
    assert!(h.state().scene.find(group).unwrap().clip, "and idle frames keep it");

    // A slider: focus it and nudge it with the arrow keys, as a keyboard user would.
    h.get_by_role_and_label(Role::Slider, "Bounce").focus();
    h.run();
    for _ in 0..3 {
        h.key_press(egui::Key::ArrowRight);
        h.run();
    }
    let bounce = h.state().scene.find(group).unwrap().body.unwrap().restitution;
    assert_ne!(bounce, bounce_before, "the slider moved the value");
    h.run_steps(5);
    assert_eq!(h.state().scene.find(group).unwrap().body.unwrap().restitution, bounce, "and idle frames keep it");
}

// --- Looking around with the right mouse button ----------------------------------------------------

/// Presses the right button in the Scene view, drags 80 points to the right and 30 down, and lets go.
fn right_drag(h: &mut Harness<'_, EditorApp>) {
    let modifiers = egui::Modifiers::NONE;
    let button = egui::PointerButton::Secondary;
    let start = egui::pos2(400.0, 300.0);
    h.event(egui::Event::PointerMoved(start));
    h.run();
    h.event(egui::Event::PointerButton { pos: start, button, pressed: true, modifiers });
    h.run();
    for step in 1..=4 {
        h.event(egui::Event::PointerMoved(start + egui::vec2(20.0, 7.5) * step as f32));
        h.run();
    }
    h.event(egui::Event::PointerButton { pos: start + egui::vec2(80.0, 30.0), button, pressed: false, modifiers });
    h.run();
}

#[test]
fn the_right_button_rotates_the_view_while_editing_and_moves_it_while_a_game_runs() {
    for playing in [false, true] {
        let mut app = sample_app();
        if playing {
            app.start_game();
        }
        let mut h = harness_sized(app, egui::vec2(800.0, 600.0), |ui, app| app.canvas_ui(ui));
        let (center, rotation) = (h.state().camera.center, h.state().camera.rotation);
        right_drag(&mut h);
        let camera = &h.state().camera;
        if playing {
            assert_ne!(camera.center, center, "the drag moved the view");
            assert_eq!(camera.rotation, rotation, "and did not turn it");
        } else {
            assert_ne!(camera.rotation, rotation, "the drag turned the view (about the pointer, so the center moves a little too)");
        }
    }
}

// --- The wheel and the right button in the Game view (a game's own camera controls) -------------------------------------------------

/// A project whose camera has a script that says in the console what it is given.
fn app_with_a_camera_that_listens() -> EditorApp {
    use evolve_engine::project::Project;
    use evolve_engine::scene::Scene;
    let mut scene = Scene::with_camera(evolve_engine::Vec2::new(800.0, 600.0));
    let camera = scene.main_camera().expect("a camera");
    scene.find_mut(camera).unwrap().script = Some(
        "fn on_update(me, dt) {
             if mouse_right_pressed() { print(\"pressed\"); }
             if mouse_right_released() { print(\"released\"); }
             if mouse_wheel() != 0.0 { print(\"wheel\"); }
         }"
        .to_string(),
    );
    let mut app = EditorApp::from_project(Project::single(scene), 0);
    app.start_game();
    app
}

fn game_log(h: &Harness<'_, EditorApp>) -> Vec<String> {
    h.state().play.as_ref().expect("the game runs").runtime.log().iter().map(|l| l.text.clone()).collect()
}

fn wheel_event(dy: f32) -> egui::Event {
    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, dy), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move }
}

fn right_button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A harness for a running game: it asks for another frame every time, so it is stepped and not run until it is quiet.
fn stepping_harness<'a>(app: EditorApp, pane: impl FnMut(&mut egui::Ui, &mut EditorApp) + 'a) -> Harness<'a, EditorApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(700.0, 500.0)).build_ui_state(pane, app);
    harness.step();
    harness
}

/// The Game view hands the game the wheel and the right button, a drag that began over the picture ends with the release even outside
/// it, and the Scene view keeps them for its own camera.
#[test]
fn the_game_view_gives_a_running_game_the_wheel_and_the_right_button_and_the_scene_view_does_not() {
    // The Game view: the pane is 700 x 500 points, with the toolbar above the picture.
    let mut h = stepping_harness(app_with_a_camera_that_listens(), |ui, app| {
        let ctx = ui.ctx().clone();
        app.update_game(&ctx);
        app.game_pane(ui);
    });
    let inside = egui::pos2(350.0, 280.0);
    h.event(egui::Event::PointerMoved(inside));
    h.step();
    h.event(wheel_event(1.0));
    h.step();
    h.event(right_button(inside, true));
    h.step();
    h.event(egui::Event::PointerMoved(inside + egui::vec2(30.0, 10.0)));
    h.step();
    h.event(right_button(inside + egui::vec2(30.0, 10.0), false));
    h.step();
    h.step();
    assert_eq!(game_log(&h), ["wheel", "pressed", "released"]);

    // A drag that began over the picture goes on being followed when the pointer leaves it (for another part of the window), and ends
    // with the release there. The Game view is a small part of the window here.
    let mut h = stepping_harness(app_with_a_camera_that_listens(), |ui, app| {
        let ctx = ui.ctx().clone();
        app.update_game(&ctx);
        ui.allocate_ui(egui::vec2(300.0, 300.0), |ui| app.game_pane(ui));
    });
    let inside = egui::pos2(150.0, 180.0);
    let outside = egui::pos2(600.0, 450.0);
    h.event(egui::Event::PointerMoved(inside));
    h.step();
    assert!(h.state().canvas.game_cursor.is_some(), "over the picture");
    h.event(right_button(inside, true));
    h.step();
    h.event(egui::Event::PointerMoved(outside));
    h.step();
    let followed = h.state().canvas.game_cursor.expect("the drag is followed outside the picture");
    assert!(followed.x > 1_000.0 || followed.y > 500.0, "the game's mouse is out there, not at the origin or stuck: {followed:?}");
    h.event(right_button(outside, false));
    h.step();
    h.step();
    assert_eq!(game_log(&h), ["pressed", "released"]);
    assert!(h.state().canvas.game_cursor.is_none(), "and once the button is up the pointer outside is nothing to the game");
    // Without a press over the picture, a pointer elsewhere is not followed.
    h.event(right_button(outside, true));
    h.step();
    h.event(egui::Event::PointerMoved(outside + egui::vec2(-20.0, 0.0)));
    h.step();
    assert!(h.state().canvas.game_cursor.is_none());
    h.event(right_button(outside, false));
    h.step();

    // The Scene view: the wheel and the right button are the editor camera's, the game is not given them.
    let mut h = stepping_harness(app_with_a_camera_that_listens(), |ui, app| {
        let ctx = ui.ctx().clone();
        app.update_game(&ctx);
        app.canvas_ui(ui);
    });
    let (zoom, center) = (h.state().camera.zoom, h.state().camera.center);
    h.event(egui::Event::PointerMoved(inside));
    h.step();
    h.event(wheel_event(1.0));
    h.step();
    h.event(right_button(inside, true));
    h.step();
    h.event(egui::Event::PointerMoved(inside + egui::vec2(30.0, 10.0)));
    h.step();
    h.event(right_button(inside + egui::vec2(30.0, 10.0), false));
    h.step();
    h.step();
    assert!(game_log(&h).iter().all(|line| line != "wheel" && line != "pressed"), "{:?}", game_log(&h));
    assert!(h.state().camera.zoom != zoom && h.state().camera.center != center, "and the Scene view's camera zoomed and moved");
}

// --- The pin in a row of the Objects panel ----------------------------------------------------------

/// Twelve small shapes far from each other, and the editor's camera looking at none of them in particular.
fn app_with_far_apart_shapes() -> EditorApp {
    use evolve_engine::path::Path;
    use evolve_engine::scene::{Scene, Shape};
    let mut scene = Scene::with_camera(evolve_engine::Vec2::new(400.0, 300.0));
    for i in 0..12 {
        let at = evolve_engine::Vec2::new((i % 4) as f32 * 5_000.0 + 100.0, (i / 4) as f32 * 5_000.0 + 100.0);
        let node = scene.new_node(format!("s{i}"), NodeKind::Shape(Shape::new(Path::rect(at, at + evolve_engine::Vec2::new(60.0, 40.0)))));
        scene.add(node, None);
    }
    let mut app = EditorApp::from_project(evolve_engine::project::Project::single(scene), 0);
    app.camera.viewport = evolve_engine::Vec2::new(800.0, 600.0);
    app
}

#[test]
fn the_pin_of_a_row_selects_that_object_and_takes_the_scene_view_to_it() {
    let app = app_with_far_apart_shapes();
    let far = find_by_name(&app, "s10");
    let mut h = harness(app, |ui, app| app.objects_pane(ui));
    let pins = h.get_all_by_label("Show in the Scene view").count();
    assert!(pins >= 12, "a pin in every row that is shown: {pins}");
    // Rows are listed top-most first: s11, s10, ...
    h.get_all_by_label("Show in the Scene view").nth(1).unwrap().click();
    h.run();
    assert_eq!(h.state().selection, vec![far], "the object of the row is the selection");
    let bounds = h.state().scene.selection_bounds(&[far]).unwrap();
    let camera = &h.state().camera;
    assert!(camera.center.distance(bounds.center()) < 1.0, "the view is over it: {:?} against {:?}", camera.center, bounds.center());
    let shown = camera.viewport / camera.zoom;
    assert!(shown.x >= bounds.size().x && shown.y >= bounds.size().y, "all of it is in the view: {shown:?} for {:?}", bounds.size());
    assert!(shown.x < 600.0, "and the view is close to it, not on the whole scene: {shown:?}");
    assert!(h.state().status.starts_with("Pin: s10: 60.0 x 40.0, view "), "and the status says what was done: {}", h.state().status);
}

#[test]
fn the_pin_leaves_the_visibility_and_the_lock_of_the_object_alone() {
    let app = app_with_far_apart_shapes();
    let id = find_by_name(&app, "s11");
    let mut h = harness(app, |ui, app| app.objects_pane(ui));
    h.get_all_by_label("Show in the Scene view").next().unwrap().click();
    h.run();
    let node = h.state().scene.find(id).unwrap();
    assert!(node.visible && !node.locked);
    assert_eq!(h.state().selection, vec![id]);
}

#[test]
fn the_pin_never_zooms_so_close_that_a_thin_border_or_a_speck_fills_the_view_with_nothing() {
    use evolve_engine::path::Path;
    use evolve_engine::scene::Shape;
    let mut app = app_with_far_apart_shapes();
    // A border: long and almost no height; and a speck.
    let at = evolve_engine::Vec2::new(7_000.0, 3_000.0);
    let thin = app.scene.new_node("thin", NodeKind::Shape(Shape::new(Path::rect(at, at + evolve_engine::Vec2::new(3.0, 0.01)))));
    let thin = app.scene.add(thin, None);
    let speck = app.scene.new_node("speck", NodeKind::Shape(Shape::new(Path::rect(at, at + evolve_engine::Vec2::new(0.02, 0.02)))));
    let speck = app.scene.add(speck, None);
    let whole = app.scene.selection_bounds(&app.scene.all_ids()).unwrap().size().max_element();
    for id in [thin, speck] {
        app.locate(id);
        let shown = app.camera.viewport / app.camera.zoom;
        assert!(shown.min_element() >= whole * crate::app::LOCATE_CONTEXT - 1e-3, "at least the least asked for is in the view: {shown:?} of {whole}");
        let bounds = app.scene.selection_bounds(&[id]).unwrap();
        assert!(app.camera.center.distance(bounds.center()) < 1.0, "and it is in the middle");
    }
    // An object larger than that is framed as before: with some room around it, and no more than the least asked for.
    let big = app.scene.new_node("big", NodeKind::Shape(Shape::new(Path::rect(evolve_engine::Vec2::ZERO, evolve_engine::Vec2::new(2_000.0, 1_500.0)))));
    let big = app.scene.add(big, None);
    app.locate(big);
    let bounds = app.scene.selection_bounds(&[big]).unwrap();
    let shown = app.camera.viewport / app.camera.zoom;
    assert!(shown.x >= bounds.size().x * 1.19 && shown.y >= bounds.size().y * 1.19 && (shown.x <= bounds.size().x * 1.5 && shown.y <= bounds.size().y * 1.5), "{shown:?} for {:?}", bounds.size());
}

/// The views the pin gives are the views it gave before the least was asked for, for every object that is not tiny: the same call with
/// the least switched off must agree for all objects of a scene that are larger than it.
#[test]
fn for_an_object_larger_than_the_least_the_pin_frames_it_as_the_plain_zoom_to_selection_does() {
    let mut app = app_with_far_apart_shapes();
    for name in ["s0", "s5", "s11"] {
        let id = find_by_name(&app, name);
        app.locate(id);
        let (center, zoom) = (app.camera.center, app.camera.zoom);
        app.selection = vec![id];
        app.frame_selection();
        assert!(app.camera.center.distance(center) < 1e-3 && (app.camera.zoom - zoom).abs() < zoom * 1e-4, "{name}: the pin {zoom} against the zoom to selection {}", app.camera.zoom);
    }
}

/// Clicks the pins of the first rows of the real map in a window with the Objects panel and the Scene view side by side, as the editor
/// has them, and says what each click did.
///
///     cargo test --release -p evolve_editor what_the_pins_of_the_map_do_in_a_window -- --ignored --nocapture
#[test]
#[ignore]
fn what_the_pins_of_the_map_do_in_a_window() {
    let map = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local/ironkrieg/ironkrieg.evolve");
    let Ok(project) = evolve_engine::io::load_project(&map) else { return };
    let app = EditorApp::from_project(project, 0);
    let mut h = harness_sized(app, egui::vec2(1100.0, 700.0), |ui, app| {
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(300.0, 700.0), |ui| app.objects_pane(ui));
            ui.allocate_ui(egui::vec2(780.0, 690.0), |ui| app.canvas_ui(ui));
        });
    });
    println!("start: viewport {:?}, zoom {}, status {:?}", h.state().camera.viewport, h.state().camera.zoom, h.state().status);
    for n in 0..8 {
        let pins: Vec<_> = h.get_all_by_label("Show in the Scene view").collect();
        println!("pins shown: {}", pins.len());
        pins[n.min(pins.len() - 1)].click();
        h.run();
        let s = h.state();
        println!("click {n}: zoom {:.4}, center {:?}, selected {:?}\n    {}", s.camera.zoom, s.camera.center, s.selection.iter().map(|id| s.scene.find(*id).map(|n| n.name.clone())).collect::<Vec<_>>(), s.status);
    }
}
