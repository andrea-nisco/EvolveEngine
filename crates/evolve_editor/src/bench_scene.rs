//! Benchmark scenes that need no game files: `evolve_editor --gen-scene <kind> <out.evolve>`.
//!
//! `large` is a scene of about the size of a big strategy map: roughly 1 200 filled shapes with about 350 points each
//! and 20 000 stroked polylines of 12 points, 21 200 shapes and 660 000 points in total, in two groups (fills below,
//! lines on top). It is generated from a fixed seed, so every run gives the same scene.
//!
//! `illustration` is what an SVG-first engine draws every day, a landscape of about 8 000 nodes: cubic curves, linear
//! and radial gradients, about a third of the shapes semi-transparent, groups with opacity (clouds, inside a group that
//! has opacity itself), clip groups, and symbol instances with color overrides. Same fixed seed.
//!
//! `sprites` is what a game of many small moving things draws: a plain background and 3 000 instances of one symbol of three parts
//! (a body, an eye and a tail), in one group, each at its own place, size and turn. Nothing in it is see-through, so every frame of it
//! is flat but for the instances; the animated benchmark (plan 6.6) moves all of them every frame. Same fixed seed.

use evolve_engine::geometry::Vec2;
use evolve_engine::path::{Anchor, Path, SubPath};
use evolve_engine::project::Project;
use evolve_engine::scene::{Node, NodeId, NodeKind, Scene, Shape};
use evolve_engine::style::{Color, FillRule, Gradient, GradientKind, GradientStop, LineCap, LineJoin, Paint, Stroke};
use evolve_engine::symbol::Instance;
use evolve_engine::Affine2;

const WORLD: Vec2 = Vec2::new(10_000.0, 8_000.0);
const FILL_COLUMNS: usize = 40;
const FILL_ROWS: usize = 30;
const POINTS_PER_FILL: usize = 350;
const LINES: usize = 20_000;
const POINTS_PER_LINE: usize = 12;

/// Names accepted by `--gen-scene`.
pub const KINDS: &[&str] = &["large", "illustration", "sprites"];

/// How many instances `sprites` has, and the size of its world.
pub const SPRITES: usize = 3_000;
const SPRITE_WORLD: Vec2 = Vec2::new(3_200.0, 1_800.0);

pub fn generate(kind: &str) -> Option<Project> {
    match kind {
        "large" => Some(Project::single(large())),
        "illustration" => Some(Project::single(illustration())),
        "sprites" => Some(Project::single(sprites())),
        _ => None,
    }
}

/// A small deterministic random generator (no dependency, same numbers on every machine).
struct Random(u64);

impl Random {
    /// A number in `[0, 1)`.
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + self.next() * (high - low)
    }
}

fn large() -> Scene {
    let mut scene = Scene::with_camera(WORLD);
    scene.name = "Large benchmark scene".into();
    let mut random = Random(12345);

    // Filled shapes: a wobbly blob in every cell of a grid.
    let cell = Vec2::new(WORLD.x / FILL_COLUMNS as f32, WORLD.y / FILL_ROWS as f32);
    let mut fills = Vec::with_capacity(FILL_COLUMNS * FILL_ROWS);
    for row in 0..FILL_ROWS {
        for column in 0..FILL_COLUMNS {
            let center = Vec2::new((column as f32 + 0.5) * cell.x, (row as f32 + 0.5) * cell.y);
            let (wobble_a, wobble_b) = (random.range(2.0, 7.0), random.range(0.05, 0.25));
            let anchors = (0..POINTS_PER_FILL)
                .map(|i| {
                    let angle = i as f32 / POINTS_PER_FILL as f32 * std::f32::consts::TAU;
                    let radius = 0.5 * (1.0 + wobble_b * (angle * wobble_a).sin());
                    Anchor::corner(center + Vec2::new(angle.cos() * cell.x, angle.sin() * cell.y) * radius)
                })
                .collect();
            let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: true }] });
            shape.fill = Some(Paint::Solid(Color::rgb(random.range(60.0, 230.0) as u8, random.range(60.0, 230.0) as u8, random.range(60.0, 230.0) as u8)));
            shape.stroke = None;
            shape.fill_rule = FillRule::EvenOdd;
            fills.push(scene.new_node(format!("Fill {row}-{column}"), NodeKind::Shape(shape)));
        }
    }
    let group = scene.new_node("Fills", NodeKind::Group(fills));
    scene.add(group, None);

    // Stroked polylines: short random walks across the whole world, thick and thin alternating.
    let mut lines = Vec::with_capacity(LINES);
    for i in 0..LINES {
        let mut at = Vec2::new(random.range(0.0, WORLD.x), random.range(0.0, WORLD.y));
        let mut heading = random.range(0.0, std::f32::consts::TAU);
        let anchors = (0..POINTS_PER_LINE)
            .map(|_| {
                let anchor = Anchor::corner(at);
                heading += random.range(-0.6, 0.6);
                at += Vec2::from_angle(heading) * random.range(8.0, 40.0);
                anchor
            })
            .collect();
        let mut shape = Shape::new(Path { subpaths: vec![SubPath { anchors, closed: false }] });
        shape.fill = None;
        let (color, width) = if i % 4 == 0 { (Color::BLACK, 3.0) } else { (Color::rgba(0, 0, 0, 90), 1.0) };
        shape.stroke = Some(Stroke { paint: Paint::Solid(color), width, join: LineJoin::Round, cap: LineCap::Round, miter_limit: 4.0 });
        lines.push(scene.new_node(format!("Line {i}"), NodeKind::Shape(shape)));
    }
    let group = scene.new_node("Lines", NodeKind::Group(lines));
    scene.add(group, None);
    scene
}

// --- The illustration scene ----------------------------------------------------------------------

const ILLUSTRATION_WORLD: Vec2 = Vec2::new(4_000.0, 2_250.0);

impl Random {
    fn color(&mut self, low: f32, high: f32, alpha: u8) -> Color {
        Color::rgba(self.range(low, high) as u8, self.range(low, high) as u8, self.range(low, high) as u8, alpha)
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() * items.len() as f32) as usize % items.len()]
    }
}

/// A closed smooth blob (cubic curves) around `center`, wobbling by `wobble` (0 to 1).
fn blob(random: &mut Random, center: Vec2, radius: Vec2, points: usize, wobble: f32) -> Path {
    let anchors = (0..points)
        .map(|i| {
            let angle = i as f32 / points as f32 * std::f32::consts::TAU;
            let r = 1.0 + random.range(-wobble, wobble);
            let pos = center + Vec2::new(angle.cos() * radius.x, angle.sin() * radius.y) * r;
            // Along the ellipse's tangent here, a third of the way to the next anchor.
            let tangent = Vec2::new(-angle.sin() * radius.x, angle.cos() * radius.y).normalize_or_zero();
            let reach = std::f32::consts::TAU / points as f32 * 0.33 * (radius.x + radius.y) * 0.5 * r;
            Anchor::smooth(pos, tangent * reach)
        })
        .collect();
    Path { subpaths: vec![SubPath { anchors, closed: true }] }
}

/// A ridge line across `width`, closed down to `bottom`, made of smooth curves.
fn ridge(random: &mut Random, width: f32, base: f32, height: f32, bottom: f32) -> Path {
    let steps = 24;
    let mut anchors: Vec<Anchor> = (0..=steps)
        .map(|i| {
            let x = i as f32 / steps as f32 * width;
            let wave = (x / width * std::f32::consts::TAU * 2.3).sin() * 0.5 + random.range(-0.5, 0.5);
            Anchor::smooth(Vec2::new(x, base - wave * height), Vec2::new(width / steps as f32 * 0.35, random.range(-0.2, 0.2) * height))
        })
        .collect();
    anchors.push(Anchor::corner(Vec2::new(width, bottom)));
    anchors.push(Anchor::corner(Vec2::new(0.0, bottom)));
    Path { subpaths: vec![SubPath { anchors, closed: true }] }
}

/// An open scribble of smooth curves wandering from `start`.
fn scribble(random: &mut Random, start: Vec2, points: usize, step: f32) -> Path {
    let mut at = start;
    let mut heading = random.range(0.0, std::f32::consts::TAU);
    let anchors = (0..points)
        .map(|_| {
            heading += random.range(-1.1, 1.1);
            let direction = Vec2::from_angle(heading);
            let anchor = Anchor::smooth(at, direction * step * 0.4);
            at += direction * step * random.range(0.6, 1.4);
            anchor
        })
        .collect();
    Path { subpaths: vec![SubPath { anchors, closed: false }] }
}

fn gradient(kind: GradientKind, stops: &[(f32, Color)]) -> Paint {
    Paint::Gradient(Gradient { kind, stops: stops.iter().map(|&(offset, color)| GradientStop { offset, color }).collect(), transform: Affine2::IDENTITY })
}

fn styled(path: Path, fill: Option<Paint>, stroke: Option<Stroke>) -> Shape {
    Shape { path, fill, stroke, fill_rule: FillRule::NonZero, skin: None }
}

fn outline(color: Color, width: f32, join: LineJoin, cap: LineCap) -> Stroke {
    Stroke { paint: Paint::Solid(color), width, join, cap, miter_limit: 4.0 }
}

fn sprites() -> Scene {
    let mut scene = Scene::with_camera(SPRITE_WORLD);
    scene.name = "Sprites benchmark scene".into();
    let mut random = Random(4242);
    let ground = styled(Path::rect(Vec2::ZERO, SPRITE_WORLD), Some(Paint::Solid(Color::rgb(34, 60, 52))), None);
    let ground = scene.new_node("Ground", NodeKind::Shape(ground));
    scene.add(ground, None);

    // The art: a round body with an outline, an eye in front of it and a curly tail behind.
    let tail_path = Path { subpaths: vec![SubPath { anchors: vec![Anchor::smooth(Vec2::new(-14.0, 0.0), Vec2::new(-6.0, -4.0)), Anchor::smooth(Vec2::new(-30.0, -6.0), Vec2::new(-6.0, 6.0))], closed: false }] };
    let tail = styled(tail_path, None, Some(outline(Color::rgb(200, 150, 60), 3.0, LineJoin::Round, LineCap::Round)));
    let body = styled(Path::ellipse(Vec2::ZERO, Vec2::new(16.0, 12.0)), Some(Paint::Solid(Color::rgb(230, 190, 70))), Some(outline(Color::rgb(90, 60, 20), 2.0, LineJoin::Round, LineCap::Round)));
    let eye = styled(Path::ellipse(Vec2::new(7.0, -3.0), Vec2::splat(3.0)), Some(Paint::Solid(Color::rgb(20, 20, 30))), None);
    let parts: Vec<NodeId> = [tail, body, eye].into_iter().enumerate().map(|(i, shape)| {
        let node = scene.new_node(format!("Part {i}"), NodeKind::Shape(shape));
        scene.add(node, None)
    }).collect();
    let (sprite, first) = scene.make_symbol(&parts, "Sprite").expect("a symbol from three shapes");
    scene.remove(first);

    let mut crowd = Vec::with_capacity(SPRITES);
    for i in 0..SPRITES {
        let at = Vec2::new(random.range(0.0, SPRITE_WORLD.x), random.range(0.0, SPRITE_WORLD.y));
        let mut node = scene.new_node(format!("Sprite {i}"), NodeKind::Instance(Instance { symbol: sprite, fill: None, stroke: None }));
        node.transform = Affine2::from_scale_angle_translation(Vec2::splat(random.range(0.7, 1.6)), random.range(0.0, std::f32::consts::TAU), at);
        crowd.push(node);
    }
    let crowd = scene.new_node("Crowd", NodeKind::Group(crowd));
    scene.add(crowd, None);
    scene
}

fn illustration() -> Scene {
    let world = ILLUSTRATION_WORLD;
    let mut scene = Scene::with_camera(world);
    scene.name = "Illustration benchmark scene".into();
    let mut random = Random(777);

    let node_of = |scene: &mut Scene, name: String, shape: Shape| scene.new_node(name, NodeKind::Shape(shape));
    let place = |node: &mut Node, at: Vec2, scale: f32, angle: f32| node.transform = Affine2::from_scale_angle_translation(Vec2::splat(scale), angle, at);

    // Sky: a vertical gradient, a sun with a soft glow.
    let sky = styled(
        Path::rect(Vec2::ZERO, world),
        Some(gradient(GradientKind::Linear { start: Vec2::ZERO, end: Vec2::new(0.0, world.y) }, &[(0.0, Color::rgb(24, 52, 120)), (0.55, Color::rgb(236, 142, 96)), (1.0, Color::rgb(255, 214, 150))])),
        None,
    );
    let sky = node_of(&mut scene, "Sky".into(), sky);
    scene.add(sky, None);
    let sun_at = Vec2::new(world.x * 0.72, world.y * 0.42);
    for (name, radius, rim, core) in [("Sun glow", 420.0, Color::rgba(255, 220, 140, 0), Color::rgba(255, 220, 140, 150)), ("Sun", 140.0, Color::rgb(255, 200, 90), Color::rgb(255, 250, 220))] {
        let paint = gradient(GradientKind::Radial { center: sun_at, radius }, &[(0.0, core), (1.0, rim)]);
        let disk = node_of(&mut scene, name.into(), styled(Path::ellipse(sun_at, Vec2::splat(radius)), Some(paint), None));
        scene.add(disk, None);
    }

    // Clouds: five overlapping ellipses each, drawn as one group with opacity; all clouds sit in a group that has
    // opacity too, so the opacity groups are nested.
    let mut clouds = Vec::new();
    for i in 0..60 {
        let at = Vec2::new(random.range(0.0, world.x), random.range(60.0, world.y * 0.45));
        let size = random.range(0.6, 1.8);
        let puffs: Vec<Node> = (0..5)
            .map(|k| {
                let offset = Vec2::new((k as f32 - 2.0) * 55.0 * random.range(0.8, 1.2), random.range(-25.0, 25.0));
                let radii = Vec2::new(random.range(60.0, 110.0), random.range(35.0, 60.0));
                node_of(&mut scene, format!("Puff {k}"), styled(Path::ellipse(offset, radii), Some(Paint::Solid(Color::rgb(255, 244, 236))), None))
            })
            .collect();
        let mut cloud = scene.new_node(format!("Cloud {i}"), NodeKind::Group(puffs));
        cloud.opacity = random.range(0.45, 0.9);
        place(&mut cloud, at, size, 0.0);
        clouds.push(cloud);
    }
    let mut sky_group = scene.new_node("Clouds", NodeKind::Group(clouds));
    sky_group.opacity = 0.92;
    scene.add(sky_group, None);

    // Ridges: layers of hills, farther ones paler and partly see-through.
    for (i, (base, height, top, bottom, alpha)) in [
        (0.55, 160.0, Color::rgb(120, 110, 170), Color::rgb(190, 150, 170), 200),
        (0.65, 200.0, Color::rgb(78, 96, 140), Color::rgb(140, 120, 150), 230),
        (0.76, 240.0, Color::rgb(40, 84, 96), Color::rgb(86, 120, 110), 255),
    ]
    .into_iter()
    .enumerate()
    {
        let path = ridge(&mut random, world.x, world.y * base, height, world.y);
        let paint = gradient(
            GradientKind::Linear { start: Vec2::new(0.0, world.y * (base - 0.1)), end: Vec2::new(0.0, world.y) },
            &[(0.0, Color { a: alpha, ..top }), (1.0, Color { a: alpha, ..bottom })],
        );
        let stroke = (i == 2).then(|| outline(Color::rgba(20, 50, 56, 160), 3.0, LineJoin::Round, LineCap::Round));
        let hill = node_of(&mut scene, format!("Ridge {i}"), styled(path, Some(paint), stroke));
        scene.add(hill, None);
    }

    // A tree symbol: a trunk and a crown with a radial gradient.
    let trunk_paint = Paint::linear(Vec2::new(-6.0, 0.0), Vec2::new(6.0, 0.0), Color::rgb(92, 60, 40), Color::rgb(60, 36, 28));
    let trunk = styled(Path::rect(Vec2::new(-6.0, 0.0), Vec2::new(6.0, 60.0)), Some(trunk_paint), Some(outline(Color::rgb(34, 22, 18), 2.0, LineJoin::Miter, LineCap::Butt)));
    let trunk = node_of(&mut scene, "Trunk".into(), trunk);
    let crown_path = blob(&mut random, Vec2::new(0.0, -30.0), Vec2::new(38.0, 52.0), 8, 0.08);
    let crown_paint = gradient(GradientKind::Radial { center: Vec2::new(-8.0, -44.0), radius: 56.0 }, &[(0.0, Color::rgb(130, 190, 90)), (1.0, Color::rgb(30, 96, 60))]);
    let crown = styled(crown_path, Some(crown_paint), Some(outline(Color::rgb(22, 60, 40), 2.5, LineJoin::Round, LineCap::Round)));
    let crown = node_of(&mut scene, "Crown".into(), crown);
    let ids: Vec<NodeId> = [trunk, crown].into_iter().map(|node| scene.add(node, None)).collect();
    let (tree, first) = scene.make_symbol(&ids, "Tree").expect("a symbol from two shapes");
    scene.remove(first);

    // A flower symbol: five petals and a center. Most instances override its colors.
    let mut parts = Vec::new();
    for k in 0..5 {
        let petal = styled(Path::ellipse(Vec2::new(0.0, -10.0), Vec2::new(5.0, 11.0)), Some(Paint::Solid(Color::rgb(240, 120, 150))), Some(outline(Color::rgb(120, 40, 70), 1.0, LineJoin::Round, LineCap::Round)));
        let mut petal = node_of(&mut scene, format!("Petal {k}"), petal);
        petal.transform = Affine2::from_angle(k as f32 / 5.0 * std::f32::consts::TAU);
        parts.push(scene.add(petal, None));
    }
    let heart = node_of(&mut scene, "Center".into(), styled(Path::ellipse(Vec2::ZERO, Vec2::splat(5.0)), Some(Paint::Solid(Color::rgb(250, 210, 80))), None));
    parts.push(scene.add(heart, None));
    let (flower, first) = scene.make_symbol(&parts, "Flower").expect("a symbol from six shapes");
    scene.remove(first);

    // A gem symbol: two overlapping semi-transparent parts, so overlaps show in the picture.
    let facet_a = styled(Path::ellipse(Vec2::new(-5.0, 0.0), Vec2::new(14.0, 9.0)), Some(Paint::Solid(Color::rgba(90, 170, 255, 150))), Some(outline(Color::rgba(20, 60, 140, 180), 1.5, LineJoin::Miter, LineCap::Butt)));
    let facet_a = node_of(&mut scene, "Facet A".into(), facet_a);
    let facet_b = styled(Path::ellipse(Vec2::new(5.0, 0.0), Vec2::new(14.0, 9.0)), Some(Paint::Solid(Color::rgba(255, 110, 190, 150))), Some(outline(Color::rgba(120, 20, 80, 180), 1.5, LineJoin::Miter, LineCap::Butt)));
    let facet_b = node_of(&mut scene, "Facet B".into(), facet_b);
    let parts: Vec<NodeId> = [facet_a, facet_b].into_iter().map(|node| scene.add(node, None)).collect();
    let (gem, first) = scene.make_symbol(&parts, "Gem").expect("a symbol from two shapes");
    scene.remove(first);

    // Forest, meadow and treasure: instances of the symbols over the lower part of the world.
    let palette = [Color::rgb(240, 120, 150), Color::rgb(250, 240, 120), Color::rgb(160, 130, 240), Color::rgb(255, 255, 255), Color::rgb(120, 220, 230)];
    let mut forest = Vec::new();
    for i in 0..1_200 {
        let at = Vec2::new(random.range(0.0, world.x), random.range(world.y * 0.58, world.y));
        let depth = (at.y - world.y * 0.55) / (world.y * 0.45);
        let mut node = scene.new_node(format!("Tree {i}"), NodeKind::Instance(Instance { symbol: tree, fill: None, stroke: None }));
        place(&mut node, at, 0.7 + depth * 1.8 + random.range(-0.1, 0.1), random.range(-0.08, 0.08));
        forest.push(node);
    }
    forest.sort_by(|a, b| a.transform.translation.y.total_cmp(&b.transform.translation.y)); // nearer trees on top
    let forest = scene.new_node("Forest", NodeKind::Group(forest));
    scene.add(forest, None);

    let mut meadow = Vec::new();
    for i in 0..1_500 {
        let at = Vec2::new(random.range(0.0, world.x), random.range(world.y * 0.62, world.y));
        let color = random.pick(&palette);
        let overridden = random.next() < 0.7;
        let instance = Instance { symbol: flower, fill: overridden.then_some(color), stroke: overridden.then_some(Color::rgb(color.r / 2, color.g / 2, color.b / 2)) };
        let mut node = scene.new_node(format!("Flower {i}"), NodeKind::Instance(instance));
        place(&mut node, at, random.range(0.8, 2.4), random.range(0.0, std::f32::consts::TAU));
        meadow.push(node);
    }
    let meadow = scene.new_node("Meadow", NodeKind::Group(meadow));
    scene.add(meadow, None);

    let mut treasure = Vec::new();
    for i in 0..600 {
        let at = Vec2::new(random.range(0.0, world.x), random.range(world.y * 0.7, world.y));
        let mut node = scene.new_node(format!("Gem {i}"), NodeKind::Instance(Instance { symbol: gem, fill: None, stroke: None }));
        node.opacity = if i % 3 == 0 { 0.7 } else { 1.0 };
        place(&mut node, at, random.range(0.8, 2.0), random.range(-0.5, 0.5));
        treasure.push(node);
    }
    let treasure = scene.new_node("Treasure", NodeKind::Group(treasure));
    scene.add(treasure, None);

    // Panels: clip groups, each a few gradient stripes seen through a blob; half of them have opacity as well.
    let mut panels = Vec::new();
    for i in 0..30 {
        let at = Vec2::new(random.range(200.0, world.x - 200.0), random.range(200.0, world.y - 200.0));
        let mut content = Vec::new();
        for k in 0..8 {
            let (first, second) = (random.color(40.0, 255.0, 255), random.color(40.0, 255.0, 160));
            let paint = Paint::linear(Vec2::new(0.0, -120.0), Vec2::new(0.0, 120.0), first, second);
            let left = -130.0 + k as f32 * 32.0;
            let mut stripe = node_of(&mut scene, format!("Stripe {k}"), styled(Path::rect(Vec2::new(left, -130.0), Vec2::new(left + 26.0, 130.0)), Some(paint), None));
            stripe.transform = Affine2::from_angle(0.5);
            content.push(stripe);
        }
        let radius = Vec2::new(random.range(70.0, 120.0), random.range(70.0, 120.0));
        let mask_path = blob(&mut random, Vec2::ZERO, radius, 7, 0.15);
        content.push(node_of(&mut scene, "Mask".into(), styled(mask_path, Some(Paint::Solid(Color::BLACK)), None)));
        let mut panel = scene.new_node(format!("Panel {i}"), NodeKind::Group(content));
        panel.clip = true;
        panel.opacity = if i % 2 == 0 { 0.85 } else { 1.0 };
        place(&mut panel, at, random.range(0.8, 1.6), random.range(-0.3, 0.3));
        panels.push(panel);
    }
    let panels = scene.new_node("Panels", NodeKind::Group(panels));
    scene.add(panels, None);

    // Rings: compound shapes (two outlines, even-odd) with a radial gradient, half of them see-through.
    let mut rings = Vec::new();
    for i in 0..400 {
        let at = Vec2::new(random.range(0.0, world.x), random.range(0.0, world.y));
        let radius = random.range(18.0, 70.0);
        let mut path = Path::ellipse(at, Vec2::splat(radius));
        path.subpaths.extend(Path::ellipse(at, Vec2::splat(radius * 0.55)).subpaths);
        let alpha = if i % 2 == 0 { 120 } else { 255 };
        let (inner, outer) = (random.color(120.0, 255.0, alpha), random.color(0.0, 160.0, alpha));
        let paint = gradient(GradientKind::Radial { center: at, radius }, &[(0.4, inner), (1.0, outer)]);
        let mut ring = styled(path, Some(paint), Some(outline(Color::rgba(30, 30, 50, 140), 1.5, LineJoin::Round, LineCap::Butt)));
        ring.fill_rule = FillRule::EvenOdd;
        rings.push(node_of(&mut scene, format!("Ring {i}"), ring));
    }
    let rings = scene.new_node("Rings", NodeKind::Group(rings));
    scene.add(rings, None);

    // Scribbles: cubic strokes of every width and join, a third of them see-through.
    let joins = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel];
    let caps = [LineCap::Butt, LineCap::Round, LineCap::Square];
    let mut scribbles = Vec::new();
    for i in 0..2_500 {
        let start = Vec2::new(random.range(0.0, world.x), random.range(0.0, world.y));
        let step = random.range(25.0, 90.0);
        let path = scribble(&mut random, start, 6, step);
        let width = 0.4 * random.range(0.0, 4.3).exp(); // 0.4 to about 30, mostly thin
        let alpha = if i % 3 == 0 { random.range(60.0, 140.0) as u8 } else { 255 };
        let color = random.color(0.0, 200.0, alpha);
        let mut stroke = outline(color, width, random.pick(&joins), random.pick(&caps));
        stroke.miter_limit = random.range(1.5, 8.0);
        scribbles.push(node_of(&mut scene, format!("Scribble {i}"), styled(path, None, Some(stroke))));
    }
    let scribbles = scene.new_node("Scribbles", NodeKind::Group(scribbles));
    scene.add(scribbles, None);

    // Grass in the foreground: small filled blades with corners.
    let mut grass = Vec::new();
    for i in 0..1_500 {
        let base = Vec2::new(random.range(0.0, world.x), random.range(world.y * 0.9, world.y));
        let lean = random.range(-14.0, 14.0);
        let height = random.range(30.0, 90.0);
        let anchors = [base + Vec2::new(-5.0, 0.0), base + Vec2::new(lean, -height), base + Vec2::new(5.0, 0.0)].map(Anchor::corner).to_vec();
        let green = Color::rgba(random.range(30.0, 90.0) as u8, random.range(100.0, 190.0) as u8, 50, if i % 3 == 0 { 170 } else { 255 });
        grass.push(node_of(&mut scene, format!("Blade {i}"), styled(Path { subpaths: vec![SubPath { anchors, closed: true }] }, Some(Paint::Solid(green)), None)));
    }
    let grass = scene.new_node("Grass", NodeKind::Group(grass));
    scene.add(grass, None);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(scene: &Scene) -> (usize, usize) {
        let (mut shapes, mut points) = (0, 0);
        fn walk(nodes: &[evolve_engine::scene::Node], shapes: &mut usize, points: &mut usize) {
            for node in nodes {
                match &node.kind {
                    NodeKind::Group(children) => walk(children, shapes, points),
                    NodeKind::Shape(shape) => {
                        *shapes += 1;
                        *points += shape.path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
                    }
                    _ => {}
                }
            }
        }
        walk(&scene.nodes, &mut shapes, &mut points);
        (shapes, points)
    }

    #[test]
    fn the_large_scene_has_the_promised_size_and_is_the_same_every_time() {
        let project = generate("large").unwrap();
        let (shapes, points) = counts(&project.scenes[0]);
        assert_eq!(shapes, 1_200 + 20_000);
        assert_eq!(points, 1_200 * 350 + 20_000 * 12);
        assert!(generate("large").unwrap() == project, "the same seed must give the same scene");
        assert!(generate("no such kind").is_none());
    }

    #[test]
    fn the_large_scene_is_drawn_inside_roughly_the_world() {
        let project = generate("large").unwrap();
        let bounds = project.scenes[0].selection_bounds(&project.scenes[0].all_ids()).expect("something to draw");
        // Lines are random walks and may leave the world a little; the fills stay inside it.
        assert!(bounds.min.x > -1_000.0 && bounds.min.y > -1_000.0 && bounds.max.x < WORLD.x + 1_000.0 && bounds.max.y < WORLD.y + 1_000.0, "{bounds:?}");
    }

    /// What the illustration scene contains, counted by walking it.
    #[derive(Default, Debug)]
    struct Features {
        shapes: usize,
        instances: usize,
        see_through: usize,
        cubic: usize,
        linear: usize,
        radial: usize,
        even_odd: usize,
        clip_groups: usize,
        opacity_groups: usize,
        nested_opacity_groups: usize,
        overridden_instances: usize,
        joins: [bool; 3],
        thinnest_stroke: f32,
        thickest_stroke: f32,
    }

    fn paint_is_see_through(paint: &Paint) -> bool {
        match paint {
            Paint::Solid(color) => color.a < 255,
            Paint::Gradient(gradient) => gradient.stops.iter().any(|stop| stop.color.a < 255),
        }
    }

    fn survey(scene: &Scene) -> Features {
        let mut f = Features { thinnest_stroke: f32::MAX, ..Default::default() };
        fn shape_see_through(shape: &Shape) -> bool {
            shape.fill.as_ref().is_some_and(paint_is_see_through) || shape.stroke.as_ref().is_some_and(|s| paint_is_see_through(&s.paint))
        }
        fn walk(scene: &Scene, nodes: &[Node], inside_opacity: bool, faded: bool, f: &mut Features) {
            for node in nodes {
                let faded_here = faded || node.opacity < 1.0;
                if node.opacity < 1.0 && node.is_group() {
                    f.opacity_groups += 1;
                    f.nested_opacity_groups += inside_opacity as usize;
                }
                f.clip_groups += node.clip as usize;
                match &node.kind {
                    NodeKind::Group(children) => walk(scene, children, inside_opacity || node.opacity < 1.0, faded_here, f),
                    NodeKind::Shape(shape) => {
                        f.shapes += 1;
                        f.see_through += (faded_here || shape_see_through(shape)) as usize;
                        f.cubic += shape.path.subpaths.iter().flat_map(|s| &s.anchors).any(|a| a.handle_in != Vec2::ZERO || a.handle_out != Vec2::ZERO) as usize;
                        f.even_odd += (shape.fill_rule == FillRule::EvenOdd) as usize;
                        for paint in shape.fill.iter().chain(shape.stroke.iter().map(|s| &s.paint)) {
                            match paint {
                                Paint::Gradient(Gradient { kind: GradientKind::Linear { .. }, .. }) => f.linear += 1,
                                Paint::Gradient(Gradient { kind: GradientKind::Radial { .. }, .. }) => f.radial += 1,
                                Paint::Solid(_) => {}
                            }
                        }
                        if let Some(stroke) = &shape.stroke {
                            f.joins[stroke.join as usize] = true;
                            f.thinnest_stroke = f.thinnest_stroke.min(stroke.width);
                            f.thickest_stroke = f.thickest_stroke.max(stroke.width);
                        }
                    }
                    NodeKind::Instance(instance) => {
                        f.instances += 1;
                        f.overridden_instances += (instance.fill.is_some() || instance.stroke.is_some()) as usize;
                        let symbol_see_through = scene.symbol(instance.symbol).is_some_and(|s| {
                            s.root.children().iter().any(|c| matches!(&c.kind, NodeKind::Shape(shape) if shape_see_through(shape)))
                        });
                        f.see_through += (faded_here || symbol_see_through) as usize;
                    }
                    NodeKind::Camera(_) => {}
                }
            }
        }
        walk(scene, &scene.nodes, false, false, &mut f);
        f
    }

    #[test]
    fn the_illustration_scene_has_every_kind_of_content_and_is_the_same_every_time() {
        let project = generate("illustration").unwrap();
        assert!(generate("illustration").unwrap() == project, "the same seed must give the same scene");
        let f = survey(&project.scenes[0]);
        println!("{f:#?}");

        let draw_items = f.shapes + f.instances;
        assert!((6_000..12_000).contains(&draw_items), "about 8 000 nodes, got {draw_items}");
        let share = f.see_through as f32 / draw_items as f32;
        assert!((0.25..0.45).contains(&share), "about 30% semi-transparent, got {:.0}%", share * 100.0);
        assert!(f.cubic > 1_000, "curves with handles: {}", f.cubic);
        assert!(f.linear > 10 && f.radial > 100, "gradients: {} linear, {} radial", f.linear, f.radial);
        assert!(f.even_odd > 100, "compound even-odd shapes: {}", f.even_odd);
        assert_eq!(f.clip_groups, 30);
        assert!(f.opacity_groups >= 60 && f.nested_opacity_groups >= 60, "opacity groups, nested: {} / {}", f.opacity_groups, f.nested_opacity_groups);
        assert!(f.instances >= 3_000 && f.overridden_instances > 500, "symbol instances with overrides: {} / {}", f.instances, f.overridden_instances);
        assert_eq!(f.joins, [true; 3], "miter, round and bevel joins all appear");
        assert!(f.thinnest_stroke < 0.5 && f.thickest_stroke > 10.0, "stroke widths {} to {}", f.thinnest_stroke, f.thickest_stroke);
    }

    #[test]
    fn the_illustration_scene_is_drawn_inside_roughly_the_world() {
        let project = generate("illustration").unwrap();
        let bounds = project.scenes[0].selection_bounds(&project.scenes[0].all_ids()).expect("something to draw");
        let world = ILLUSTRATION_WORLD;
        assert!(bounds.min.x > -500.0 && bounds.min.y > -500.0 && bounds.max.x < world.x + 500.0 && bounds.max.y < world.y + 500.0, "{bounds:?}");
    }
}
