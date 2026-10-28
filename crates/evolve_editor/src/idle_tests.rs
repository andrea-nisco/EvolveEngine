//! An editor that nobody touches must not change the scene.
//!
//! The panels run on every frame. A panel that hands `&mut node.field` to a widget would count as a change on every
//! frame (see `evolve_engine::revision`), which would make the undo check and the GPU cache work all the time. This
//! runs the real panels (including the Scene and Game views), headless, for every node of the sample project and for a few
//! selections, with no input, and checks that the scene's change stamps do not move.

use crate::app::EditorApp;
use eframe::egui;
use evolve_engine::scene::NodeId;

fn frame(app: &mut EditorApp, ctx: &egui::Context) {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 1400.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        app.objects_pane(ui);
        app.inspector_pane(ui);
        app.assets_contents(ui);
        app.scenes_pane(ui);
        app.anim.timeline_visible = true;
        app.timeline_contents(ui);
        // The Scene and Game views, with the renderer they use (the CPU renderer here; the GPU renderer in `gpu_view_tests`).
        ui.allocate_ui(egui::vec2(480.0, 380.0), |ui| app.canvas_ui(ui));
        ui.allocate_ui(egui::vec2(480.0, 380.0), |ui| app.game_pane(ui));
    });
    output.textures_delta.clear(); // there is no graphics card to hand them to
}

fn assert_quiet_while_selected(app: &mut EditorApp, ctx: &egui::Context, selection: Vec<NodeId>, renaming: Option<NodeId>, what: &str) {
    app.selection = selection;
    app.tree.renaming = renaming;
    frame(app, ctx); // the first frame lays things out
    let before = app.scene.revision();
    for _ in 0..3 {
        frame(app, ctx);
    }
    let changed = app.scene.changed_since(&before);
    assert!(changed.is_empty(), "{what}: idle frames changed the scene ({changed:?}); a panel is writing to a node on every frame");
    assert_eq!(app.scene, app.history.committed().clone(), "{what}: idle frames changed the content");
}

#[test]
fn idle_frames_do_not_change_the_scene_whatever_is_selected() {
    let project = crate::sample::project();
    for open in 0..project.scenes.len() {
        let mut app = EditorApp::from_project(project.clone(), open);
        let ctx = egui::Context::default();
        let ids = app.scene.all_ids();
        assert!(!ids.is_empty());
        for id in &ids {
            assert_quiet_while_selected(&mut app, &ctx, vec![*id], None, &format!("scene {open}, node {id:?}"));
        }
        assert_quiet_while_selected(&mut app, &ctx, ids.clone(), None, &format!("scene {open}, everything selected"));
        assert_quiet_while_selected(&mut app, &ctx, Vec::new(), None, &format!("scene {open}, nothing selected"));
        if let Some(&first) = ids.first() {
            assert_quiet_while_selected(&mut app, &ctx, vec![first], Some(first), &format!("scene {open}, renaming {first:?}"));
        }
    }
}
