//! Reading and writing projects: the native `.evolve` project format and SVG import.

use crate::geometry::{Affine2, Vec2};
use crate::path::{Anchor, Path, SubPath};
use crate::project::Project;
use crate::scene::{Node, NodeId, NodeKind, Scene, Shape};
use crate::style::{Color, FillRule, Gradient, GradientKind, GradientStop, LineCap, LineJoin, Paint, Stroke};
use serde::{Deserialize, Serialize};
use std::path::Path as FsPath;
use usvg::tiny_skia_path::PathSegment;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Version 2 added the camera object, version 3 several scenes per project. Older files are
/// upgraded when they are loaded: a version 1 or 2 file becomes a project with one scene.
pub const FORMAT_VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format: String,
    version: u32,
    /// All scenes in play order (version 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scenes: Vec<Scene>,
    /// Index into `scenes` of the scene the game starts with.
    #[serde(default)]
    start: usize,
    /// The single scene of a version 1 or 2 file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scene: Option<Scene>,
}

/// Projects up to this size (in bytes of compact JSON) are written indented, so small files stay easy to read
/// and to compare. Bigger ones are written on a single line: indenting a map of 21,000 shapes made it 382 MB instead
/// of about a tenth of that. Both are the same JSON.
const INDENT_LIMIT: usize = 1 << 20;

/// The project as JSON text (the `.evolve` file format).
pub fn project_to_string(project: &Project) -> Result<String> {
    let file = ProjectFile {
        format: "evolve".into(),
        version: FORMAT_VERSION,
        scenes: project.scenes.clone(),
        start: project.start,
        scene: None,
    };
    let compact = serde_json::to_string(&file)?;
    if compact.len() > INDENT_LIMIT {
        return Ok(compact);
    }
    Ok(serde_json::to_string_pretty(&file)?)
}

pub fn save_project(project: &Project, file: &FsPath) -> Result<()> {
    std::fs::write(file, project_to_string(project)?)?;
    Ok(())
}

pub fn load_project(file: &FsPath) -> Result<Project> {
    parse_project(&std::fs::read_to_string(file)?)
}

/// Reads project JSON text, checking that it is a project this version understands.
pub fn parse_project(text: &str) -> Result<Project> {
    let file: ProjectFile = serde_json::from_str(text)?;
    if file.format != "evolve" {
        return Err("not a Evolve project file".into());
    }
    if file.version > FORMAT_VERSION {
        return Err(format!("project was saved by a newer version ({})", file.version).into());
    }
    let mut scenes = file.scenes;
    if scenes.is_empty() {
        scenes.push(file.scene.ok_or("the project has no scenes")?);
    }
    for scene in &mut scenes {
        if scene.has_symbol_cycle() {
            return Err("a symbol contains itself".into());
        }
        scene.upgrade_legacy_view();
    }
    let start = file.start.min(scenes.len() - 1);
    let mut project = Project { scenes, start };
    // Scripts find scenes by name, so repair duplicate names from hand-edited files.
    for i in 1..project.scenes.len() {
        let name = project.scenes[i].name.clone();
        if project.scenes[..i].iter().any(|s| s.name == name) {
            project.rename_scene(i, &name);
        }
    }
    Ok(project)
}
// --- SVG import ------------------------------------------------------------------------------

/// Imports an SVG document as a new group on top of `scene`. Returns the group's id.
/// Supported: paths, groups, solid and gradient paints, strokes, opacity.
/// Ignored: images, text, filters, masks, clip paths, dashes.
pub fn import_svg(scene: &mut Scene, data: &[u8], name: &str) -> Result<NodeId> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
    let children = convert_children(scene, tree.root());
    let mut group = scene.new_node(name, NodeKind::Group(children));
    group.transform = convert_transform(tree.root().transform());
    Ok(scene.add(group, None))
}

fn convert_children(scene: &mut Scene, group: &usvg::Group) -> Vec<Node> {
    let mut nodes = Vec::new();
    for child in group.children() {
        match child {
            usvg::Node::Group(g) => {
                let children = convert_children(scene, g);
                let mut node = scene.new_node(display_name(g.id(), "Group"), NodeKind::Group(children));
                node.transform = convert_transform(g.transform());
                node.opacity = g.opacity().get();
                nodes.push(node);
            }
            usvg::Node::Path(p) if p.is_visible() => {
                let mut shape = Shape::new(convert_path(p.data()));
                shape.fill = p.fill().and_then(|f| convert_paint(f.paint(), f.opacity().get()));
                shape.fill_rule = match p.fill().map(|f| f.rule()) {
                    Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
                    _ => FillRule::NonZero,
                };
                shape.stroke = p.stroke().and_then(convert_stroke);
                nodes.push(scene.new_node(display_name(p.id(), "Path"), NodeKind::Shape(shape)));
            }
            _ => {}
        }
    }
    nodes
}

fn display_name(id: &str, fallback: &str) -> String {
    if id.is_empty() { fallback.to_string() } else { id.to_string() }
}

fn convert_transform(t: usvg::Transform) -> Affine2 {
    Affine2::from_cols_array(&[t.sx, t.ky, t.kx, t.sy, t.tx, t.ty])
}

fn convert_path(data: &usvg::tiny_skia_path::Path) -> Path {
    let vec = |p: usvg::tiny_skia_path::Point| Vec2::new(p.x, p.y);
    let mut subpaths = Vec::new();
    let mut current: Option<SubPath> = None;

    fn finish(current: &mut Option<SubPath>, subpaths: &mut Vec<SubPath>) {
        if let Some(sub) = current.take().filter(|s| s.anchors.len() >= 2) {
            subpaths.push(sub);
        }
    }

    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                finish(&mut current, &mut subpaths);
                current = Some(SubPath { anchors: vec![Anchor::corner(vec(p))], closed: false });
            }
            PathSegment::LineTo(p) => {
                if let Some(sub) = &mut current {
                    sub.anchors.push(Anchor::corner(vec(p)));
                }
            }
            PathSegment::QuadTo(q, p) => {
                if let Some(sub) = &mut current {
                    // Raise the quadratic curve to a cubic one.
                    let (p0, q, p) = (sub.anchors.last().map_or(vec(p), |a| a.pos), vec(q), vec(p));
                    push_cubic(sub, p0 + (q - p0) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p);
                }
            }
            PathSegment::CubicTo(c1, c2, p) => {
                if let Some(sub) = &mut current {
                    push_cubic(sub, vec(c1), vec(c2), vec(p));
                }
            }
            PathSegment::Close => {
                if let Some(sub) = &mut current {
                    sub.closed = true;
                    // "M a ... L a Z" repeats the start point: fold the duplicate into the first anchor.
                    let first = sub.anchors[0].pos;
                    if sub.anchors.len() > 2 && sub.anchors.last().is_some_and(|a| a.pos == first) {
                        let last = sub.anchors.pop().unwrap();
                        sub.anchors[0].handle_in = last.handle_in;
                    }
                }
                finish(&mut current, &mut subpaths);
            }
        }
    }
    finish(&mut current, &mut subpaths);
    Path { subpaths }
}

fn push_cubic(sub: &mut SubPath, c1: Vec2, c2: Vec2, end: Vec2) {
    if let Some(last) = sub.anchors.last_mut() {
        last.handle_out = c1 - last.pos;
    }
    sub.anchors.push(Anchor { pos: end, handle_in: c2 - end, handle_out: Vec2::ZERO });
}

fn convert_paint(paint: &usvg::Paint, opacity: f32) -> Option<Paint> {
    let stops = |base: &usvg::BaseGradient| -> Vec<GradientStop> {
        base.stops()
            .iter()
            .map(|s| GradientStop {
                offset: s.offset().get(),
                color: Color::rgb(s.color().red, s.color().green, s.color().blue)
                    .with_alpha_scaled(s.opacity().get() * opacity),
            })
            .collect()
    };
    match paint {
        usvg::Paint::Color(c) => Some(Paint::Solid(Color::rgb(c.red, c.green, c.blue).with_alpha_scaled(opacity))),
        usvg::Paint::LinearGradient(g) => Some(Paint::Gradient(Gradient {
            kind: GradientKind::Linear { start: Vec2::new(g.x1(), g.y1()), end: Vec2::new(g.x2(), g.y2()) },
            stops: stops(g),
            transform: convert_transform(g.transform()),
        })),
        usvg::Paint::RadialGradient(g) => Some(Paint::Gradient(Gradient {
            kind: GradientKind::Radial { center: Vec2::new(g.cx(), g.cy()), radius: g.r().get() },
            stops: stops(g),
            transform: convert_transform(g.transform()),
        })),
        usvg::Paint::Pattern(_) => None,
    }
}

fn convert_stroke(stroke: &usvg::Stroke) -> Option<Stroke> {
    Some(Stroke {
        paint: convert_paint(stroke.paint(), stroke.opacity().get())?,
        width: stroke.width().get(),
        join: match stroke.linejoin() {
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
            _ => LineJoin::Miter,
        },
        cap: match stroke.linecap() {
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
            usvg::LineCap::Butt => LineCap::Butt,
        },
        miter_limit: stroke.miterlimit().get(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_projects_are_indented_and_big_ones_are_not_and_both_read_back_the_same() {
        let small = Project::new();
        let text = project_to_string(&small).unwrap();
        assert!(text.lines().count() > 10, "a small project stays easy to read");
        assert_eq!(parse_project(&text).unwrap(), small);

        // About 40 000 anchors is several megabytes of JSON, well past the limit.
        let anchors = (0..40_000).map(|i| Anchor::corner(Vec2::new(i as f32 * 0.25, (i % 97) as f32))).collect();
        let mut big = Project::new();
        let shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: false }] });
        let node = big.scenes[0].new_node("long line", NodeKind::Shape(shape));
        big.scenes[0].add(node, None);
        let text = project_to_string(&big).unwrap();
        assert!(text.len() > INDENT_LIMIT);
        assert_eq!(text.lines().count(), 1, "a big project is written on one line");
        assert_eq!(parse_project(&text).unwrap(), big);
    }

    /// A project as an older version wrote it: one scene under `scene`.
    fn as_old_file(project: &Project, version: u32) -> String {
        let mut json: serde_json::Value = serde_json::from_str(&project_to_string(project).unwrap()).unwrap();
        let scene = json["scenes"][0].take();
        let object = json.as_object_mut().unwrap();
        object.remove("scenes");
        object.remove("start");
        object.insert("scene".into(), scene);
        object.insert("version".into(), version.into());
        json.to_string()
    }

    #[test]
    fn project_round_trips_through_json() {
        let mut scene = Scene::default();
        let node = scene.new_node("rect", NodeKind::Shape(Shape::new(Path::rect(Vec2::ZERO, Vec2::splat(10.0)))));
        let id = scene.add(node, None);
        let mut clip = crate::animation::Clip::new("Idle");
        clip.set_key(id, crate::animation::Property::Opacity, 1.0, crate::animation::Value::Float(0.5), Some(crate::animation::Value::Float(1.0)));
        scene.clips.push(clip);
        let project = Project::single(scene);
        let file = std::env::temp_dir().join("evolve_round_trip_test.evolve");
        save_project(&project, &file).unwrap();
        assert_eq!(load_project(&file).unwrap(), project);
        std::fs::remove_file(file).ok();
    }

    #[test]
    fn several_scenes_keep_their_order_names_and_start() {
        let mut project = Project::new();
        project.add_scene();
        project.add_scene();
        project.rename_scene(1, "Level");
        project.start = 2;
        let loaded = parse_project(&project_to_string(&project).unwrap()).unwrap();
        assert_eq!(loaded, project);
        assert_eq!((loaded.scenes[1].name.as_str(), loaded.start), ("Level", 2));
    }

    #[test]
    fn a_version_2_project_becomes_a_project_with_one_scene() {
        let project = Project::single(Scene::with_camera(Vec2::new(800.0, 450.0)));
        let loaded = parse_project(&as_old_file(&project, 2)).unwrap();
        assert_eq!(loaded, project);
    }

    #[test]
    fn a_version_1_project_gets_a_camera_from_its_screen_size() {
        // Version 1 stored the screen size and color on the scene itself.
        let old = Scene { legacy_size: Some(Vec2::new(640.0, 360.0)), legacy_background: Some(Color::rgb(9, 8, 7)), ..Scene::default() };
        let text = as_old_file(&Project::single(old), 1);
        let project = parse_project(&text).unwrap();
        let view = project.scenes[0].camera_view();
        assert_eq!((view.size, view.center, view.background), (Vec2::new(640.0, 360.0), Vec2::new(320.0, 180.0), Color::rgb(9, 8, 7)));
        // The old fields are gone after the upgrade, and saving again does not add a second camera.
        assert!(project.scenes[0].legacy_size.is_none() && project.scenes[0].legacy_background.is_none());
        assert_eq!(parse_project(&project_to_string(&project).unwrap()).unwrap(), project);
    }

    #[test]
    fn duplicate_scene_names_in_a_file_are_repaired() {
        let mut project = Project::new();
        project.add_scene();
        project.scenes[1].name = "Scene 1".into(); // a hand-edited file could do this
        let loaded = parse_project(&project_to_string(&project).unwrap()).unwrap();
        assert_eq!((loaded.scenes[0].name.as_str(), loaded.scenes[1].name.as_str()), ("Scene 1", "Scene 1 2"));
    }

    #[test]
    fn files_that_are_not_projects_are_refused() {
        assert!(parse_project("{\"format\":\"other\",\"version\":3}").is_err());
        assert!(parse_project("{\"format\":\"evolve\",\"version\":99}").is_err());
        assert!(parse_project("{\"format\":\"evolve\",\"version\":3}").is_err(), "no scenes at all");
    }
    #[test]
    fn imports_a_simple_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
            <g id="art"><circle cx="50" cy="50" r="40" fill="url(#g)" stroke="black" stroke-width="4"/></g>
        </svg>"##;
        let mut scene = Scene::default();
        let id = import_svg(&mut scene, svg, "test").unwrap();
        let group = scene.find(id).unwrap();
        assert_eq!(group.children().len(), 1);
        let NodeKind::Group(inner) = &group.children()[0].kind else { panic!("expected group") };
        let NodeKind::Shape(shape) = &inner[0].kind else { panic!("expected shape") };
        assert!(matches!(shape.fill, Some(Paint::Gradient(_))));
        assert!(shape.stroke.is_some());
        assert!(!shape.path.subpaths.is_empty());
    }

    /// What the importer does today with SVG features the engine does not have (OPEN_POINTS.md, P10). It never fails;
    /// it flattens or drops them without telling the user. If the importer learns one of these, this test changes with it.
    #[test]
    fn svg_features_the_engine_lacks_are_flattened_or_dropped_without_failing() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="300">
  <defs>
    <clipPath id="cp"><circle cx="50" cy="50" r="20"/></clipPath>
    <mask id="mk"><rect width="400" height="300" fill="white"/></mask>
    <filter id="blur"><feGaussianBlur stdDeviation="5"/></filter>
    <pattern id="pat" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="5" height="5" fill="red"/></pattern>
    <radialGradient id="rg" cx="0.5" cy="0.5" r="0.5" fx="0.2" fy="0.2" spreadMethod="reflect"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></radialGradient>
  </defs>
  <rect id="dashed" width="30" height="30" fill="none" stroke="black" stroke-width="3" stroke-dasharray="4 2"/>
  <g id="clipped" clip-path="url(#cp)"><rect id="insideclip" x="30" y="30" width="80" height="80" fill="orange"/></g>
  <g id="masked" mask="url(#mk)"><rect id="insidemask" width="80" height="80" fill="purple"/></g>
  <g id="blurred" filter="url(#blur)"><rect id="insideblur" width="30" height="30" fill="teal"/></g>
  <rect id="patterned" width="30" height="30" fill="url(#pat)"/>
  <rect id="focal" width="30" height="30" fill="url(#rg)"/>
  <g id="blend" style="mix-blend-mode:multiply"><rect id="insideblend" width="30" height="30" fill="gray"/></g>
  <text id="label" x="10" y="200" font-size="20">Hello</text>
  <image id="photo" x="200" y="200" width="20" height="20" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="/>
</svg>"##;
        let mut scene = Scene::default();
        let root = import_svg(&mut scene, svg, "svg").expect("importing never fails because of an unsupported feature");
        let shape = |name: &str| match &scene.find(scene.find_by_name(name).unwrap_or_else(|| panic!("{name} was imported"))).unwrap().kind {
            NodeKind::Shape(shape) => shape.clone(),
            _ => panic!("{name} is not a shape"),
        };

        // A dashed outline becomes a solid one: the picture changes.
        assert_eq!(shape("dashed").stroke.map(|s| s.width), Some(3.0));
        // Clip paths, masks, filters and blend modes are ignored: the content arrives whole, unclipped, unmasked, sharp.
        for (group, inside) in [("clipped", "insideclip"), ("masked", "insidemask"), ("blurred", "insideblur"), ("blend", "insideblend")] {
            let group = scene.find(scene.find_by_name(group).unwrap()).unwrap();
            assert!(!group.clip && group.opacity == 1.0, "{} carries no trace of the effect", group.name);
            assert_eq!(group.children().len(), 1);
            assert_eq!(group.children()[0].name, inside);
        }
        // A pattern fill leaves a shape with no fill and no stroke: it becomes invisible.
        let patterned = shape("patterned");
        assert!(patterned.fill.is_none() && patterned.stroke.is_none());
        // A gradient's focal point and spread method are lost: it is a plain radial gradient that pads.
        assert!(matches!(shape("focal").fill, Some(Paint::Gradient(Gradient { kind: GradientKind::Radial { .. }, .. }))));
        // Text (no fonts are loaded) and images leave nothing behind.
        assert!(scene.find_by_name("label").is_none(), "text is dropped");
        assert!(scene.find(scene.find_by_name("photo").unwrap()).unwrap().children().is_empty(), "an image leaves an empty group");
        let _ = root;
    }
}
