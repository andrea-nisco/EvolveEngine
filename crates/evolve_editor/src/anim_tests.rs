//! Clip playback in the editor (plan 6.3): one copy of the scene is kept and posed again in every frame, instead of a copy for each
//! frame. The picture must be the one a copy for each frame gives, whatever happens to the scene, the clip and the playhead.

use crate::app::EditorApp;
use evolve_engine::animation::{Property, Value};
use evolve_engine::geometry::Affine2;
use evolve_engine::path::Path;
use evolve_engine::scene::{NodeId, NodeKind};
use evolve_engine::style::Color;
use evolve_engine::Vec2;
use std::sync::Arc;

/// The editor with the built-in sample open (a clip with tracks for a pulsing sun and stars, a skinned tentacle moved by IK and its
/// target swaying), playing.
fn playing() -> EditorApp {
    let mut app = EditorApp::from_project(crate::sample::project(), crate::sample::OPEN_AT_START);
    assert!(!app.scene.clips.is_empty() && !app.scene.ik.is_empty(), "the sample has a clip and IK");
    app.anim.clip = 0;
    app.anim.playing = true;
    app
}

/// The preview is what a copy made for this frame is.
fn assert_preview_is_a_fresh_copy(app: &mut EditorApp, what: &str) {
    app.update_preview();
    let preview = app.anim.preview.clone().expect("a preview while playing");
    let fresh = app.playback_scene().expect("a copy while playing");
    assert!(*preview == fresh, "{what}: the preview is not what a new copy posed at {} is", app.anim.time);
}

/// The id of a shape that is not skinned (the pose can give it another outline).
fn a_plain_shape(app: &EditorApp) -> NodeId {
    app.scene
        .all_ids()
        .into_iter()
        .find(|&id| matches!(app.scene.find(id).map(|n| &n.kind), Some(NodeKind::Shape(s)) if s.skin.is_none() && s.fill.is_some()))
        .expect("a plain shape")
}

#[test]
fn the_preview_is_what_a_copy_for_each_frame_is_at_any_time_in_any_order() {
    let mut app = playing();
    for (step, time) in [0.0f32, 0.7, 1.3, 0.2, 2.9, 1.0, 1.0, 0.0, 2.4, 0.05, 1.5, 0.0].into_iter().enumerate() {
        app.anim.time = time;
        assert_preview_is_a_fresh_copy(&mut app, &format!("step {step}"));
    }
}

#[test]
fn the_copy_is_made_once_and_the_same_one_is_posed_in_every_frame() {
    let mut app = playing();
    app.update_preview();
    let first = app.anim.preview.clone().unwrap();
    let epoch = first.journal_epoch();
    drop(first);
    for step in 1..=20 {
        app.anim.time = step as f32 * 0.1;
        let before = Arc::as_ptr(app.anim.preview.as_ref().unwrap());
        app.update_preview();
        let preview = app.anim.preview.as_ref().unwrap();
        assert_eq!(Arc::as_ptr(preview), before, "step {step}: the copy was made again");
        assert_eq!(preview.journal_epoch(), epoch, "step {step}: the scene the renderer sees is another scene");
    }
    // Stopping drops it, and playing again makes a new one.
    app.anim.playing = false;
    app.update_preview();
    assert!(app.anim.preview.is_none());
    app.anim.playing = true;
    app.update_preview();
    assert_ne!(app.anim.preview.as_ref().unwrap().journal_epoch(), epoch);
}

/// Everything the copy was made from, changing while it plays: the scene (an edit, undo), the clip (a key added, a track taken away),
/// the IK constraints.
#[test]
fn the_copy_is_made_again_when_the_scene_the_clip_or_the_constraints_change() {
    let mut app = playing();
    app.anim.time = 1.1;
    assert_preview_is_a_fresh_copy(&mut app, "before anything");
    let epoch = app.anim.preview.as_ref().unwrap().journal_epoch();

    // The scene is edited: a node moves (one that no track moves).
    let shape = a_plain_shape(&app);
    app.scene.find_mut(shape).unwrap().set_transform(Affine2::from_translation(Vec2::new(5.0, 7.0)));
    assert_preview_is_a_fresh_copy(&mut app, "a node moved");
    assert_ne!(app.anim.preview.as_ref().unwrap().journal_epoch(), epoch, "the copy was made again");

    // The clip gets a new track (a node starts to move, to fade and to change its outline), and then loses it again.
    let epoch = app.anim.preview.as_ref().unwrap().journal_epoch();
    let before = app.scene.find(shape).unwrap().clone();
    let outline = match &before.kind {
        NodeKind::Shape(s) => s.path.clone(),
        _ => unreachable!(),
    };
    let wide = outline.transformed(&Affine2::from_scale(Vec2::new(1.8, 0.6)));
    let clip = &mut app.scene.clips[0];
    clip.set_key(shape, Property::Opacity, 1.5, Value::Float(0.3), Some(Value::Float(1.0)));
    clip.set_key(shape, Property::Fill, 2.0, Value::Color(Color::rgb(250, 20, 20)), Some(Value::Color(Color::rgb(10, 10, 10))));
    clip.set_key(shape, Property::Shape, 2.0, Value::Path(wide), Some(Value::Path(outline)));
    for time in [0.0, 1.0, 1.7, 2.0, 2.6, 0.4] {
        app.anim.time = time;
        assert_preview_is_a_fresh_copy(&mut app, &format!("a morph, a fade and a color at {time}"));
    }
    assert_ne!(app.anim.preview.as_ref().unwrap().journal_epoch(), epoch);
    app.scene.clips[0].tracks.retain(|t| t.node != shape);
    for time in [2.0, 0.4] {
        app.anim.time = time;
        assert_preview_is_a_fresh_copy(&mut app, "the tracks taken away");
        let node = app.anim.preview.as_ref().unwrap().find(shape).unwrap().clone();
        assert_eq!((node.opacity, node.transform), (1.0, Affine2::from_translation(Vec2::new(5.0, 7.0))), "nothing of the old pose is left on it");
    }

    // The IK constraints: one is taken away.
    let removed = app.scene.ik.remove(0);
    assert_preview_is_a_fresh_copy(&mut app, "an IK constraint taken away");
    app.scene.ik.push(removed);
    assert_preview_is_a_fresh_copy(&mut app, "and given back");

    // Another clip, and the same clip edited in place.
    app.scene.clips.push(evolve_engine::animation::Clip::new("empty"));
    app.anim.clip = 1;
    assert_preview_is_a_fresh_copy(&mut app, "another clip");
    app.anim.clip = 0;
    assert_preview_is_a_fresh_copy(&mut app, "back to the first");
    let _ = (Path::default(), Color::BLACK);
}

/// The IK solver starts from where the bones are. A copy that is posed again must start every frame from the bones of the scene, as a
/// new copy does, and not from where the last frame left them: going back and forth must give the same pose at the same time.
#[test]
fn the_ik_chains_start_from_the_scene_in_every_frame() {
    let mut app = playing();
    let mut seen = Vec::new();
    for time in [0.3f32, 2.2, 0.3, 1.4, 2.2, 0.3] {
        app.anim.time = time;
        app.update_preview();
        let preview = app.anim.preview.clone().unwrap();
        let bones: Vec<_> = app.scene.bone_ids().into_iter().map(|id| preview.find(id).unwrap().transform).collect();
        seen.push((time, bones));
    }
    for (time, bones) in &seen {
        let first = seen.iter().find(|(t, _)| t == time).unwrap();
        assert!(first.1 == *bones, "the bones at {time} are not where they were the first time");
    }
}
