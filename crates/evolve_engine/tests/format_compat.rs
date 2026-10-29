//! Old project files must keep opening. `tests/fixtures` holds one frozen file per format version
//! (see its README); these tests load them all. Never edit a fixture to make a test pass: fix the loader.

use evolve_engine::geometry::Vec2;
use evolve_engine::io::{self, FORMAT_VERSION};
use evolve_engine::physics::PhysicsSettings;
use evolve_engine::project::Project;
use evolve_engine::scene::{NodeKind, Scene};
use std::path::PathBuf;

fn fixture(version: u32) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("v{version}.evolve"))
}

fn load(version: u32) -> Project {
    io::load_project(&fixture(version)).unwrap_or_else(|e| panic!("the version {version} fixture no longer opens: {e}"))
}

fn sunset(project: &Project) -> &Scene {
    project.scenes.iter().find(|s| s.name == "Sunset").expect("the Sunset scene is in every fixture")
}

#[test]
fn there_is_a_fixture_for_every_format_version() {
    for version in 1..=FORMAT_VERSION {
        let path = fixture(version);
        assert!(path.is_file(), "format version {version} has no fixture: add tests/fixtures/v{version}.evolve (see its README)");
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["version"], version, "{} must be a version {version} file", path.display());
    }
}

#[test]
fn every_old_file_still_opens_with_its_content_intact() {
    for version in 1..=FORMAT_VERSION {
        let project = load(version);
        let scene = sunset(&project);

        let ball = scene.find(scene.find_by_name("Ball").expect("the ball")).unwrap();
        assert!(ball.body.is_some(), "v{version}: the ball keeps its physics body");
        let tentacle = scene.find(scene.find_by_name("Tentacle").expect("the tentacle")).unwrap();
        assert!(matches!(&tentacle.kind, NodeKind::Shape(_)), "v{version}: the tentacle is still a shape");
        assert!(ball.script.as_deref().is_some_and(|code| code.contains("on_")), "v{version}: the ball keeps its script");
        let idle = scene.clips.iter().find(|c| c.name == "Idle").expect("the Idle clip");
        assert_eq!((idle.duration, idle.fps), (3.0, 30.0), "v{version}: the clip keeps its timing");
        assert!(!idle.tracks.is_empty(), "v{version}: the clip keeps its keys");
        assert!(!scene.symbols.is_empty(), "v{version}: the symbols are kept");

        let view = scene.camera_view();
        assert_eq!(view.size, Vec2::new(1280.0, 720.0), "v{version}: the camera frames the old screen");
        assert_eq!(view.center, Vec2::new(640.0, 360.0), "v{version}: and is centred on it");
        assert_eq!(project.physics, PhysicsSettings::default(), "v{version}: a file without physics settings gets the defaults");
    }
}

#[test]
fn scenes_survive_the_upgrade_to_the_scene_list() {
    for version in [1, 2] {
        let project = load(version);
        assert_eq!((project.scenes.len(), project.start), (1, 0), "v{version}: one scene, which is the start scene");
    }
    let project = load(3);
    let names: Vec<_> = project.scenes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!((names, project.start), (vec!["Title", "Sunset"], 0));
}

#[test]
fn an_upgraded_file_saves_and_reopens_unchanged() {
    for version in 1..=FORMAT_VERSION {
        let project = load(version);
        let saved = io::project_to_string(&project).unwrap();
        assert_eq!(io::parse_project(&saved).unwrap(), project, "v{version}: save then open changes the project");
    }
}

/// If this fails, the file format changed. Bump `FORMAT_VERSION`, keep a genuine file of the old version as a new
/// fixture *before* the change, and make the loader upgrade it. See CONTRIBUTING.md.
#[test]
fn the_writer_still_writes_exactly_what_the_current_version_wrote() {
    let text = std::fs::read_to_string(fixture(FORMAT_VERSION)).unwrap();
    let written = io::project_to_string(&io::parse_project(&text).unwrap()).unwrap();
    let (old, new): (serde_json::Value, serde_json::Value) = (serde_json::from_str(&text).unwrap(), serde_json::from_str(&written).unwrap());
    assert_eq!(new, old, "the saved format of version {FORMAT_VERSION} changed without a version bump");
}
