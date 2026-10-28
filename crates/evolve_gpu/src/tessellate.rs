//! Turning the outline of one shape into triangles (plan D3 and D5, task 2.2).
//!
//! The tessellation happens in the shape's **own coordinates**, so moving, scaling or recoloring a shape never needs it
//! again. Only two things make it necessary again: the shape's geometry changed (its stamp, from Phase 1), or its
//! *effective scale* (camera zoom times the shape's own largest scale) moved into another power-of-two **bucket**, because
//! curves and round joins and caps are drawn as short straight pieces whose length depends on how big the shape is on the
//! screen. The tolerance is a quarter of a screen pixel divided by the scale of the bucket.
//!
//! A shape that is only straight lines with miter or bevel joins and butt or square caps looks the same at every scale
//! ([`depends_on_scale`]), so it is never tessellated again for a zoom change.
//!
//! Everything that decides how much work or memory a shape costs is clamped here (GPU_SAFETY.md B2): the tolerance, the
//! miter limit and the number of vertices of one shape.

use crate::vertex::{Mesh, Vertex};
use evolve_engine::path::{Cubic, SubPath};
use evolve_engine::Vec2;
use evolve_engine::scene::Shape;
use evolve_engine::style::{FillRule, LineCap, LineJoin};
use lyon::math::point;
use lyon::path::Path as LyonPath;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFillRule, FillTessellator, FillVertex, FillVertexConstructor, LineCap as LyonCap, LineJoin as LyonJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, StrokeVertexConstructor, VertexBuffers,
};

/// How coarse a picture may be: a quarter of a screen pixel.
pub const PIXEL_TOLERANCE: f32 = 0.25;
/// Strokes are flattened this much finer than fills: the offset edges of a wide stroke come out up to three times the
/// nominal tolerance away from the true edge (0.75 px instead of 0.25 on a 48-pixel stroke around a curve), which showed as
/// an edge half a pixel too far in at a deep zoom.
pub const STROKE_TOLERANCE_FACTOR: f32 = 0.35;
/// The smallest and largest tolerance, in the shape's own units. Below the first nothing can be seen (and the work would
/// explode); above the second a shape is a blob.
pub const MIN_TOLERANCE: f32 = 1e-4;
pub const MAX_TOLERANCE: f32 = 100.0;
/// The range of buckets, as powers of two of the effective scale: from 1/65 536 to 1 048 576 times.
pub const MIN_BUCKET: i32 = -16;
pub const MAX_BUCKET: i32 = 20;
/// Miter limits outside 1 to 100 make no sense (lyon rejects less than 1).
pub const MAX_MITER_LIMIT: f32 = 100.0;
/// How many times a shape that has too many vertices is tried again with a coarser tolerance, each four times coarser.
const MAX_COARSENINGS: u32 = 6;

/// What may be spent on one shape.
#[derive(Clone, Copy, Debug)]
pub struct TessLimits {
    /// Vertices of the fill, and of the stroke, of one shape. Above it the shape is made coarser, and if that does not
    /// help it is not drawn.
    pub max_vertices: usize,
}

impl Default for TessLimits {
    fn default() -> Self {
        Self { max_vertices: 4_000_000 }
    }
}

/// Something that went differently from what was asked. The picture is still produced.
#[derive(Clone, Debug, PartialEq)]
pub enum TessWarning {
    /// The path has a coordinate that is not a finite number; the shape is not drawn.
    NotFinite,
    /// The shape had too many vertices and was made this many times coarser (tolerance multiplied by the factor).
    Coarsened { factor: f32 },
    /// Even the coarsest try had more vertices than allowed; this part of the shape is not drawn.
    TooComplex { vertices: usize },
    /// The tessellator gave up (the text says why); this part of the shape is not drawn.
    Failed(String),
}

/// The fill and the stroke of one shape as triangles, each with row 0 (the draw list sets the rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeMesh {
    pub fill: Option<Mesh>,
    pub stroke: Option<Mesh>,
    pub warnings: Vec<TessWarning>,
}

impl ShapeMesh {
    pub fn vertex_count(&self) -> usize {
        self.fill.iter().chain(&self.stroke).map(|m| m.vertices.len()).sum()
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.fill.iter().chain(&self.stroke).map(Mesh::gpu_bytes).sum()
    }
}

/// The bucket of an effective scale: the smallest power of two that is at least the scale, so that the tolerance of the
/// bucket is never coarser than a quarter pixel anywhere in it. Not finite or not positive scales get the lowest bucket.
pub fn scale_bucket(effective_scale: f32) -> i32 {
    if !effective_scale.is_finite() || effective_scale <= 0.0 {
        return MIN_BUCKET;
    }
    (effective_scale.log2().ceil() as i32).clamp(MIN_BUCKET, MAX_BUCKET)
}

/// How far past its edges a bucket is kept (hysteresis): a shape stays in the bucket it was made for while the effective
/// scale is within 10% of it, so that a zoom that sits on a power of two does not make every shape again on every frame. The
/// tolerance then is at most 10% coarser than a quarter pixel, or the shape has a little more detail than it needs.
pub const BUCKET_HYSTERESIS: f32 = 0.1;

/// Whether a shape made for `bucket` is still good for `effective_scale`: the scale is inside the bucket, or just outside.
pub fn bucket_holds(bucket: i32, effective_scale: f32) -> bool {
    if !effective_scale.is_finite() || effective_scale <= 0.0 {
        return bucket == MIN_BUCKET;
    }
    let (low, high) = (2f32.powi(bucket - 1), 2f32.powi(bucket));
    effective_scale <= high * (1.0 + BUCKET_HYSTERESIS) && effective_scale >= low * (1.0 - BUCKET_HYSTERESIS) || scale_bucket(effective_scale) == bucket
}

/// A little less than the whole window, so that rounding in the factor can never lead outside it.
const WINDOW_SAFETY: f32 = 1.001;

/// By what factors `effective_scale` can change before a shape made for `bucket` stops being good for it (see [`bucket_holds`]):
/// returns `(low, high_inverse)`, and the shape still holds after a change by the factor `r` when `r >= low` and
/// `r * high_inverse <= 1`. The upper limit is stored as an inverse so that "no limit" is 0 (the lowest bucket has no lower limit
/// and the highest none above, as every scale beyond them is clamped into them). A scale that is not a positive number allows no change.
pub fn bucket_window(bucket: i32, effective_scale: f32) -> (f32, f32) {
    if !effective_scale.is_finite() || effective_scale <= 0.0 {
        return (1.0, 1.0);
    }
    let low = if bucket > MIN_BUCKET { 2f32.powi(bucket - 1) * (1.0 - BUCKET_HYSTERESIS) / effective_scale } else { 0.0 };
    let high_inverse = if bucket < MAX_BUCKET { effective_scale / (2f32.powi(bucket) * (1.0 + BUCKET_HYSTERESIS)) } else { 0.0 };
    (low * WINDOW_SAFETY, high_inverse * WINDOW_SAFETY)
}

/// The flattening tolerance of a bucket, in the shape's own units.
pub fn tolerance_for(bucket: i32) -> f32 {
    (PIXEL_TOLERANCE / 2f32.powi(bucket.clamp(MIN_BUCKET, MAX_BUCKET))).clamp(MIN_TOLERANCE, MAX_TOLERANCE)
}

/// How much wider than the width a stroke's triangles were made for it may get before they are made again: a round join or cap is
/// made of short straight pieces sized for the width, so a stroke that has become 10% wider has visibly coarser caps.
pub const WIDTH_HYSTERESIS: f32 = 0.1;
/// How much thinner than the width a stroke's triangles were made for it may get: no thinner than this share of it. The triangles are
/// the outline of the stroke at the width they were made for, pushed in or out along their normals by the shader; where the segments
/// of a path are shorter than the stroke is wide the stroker builds its inner corners differently for another width, so a mesh far from
/// the width it is drawn at is not the mesh a fresh one would be (a bevel-joined scribble made at width 3 and drawn at 0.5 shows it).
pub const THINNEST_SHARE: f32 = 0.25;

/// The width of a shape's stroke if it is drawn at all (a number above 0), else 0.
pub fn visible_stroke_width(shape: &Shape) -> f32 {
    shape.stroke.as_ref().map_or(0.0, |s| if s.width.is_finite() && s.width > 0.0 { s.width } else { 0.0 })
}

/// Whether triangles made when the stroke's width was `made` (see [`visible_stroke_width`]) are still good for the stroke the shape has
/// now. The vertex shader pushes every stroke vertex out by half the width in the shape table, so a small change of width needs no new
/// triangles (plan C1): they hold while the stroke is at most `WIDTH_HYSTERESIS` wider and at least `THINNEST_SHARE` of the width they
/// were made for; one outside that range is made again at its new width. (Triangles made for no stroke, `made` 0, hold only for a stroke
/// that is still not drawn: nothing fits a width above 0.)
pub fn stroke_width_holds(made: f32, shape: &Shape) -> bool {
    let now = visible_stroke_width(shape);
    now <= made * (1.0 + WIDTH_HYSTERESIS) && now >= made * THINNEST_SHARE
}

/// Whether the triangles of this shape change with zoom: it has a curve, or a round join or cap.
pub fn depends_on_scale(shape: &Shape) -> bool {
    let has_curve = shape.path.subpaths.iter().any(|sub| (0..sub.segment_count()).any(|i| !sub.segment(i).is_line()));
    let round_stroke = shape.stroke.as_ref().is_some_and(|s| s.width > 0.0 && (s.join == LineJoin::Round || s.cap == LineCap::Round));
    has_curve || round_stroke
}

/// Tessellates the fill and the stroke of `shape` for scale bucket `bucket`. Skinned shapes are bent by bones at draw
/// time and are not handled here (their bind-pose path would be wrong), the caller must not pass them.
pub fn tessellate_shape(shape: &Shape, bucket: i32, limits: &TessLimits) -> ShapeMesh {
    let mut result = ShapeMesh::default();
    if !path_is_finite(shape) {
        result.warnings.push(TessWarning::NotFinite);
        return result;
    }
    let tolerance = tolerance_for(bucket);
    let path = build_lyon_path(shape);
    if shape.fill.is_some() {
        result.fill = limited(&mut result.warnings, limits, tolerance, |tolerance| fill(&path, shape.fill_rule, tolerance));
    }
    if let Some(stroke) = shape.stroke.as_ref().filter(|s| s.width.is_finite() && s.width > 0.0) {
        let options = |tolerance: f32| {
            StrokeOptions::tolerance(tolerance * STROKE_TOLERANCE_FACTOR)
                .with_line_width(stroke.width)
                .with_line_join(match stroke.join {
                    LineJoin::Miter => LyonJoin::Miter,
                    LineJoin::Round => LyonJoin::Round,
                    LineJoin::Bevel => LyonJoin::Bevel,
                })
                .with_line_cap(match stroke.cap {
                    LineCap::Butt => LyonCap::Butt,
                    LineCap::Round => LyonCap::Round,
                    LineCap::Square => LyonCap::Square,
                })
                .with_miter_limit(if stroke.miter_limit.is_finite() { stroke.miter_limit.clamp(1.0, MAX_MITER_LIMIT) } else { 4.0 })
        };
        // With a miter join, tiny-skia cuts a corner flat (a bevel) when the miter would be longer than the limit, where lyon
        // clips the miter (and counts the limit as twice what the SVG rule says). So such corners are not left to lyon: the
        // stroke is cut into pieces there, and the little triangle that a bevel adds is made here (plan 3.12).
        let miter_limit = if stroke.miter_limit.is_finite() { stroke.miter_limit.clamp(1.0, MAX_MITER_LIMIT) } else { 4.0 };
        let pieces = if stroke.join == LineJoin::Miter { bevel_pieces(shape, miter_limit) } else { None };
        result.stroke = limited(&mut result.warnings, limits, tolerance, |tolerance| match &pieces {
            None => stroke_mesh(&path, &options(tolerance)),
            Some((pieces, wedges)) => {
                let mut whole = wedges.clone();
                for piece in pieces {
                    let mut options = options(tolerance);
                    if piece.start_flat {
                        options = options.with_start_cap(LyonCap::Butt);
                    }
                    if piece.end_flat {
                        options = options.with_end_cap(LyonCap::Butt);
                    }
                    whole.append(stroke_mesh(&piece.path, &options)?);
                }
                Ok(whole)
            }
        });
    }
    result
}

/// The silhouette of `shape` for use as a clip mask (plan 4.1): its fill rule's fill, whatever the shape's paints are (a shape
/// with no fill still clips, and its stroke does not count). Only `fill` of the result is set. Skinned shapes must not be passed.
pub fn tessellate_mask(shape: &Shape, bucket: i32, limits: &TessLimits) -> ShapeMesh {
    let mut result = ShapeMesh::default();
    if !path_is_finite(shape) {
        result.warnings.push(TessWarning::NotFinite);
        return result;
    }
    let path = build_lyon_path(shape);
    result.fill = limited(&mut result.warnings, limits, tolerance_for(bucket), |tolerance| fill(&path, shape.fill_rule, tolerance));
    result
}

/// Runs `make` with the tolerance, and with coarser ones while the result has too many vertices.
fn limited(
    warnings: &mut Vec<TessWarning>,
    limits: &TessLimits,
    tolerance: f32,
    make: impl Fn(f32) -> Result<Mesh, String>,
) -> Option<Mesh> {
    let mut factor = 1.0;
    for _ in 0..=MAX_COARSENINGS {
        let mesh = match make((tolerance * factor).min(MAX_TOLERANCE)) {
            Ok(mesh) => mesh,
            Err(why) => {
                warnings.push(TessWarning::Failed(why));
                return None;
            }
        };
        if mesh.vertices.len() <= limits.max_vertices {
            if factor > 1.0 {
                warnings.push(TessWarning::Coarsened { factor });
            }
            return (!mesh.is_empty()).then_some(mesh);
        }
        if (tolerance * factor) >= MAX_TOLERANCE {
            warnings.push(TessWarning::TooComplex { vertices: mesh.vertices.len() });
            return None;
        }
        factor *= 4.0;
    }
    warnings.push(TessWarning::TooComplex { vertices: limits.max_vertices.saturating_add(1) });
    None
}

fn path_is_finite(shape: &Shape) -> bool {
    shape.path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| {
        [a.pos, a.handle_in, a.handle_out].iter().all(|v| v.x.is_finite() && v.y.is_finite())
    })
}

fn build_lyon_path(shape: &Shape) -> LyonPath {
    let mut builder = LyonPath::builder();
    let p = |v: evolve_engine::Vec2| point(v.x, v.y);
    for sub in shape.path.subpaths.iter().filter(|s| s.anchors.len() >= 2) {
        builder.begin(p(sub.anchors[0].pos));
        for i in 0..sub.segment_count() {
            let segment = sub.segment(i);
            if segment.is_line() {
                builder.line_to(p(segment.p3));
            } else {
                builder.cubic_bezier_to(p(segment.c1), p(segment.c2), p(segment.p3));
            }
        }
        builder.end(sub.closed);
    }
    builder.build()
}

/// Part of a stroked path, made so that a corner that is cut flat ends one piece and starts the next.
struct Piece {
    path: LyonPath,
    /// This end is such a corner, and is cut flat whatever the stroke's cap is; otherwise it is a real end of the path.
    start_flat: bool,
    end_flat: bool,
}

/// The direction a curve leaves its end point in.
fn end_direction(c: &Cubic) -> Vec2 {
    [c.p3 - c.c2, c.p3 - c.c1, c.p3 - c.p0].into_iter().find(|d| *d != Vec2::ZERO).unwrap_or(Vec2::ZERO)
}

/// The direction a curve leaves its start point in.
fn start_direction(c: &Cubic) -> Vec2 {
    [c.c1 - c.p0, c.c2 - c.p0, c.p3 - c.p0].into_iter().find(|d| *d != Vec2::ZERO).unwrap_or(Vec2::ZERO)
}

/// Whether tiny-skia cuts the miter of this corner flat: arriving in direction `a` and leaving in direction `b`, the miter is
/// longer than `limit` times the stroke's width. This is Skia's own test, on half the angle between the two directions:
/// a bevel when `sin(half the angle between the segments) < 1 / limit`. A turn back on itself exactly is always cut.
fn is_cut_corner(a: Vec2, b: Vec2, limit: f32) -> bool {
    let (Some(a), Some(b)) = (a.try_normalize(), b.try_normalize()) else { return false };
    let sin_half_angle = ((1.0 + a.dot(b)) * 0.5).max(0.0).sqrt();
    sin_half_angle < 1.0 / limit
}

/// The triangle that a bevel adds at a corner: the corner point and the two outer corners of the strokes, drawn as vertices
/// that the vertex shader pushes out by half the width, like the vertices of lyon's strokes.
fn bevel_wedge(corner: Vec2, a: Vec2, b: Vec2) -> Option<Mesh> {
    let (a, b) = (a.try_normalize()?, b.try_normalize()?);
    let cross = a.perp_dot(b);
    if cross.abs() < 1e-6 {
        return None; // a turn straight back: the bevel is a straight cut through the corner, no area
    }
    // The outer side is the side away from where the path turns.
    let side = |d: Vec2| if cross > 0.0 { Vec2::new(d.y, -d.x) } else { Vec2::new(-d.y, d.x) };
    let vertex = |normal: Vec2| Vertex { point: corner.to_array(), normal: normal.to_array(), distance: 0.0, row: 0 };
    Some(Mesh { vertices: vec![vertex(Vec2::ZERO), vertex(side(a)), vertex(side(b))], indices: vec![0, 1, 2] })
}

/// The stroke of `shape` in pieces, with the bevel triangles of the corners that are cut flat, or `None` when no corner of
/// it is (the usual case: nothing changes and lyon strokes the whole path).
fn bevel_pieces(shape: &Shape, miter_limit: f32) -> Option<(Vec<Piece>, Mesh)> {
    let p = |v: Vec2| point(v.x, v.y);
    let build = |segments: &[Cubic]| {
        let mut builder = LyonPath::builder();
        builder.begin(p(segments[0].p0));
        for segment in segments {
            if segment.is_line() {
                builder.line_to(p(segment.p3));
            } else {
                builder.cubic_bezier_to(p(segment.c1), p(segment.c2), p(segment.p3));
            }
        }
        builder.end(false);
        builder.build()
    };
    let mut pieces = Vec::new();
    let mut wedges = Mesh::default();
    let mut plain = LyonPath::builder();
    let mut any_plain = false;
    let mut found = false;
    for sub in drawable_subpaths(shape) {
        let segments: Vec<Cubic> = (0..sub.segment_count()).map(|i| sub.segment(i)).collect();
        // Corner k is where segment k ends and the next one begins (for a closed path the last one wraps to the first).
        let corners = if sub.closed { segments.len() } else { segments.len().saturating_sub(1) };
        let directions = |k: usize| (end_direction(&segments[k]), start_direction(&segments[(k + 1) % segments.len()]));
        let cut: Vec<usize> = (0..corners)
            .filter(|&k| {
                let (a, b) = directions(k);
                is_cut_corner(a, b, miter_limit)
            })
            .collect();
        if cut.is_empty() {
            // Nothing special: this subpath goes into the one path of ordinary strokes.
            let mut subpath_shape = Shape::new(evolve_engine::path::Path { subpaths: vec![sub.clone()] });
            subpath_shape.fill_rule = shape.fill_rule;
            let single = build_lyon_path(&subpath_shape);
            plain.extend_from_paths(&[single.as_slice()]);
            any_plain = true;
            continue;
        }
        found = true;
        for &k in &cut {
            let (a, b) = directions(k);
            if let Some(wedge) = bevel_wedge(segments[k].p3, a, b) {
                wedges.append(wedge);
            }
        }
        // Cut at every such corner. A closed path is first turned around so that it starts right after one: then it is open,
        // with flat ends, and the cut there loses no ordinary corner.
        let mut order: Vec<Cubic> = segments.clone();
        let mut cuts: Vec<usize> = cut.clone();
        if sub.closed {
            let first = cut[0];
            order.rotate_left(first + 1);
            cuts = cut.iter().map(|&k| (k + segments.len() - first - 1) % segments.len()).collect();
            cuts.sort_unstable();
        }
        let mut start = 0;
        for (n, &end) in cuts.iter().enumerate() {
            // The segments start..=end make one piece. (For a closed path the last cut is its final segment.)
            pieces.push(Piece { path: build(&order[start..=end]), start_flat: sub.closed || n > 0, end_flat: true });
            start = end + 1;
        }
        if start < order.len() {
            pieces.push(Piece { path: build(&order[start..]), start_flat: true, end_flat: sub.closed });
        }
    }
    if !found {
        return None;
    }
    if any_plain {
        pieces.push(Piece { path: plain.build(), start_flat: false, end_flat: false });
    }
    Some((pieces, wedges))
}

struct FillCtor;

impl FillVertexConstructor<Vertex> for FillCtor {
    fn new_vertex(&mut self, vertex: FillVertex) -> Vertex {
        Vertex { point: vertex.position().to_array(), normal: [0.0, 0.0], distance: 0.0, row: 0 }
    }
}

struct StrokeCtor;

impl StrokeVertexConstructor<Vertex> for StrokeCtor {
    fn new_vertex(&mut self, vertex: StrokeVertex) -> Vertex {
        // `position() == position_on_path() + normal() * half_width`, which is what the vertex shader recomputes.
        Vertex { point: vertex.position_on_path().to_array(), normal: vertex.normal().to_array(), distance: vertex.advancement(), row: 0 }
    }
}

fn fill(path: &LyonPath, rule: FillRule, tolerance: f32) -> Result<Mesh, String> {
    let options = FillOptions::tolerance(tolerance).with_fill_rule(match rule {
        FillRule::NonZero => LyonFillRule::NonZero,
        FillRule::EvenOdd => LyonFillRule::EvenOdd,
    });
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    FillTessellator::new().tessellate_path(path, &options, &mut BuffersBuilder::new(&mut buffers, FillCtor)).map_err(|e| format!("fill: {e:?}"))?;
    Ok(Mesh { vertices: buffers.vertices, indices: buffers.indices })
}

fn stroke_mesh(path: &LyonPath, options: &StrokeOptions) -> Result<Mesh, String> {
    let mut buffers: VertexBuffers<Vertex, u32> = VertexBuffers::new();
    StrokeTessellator::new().tessellate_path(path, options, &mut BuffersBuilder::new(&mut buffers, StrokeCtor)).map_err(|e| format!("stroke: {e:?}"))?;
    Ok(Mesh { vertices: buffers.vertices, indices: buffers.indices })
}

/// A subpath is drawn (by lyon here, by tiny-skia on the CPU) only if it has two anchors or more.
pub fn drawable_subpaths(shape: &Shape) -> impl Iterator<Item = &SubPath> {
    shape.path.subpaths.iter().filter(|s| s.anchors.len() >= 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_engine::path::{Anchor, Path};
    use evolve_engine::style::{Color, Paint, Stroke};
    use evolve_engine::Vec2;

    fn filled(path: Path) -> Shape {
        let mut shape = Shape::new(path);
        shape.fill = Some(Paint::Solid(Color::BLACK));
        shape.stroke = None;
        shape
    }

    fn stroked(path: Path, width: f32, join: LineJoin, cap: LineCap) -> Shape {
        let mut shape = Shape::new(path);
        shape.fill = None;
        shape.stroke = Some(Stroke { paint: Paint::Solid(Color::BLACK), width, join, cap, miter_limit: 4.0 });
        shape
    }

    fn polyline(points: &[(f32, f32)], closed: bool) -> Path {
        Path { subpaths: vec![SubPath { anchors: points.iter().map(|&(x, y)| Anchor::corner(Vec2::new(x, y))).collect(), closed }] }
    }

    /// Total area of the triangles of a fill mesh.
    fn area(mesh: &Mesh) -> f32 {
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize].point);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    fn mesh_of(shape: &Shape, bucket: i32) -> ShapeMesh {
        tessellate_shape(shape, bucket, &TessLimits::default())
    }

    fn corner_pieces(points: &[(f32, f32)], closed: bool, limit: f32) -> Option<Vec<(bool, bool)>> {
        let shape = stroked(polyline(points, closed), 10.0, LineJoin::Miter, LineCap::Butt);
        bevel_pieces(&shape, limit).map(|(pieces, _)| pieces.iter().map(|p| (p.start_flat, p.end_flat)).collect())
    }

    #[test]
    fn only_corners_beyond_the_miter_limit_cut_a_stroke_into_pieces() {
        // A right angle and a 30 degree corner (miter 3.86 times the width) stay miters at the default limit of 4...
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)], false, 4.0), None);
        assert_eq!(corner_pieces(&[(190.0, 60.0), (190.0, 130.0), (150.0, 60.0)], false, 4.0), None);
        // ...a 23 degree corner (4.97 times) is cut flat, and so is a corner that is much sharper, or a turn straight back.
        assert_eq!(corner_pieces(&[(190.0, 60.0), (190.0, 130.0), (160.0, 60.0)], false, 4.0), Some(vec![(false, true), (true, false)]));
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (0.0, 10.0)], false, 4.0).map(|p| p.len()), Some(2));
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (40.0, 0.0), (40.0, 50.0)], false, 4.0), Some(vec![(false, true), (true, false)]));
        // The limit decides: the same 90 degree corner is a bevel at a limit of 1, and the 23 degree one is a miter at 6.
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)], false, 1.0).map(|p| p.len()), Some(2));
        assert_eq!(corner_pieces(&[(190.0, 60.0), (190.0, 130.0), (160.0, 60.0)], false, 6.0), None);
        // Two in a row: three pieces.
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (40.0, 0.0), (140.0, 0.0)], false, 4.0), Some(vec![(false, true), (true, true), (true, false)]));
        // In a closed path the cut makes it open, with flat ends only (here the closing corner, back along the first side,
        // is the one that is cut; the other two corners keep their miters).
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (30.0, 0.0)], true, 4.0), Some(vec![(true, true)]));
        // The corner that closes a path counts too.
        assert_eq!(corner_pieces(&[(0.0, 0.0), (100.0, 0.0), (50.0, 0.0)], true, 4.0).map(|p| p.len()), Some(2));
    }

    #[test]
    fn a_stroke_that_turns_back_ends_flat_and_does_not_make_a_spike() {
        // Out to x = 100 and back to x = 20 with a miter join: the end of the stroke must not go past x = 100.
        let reach = |points: &[(f32, f32)], join: LineJoin| {
            let mesh = mesh_of(&stroked(polyline(points, false), 10.0, join, LineCap::Butt), 0).stroke.expect("a stroke");
            mesh.vertices.iter().map(|v| v.point[0] + v.normal[0] * 5.0).fold(f32::MIN, f32::max)
        };
        let back = [(0.0, 0.0), (100.0, 0.0), (20.0, 0.0)];
        assert!(reach(&back, LineJoin::Miter) <= 100.001, "a spike of {}", reach(&back, LineJoin::Miter));
        // A round join makes a half circle past the turn, which is right and must stay.
        assert!(reach(&back, LineJoin::Round) > 104.0);
    }

    #[test]
    fn the_bevel_of_a_cut_corner_is_the_triangle_between_the_two_outer_corners() {
        // Right angle at (100, 0), walking right and then down, cut by a limit of 1: the path turns clockwise on a y-down
        // screen, so the outer side is up and to the right. The wedge is (100,0), (100,-5), (105,0).
        let wedge = bevel_wedge(Vec2::new(100.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)).expect("a wedge");
        let corners: Vec<[f32; 2]> = wedge.vertices.iter().map(|v| [v.point[0] + v.normal[0] * 5.0, v.point[1] + v.normal[1] * 5.0]).collect();
        assert_eq!(corners, vec![[100.0, 0.0], [100.0, -5.0], [105.0, 0.0]]);
        assert_eq!(wedge.indices, vec![0, 1, 2]);
        // A turn straight back has no area, so no wedge.
        assert!(bevel_wedge(Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)).is_none());
    }

    #[test]
    fn a_bucket_holds_a_little_past_both_edges_and_no_more() {
        // Bucket 1 is for scales in (1, 2]; with 10% more on each side it holds 0.9 to 2.2 (the lower edge is 2^0 less 10%).
        for scale in [0.91, 1.0, 1.5, 2.0, 2.19] {
            assert!(bucket_holds(1, scale), "{scale}");
        }
        for scale in [0.85, 2.25, 4.0] {
            assert!(!bucket_holds(1, scale), "{scale}");
        }
        // What belongs to the bucket by the plain rule always holds, whatever the margin, and nonsense scales hold only the lowest.
        assert!((-5..10).all(|b| bucket_holds(b, 2f32.powi(b)) && bucket_holds(b, 2f32.powi(b) * 0.75)));
        assert!(bucket_holds(MIN_BUCKET, f32::NAN) && !bucket_holds(0, f32::NAN) && !bucket_holds(0, -1.0));
    }

    #[test]
    fn a_filled_rectangle_is_two_triangles_with_no_normals() {
        let mesh = mesh_of(&filled(Path::rect(Vec2::ZERO, Vec2::new(10.0, 5.0))), 0);
        let fill = mesh.fill.expect("a fill");
        assert!(mesh.stroke.is_none() && mesh.warnings.is_empty());
        assert_eq!((fill.vertices.len(), fill.triangle_count()), (4, 2));
        assert!((area(&fill) - 50.0).abs() < 1e-4);
        assert!(fill.vertices.iter().all(|v| v.normal == [0.0, 0.0] && v.distance == 0.0 && v.row == 0));
        assert!(fill.indices.iter().all(|&i| (i as usize) < fill.vertices.len()), "indices start at zero");
    }

    #[test]
    fn nonzero_fills_a_hole_that_runs_the_same_way_and_even_odd_does_not() {
        let compound = |rule| {
            let mut path = Path::rect(Vec2::ZERO, Vec2::splat(10.0));
            path.subpaths.extend(Path::rect(Vec2::splat(3.0), Vec2::splat(7.0)).subpaths); // same winding as the outer one
            let mut shape = filled(path);
            shape.fill_rule = rule;
            shape
        };
        let nonzero = area(&mesh_of(&compound(FillRule::NonZero), 0).fill.unwrap());
        let even_odd = area(&mesh_of(&compound(FillRule::EvenOdd), 0).fill.unwrap());
        assert!((nonzero - 100.0).abs() < 1e-3, "{nonzero}");
        assert!((even_odd - (100.0 - 16.0)).abs() < 1e-3, "{even_odd}");
    }

    #[test]
    fn a_circle_has_more_triangles_at_a_finer_bucket_and_its_area_gets_closer() {
        let circle = filled(Path::ellipse(Vec2::ZERO, Vec2::splat(100.0)));
        // The engine's circles are four cubic curves, which are not exactly circles, so the finest mesh of the same shape
        // is the reference, not pi r squared.
        let exact = area(&mesh_of(&circle, 14).fill.unwrap());
        let mut last_count = 0;
        let mut last_error = f32::MAX;
        for bucket in [-2, 0, 2, 4, 6] {
            let fill = mesh_of(&circle, bucket).fill.unwrap();
            let error = (exact - area(&fill)).abs();
            assert!(fill.vertices.len() > last_count, "bucket {bucket}: {} vertices after {last_count}", fill.vertices.len());
            assert!(error < last_error, "bucket {bucket}: area error {error} after {last_error}");
            // The error is below what a quarter pixel of the bucket allows: the polygon is inside the circle by at most
            // the tolerance, so the area lacks at most about perimeter times tolerance.
            assert!(error <= 2.0 * std::f32::consts::PI * 100.0 * tolerance_for(bucket) * 1.1, "bucket {bucket}: {error}");
            (last_count, last_error) = (fill.vertices.len(), error);
        }
    }

    #[test]
    fn buckets_are_powers_of_two_with_a_quarter_pixel_tolerance_and_clamped() {
        assert_eq!([0.9, 1.0, 1.1, 2.0, 3.9, 4.0, 4.1].map(scale_bucket), [0, 0, 1, 1, 2, 2, 3]);
        assert_eq!(scale_bucket(0.5), -1);
        assert_eq!(tolerance_for(0), 0.25);
        assert_eq!(tolerance_for(2), 0.0625);
        assert_eq!(tolerance_for(-2), 1.0);
        // Everything that sits in a bucket gets a tolerance no coarser than a quarter pixel there.
        for scale in [0.3f32, 0.7, 1.0, 1.5, 3.0, 17.0, 250.0] {
            assert!(tolerance_for(scale_bucket(scale)) * scale <= PIXEL_TOLERANCE + 1e-6, "scale {scale}");
        }
        // The clamp table: nonsense and extremes stay inside the allowed range.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -3.0] {
            assert_eq!(scale_bucket(bad), MIN_BUCKET, "{bad}");
        }
        assert_eq!(scale_bucket(1e30), MAX_BUCKET);
        assert!(tolerance_for(MAX_BUCKET + 50) >= MIN_TOLERANCE && tolerance_for(MIN_BUCKET - 50) <= MAX_TOLERANCE);
        assert_eq!(tolerance_for(MAX_BUCKET), (0.25f32 / 1_048_576.0).max(MIN_TOLERANCE));
    }

    #[test]
    fn only_shapes_with_curves_or_round_strokes_depend_on_scale() {
        let line = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false);
        assert!(!depends_on_scale(&filled(line.clone())));
        assert!(!depends_on_scale(&stroked(line.clone(), 2.0, LineJoin::Miter, LineCap::Butt)));
        assert!(!depends_on_scale(&stroked(line.clone(), 2.0, LineJoin::Bevel, LineCap::Square)));
        assert!(depends_on_scale(&stroked(line.clone(), 2.0, LineJoin::Round, LineCap::Butt)), "a round join is a curve");
        assert!(depends_on_scale(&stroked(line.clone(), 2.0, LineJoin::Miter, LineCap::Round)), "a round cap is a curve");
        assert!(!depends_on_scale(&stroked(line, 0.0, LineJoin::Round, LineCap::Round)), "a stroke of no width is not drawn");
        assert!(depends_on_scale(&filled(Path::ellipse(Vec2::ZERO, Vec2::splat(5.0)))));
        // And the triangles really are the same at every bucket for such a shape.
        let straight = stroked(polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false), 2.0, LineJoin::Miter, LineCap::Butt);
        assert_eq!(mesh_of(&straight, -3), mesh_of(&straight, 8));
    }

    #[test]
    fn a_stroke_vertex_is_the_path_point_plus_the_normal_times_the_half_width() {
        // A straight line of width 4 from x = 0 to x = 10, butt caps: the outline is the rectangle y in [-2, 2].
        let shape = stroked(polyline(&[(0.0, 0.0), (10.0, 0.0)], false), 4.0, LineJoin::Miter, LineCap::Butt);
        let stroke = mesh_of(&shape, 0).stroke.expect("a stroke");
        assert_eq!(stroke.vertices.len(), 4);
        for v in &stroke.vertices {
            let [x, y] = v.displaced(2.0);
            assert!(x == 0.0 || x == 10.0, "{x}");
            assert!((y.abs() - 2.0).abs() < 1e-5, "{y}");
            assert_eq!(v.point[1], 0.0, "the point is on the path");
            assert!((v.normal[0].powi(2) + v.normal[1].powi(2) - 1.0).abs() < 1e-5, "a unit normal on a straight piece");
            assert!(v.distance == 0.0 || v.distance == 10.0, "distance along the path: {}", v.distance);
        }
        // Because only the half width changes, the same triangles give the wider or thinner stroke, with no new tessellation.
        let ys: Vec<f32> = stroke.vertices.iter().map(|v| v.displaced(0.5)[1].abs()).collect();
        assert!(ys.iter().all(|&y| (y - 0.5).abs() < 1e-5));
    }

    #[test]
    fn miter_corners_extend_the_normal_and_bevels_and_round_joins_do_not_need_to() {
        let corner = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false);
        let miter = mesh_of(&stroked(corner.clone(), 2.0, LineJoin::Miter, LineCap::Butt), 0).stroke.unwrap();
        let longest = miter.vertices.iter().map(|v| (v.normal[0].powi(2) + v.normal[1].powi(2)).sqrt()).fold(0.0, f32::max);
        assert!((longest - std::f32::consts::SQRT_2).abs() < 1e-4, "a right-angle miter reaches sqrt(2) half widths: {longest}");
        // A bevel cuts the corner with two outer vertices where a miter has one; a round join has its arc on top of that.
        let bevel = mesh_of(&stroked(corner.clone(), 2.0, LineJoin::Bevel, LineCap::Butt), 0).stroke.unwrap();
        assert_eq!(bevel.vertices.len(), miter.vertices.len() + 1);
        let round = mesh_of(&stroked(corner.clone(), 2.0, LineJoin::Round, LineCap::Butt), 2).stroke.unwrap();
        assert!(round.vertices.len() > bevel.vertices.len(), "the round join has its arc");
        // A miter that is too long for its limit falls back to a bevel, as in SVG and tiny-skia: a sharp angle with limit 1
        // has no vertex pushed out by more than one half width, and with a limit of 100 it keeps its spike.
        let sharp = polyline(&[(0.0, 0.0), (10.0, 1.0), (0.0, 2.0)], false);
        let longest_with_limit = |limit: f32| {
            let mut shape = stroked(sharp.clone(), 2.0, LineJoin::Miter, LineCap::Butt);
            shape.stroke.as_mut().unwrap().miter_limit = limit;
            let mesh = mesh_of(&shape, 0).stroke.unwrap();
            mesh.vertices.iter().map(|v| (v.normal[0].powi(2) + v.normal[1].powi(2)).sqrt()).fold(0.0, f32::max)
        };
        assert!(longest_with_limit(1.0) <= 1.0 + 1e-4, "{}", longest_with_limit(1.0));
        assert!(longest_with_limit(100.0) > 5.0, "{}", longest_with_limit(100.0));
    }

    #[test]
    fn distance_grows_along_the_path_and_round_caps_add_vertices() {
        let path = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)], false);
        let stroke = mesh_of(&stroked(path.clone(), 2.0, LineJoin::Miter, LineCap::Butt), 0).stroke.unwrap();
        let max = stroke.vertices.iter().map(|v| v.distance).fold(0.0, f32::max);
        assert!((max - 15.0).abs() < 1e-4, "the path is 15 long: {max}");
        let butt = stroke.vertices.len();
        let round = mesh_of(&stroked(path.clone(), 2.0, LineJoin::Miter, LineCap::Round), 3).stroke.unwrap().vertices.len();
        let square = mesh_of(&stroked(path, 2.0, LineJoin::Miter, LineCap::Square), 3).stroke.unwrap().vertices.len();
        assert!(round > square && square >= butt, "round {round}, square {square}, butt {butt}");
    }

    #[test]
    fn closed_paths_stroke_all_the_way_round_and_open_ones_fill_as_if_closed() {
        let square = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true);
        let mut both = Shape::new(square);
        both.fill = Some(Paint::Solid(Color::BLACK));
        both.stroke = Some(Stroke { width: 2.0, ..Stroke::solid(Color::BLACK, 2.0) });
        let mesh = mesh_of(&both, 0);
        let max = mesh.stroke.unwrap().vertices.iter().map(|v| v.distance).fold(0.0, f32::max);
        assert!((max - 40.0).abs() < 1e-3, "the closing side counts: {max}");
        let open = filled(polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], false));
        assert!((area(&mesh_of(&open, 0).fill.unwrap()) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn what_the_cpu_renderer_does_not_draw_is_not_tessellated() {
        let mut shape = Shape::new(polyline(&[(0.0, 0.0), (10.0, 0.0)], false));
        shape.fill = None;
        shape.stroke = Some(Stroke::solid(Color::BLACK, 0.0));
        assert_eq!(mesh_of(&shape, 0), ShapeMesh::default(), "a stroke of width 0 draws nothing");
        shape.stroke = Some(Stroke::solid(Color::BLACK, -3.0));
        assert_eq!(mesh_of(&shape, 0), ShapeMesh::default());
        shape.stroke = Some(Stroke::solid(Color::BLACK, f32::NAN));
        assert_eq!(mesh_of(&shape, 0), ShapeMesh::default());
        // A subpath with one anchor is dropped, a single point has no outline.
        let one = filled(polyline(&[(5.0, 5.0)], true));
        assert_eq!(mesh_of(&one, 0), ShapeMesh::default());
        assert_eq!(drawable_subpaths(&one).count(), 0);
    }

    #[test]
    fn broken_numbers_make_a_shape_disappear_with_a_warning_instead_of_hanging_or_panicking() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let shape = filled(polyline(&[(0.0, 0.0), (bad, 0.0), (10.0, 10.0)], true));
            let mesh = mesh_of(&shape, 0);
            assert_eq!(mesh.warnings, [TessWarning::NotFinite], "{bad}");
            assert!(mesh.fill.is_none() && mesh.stroke.is_none());
        }
        // A broken handle counts too.
        let mut path = polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true);
        path.subpaths[0].anchors[1].handle_out = Vec2::new(f32::NAN, 0.0);
        assert_eq!(mesh_of(&filled(path), 0).warnings, [TessWarning::NotFinite]);
        // A miter limit that is nonsense is clamped, and the stroke is still made.
        for limit in [f32::NAN, -5.0, 0.0, 1e9] {
            let mut shape = stroked(polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], false), 2.0, LineJoin::Miter, LineCap::Butt);
            shape.stroke.as_mut().unwrap().miter_limit = limit;
            let mesh = mesh_of(&shape, 0);
            assert!(mesh.stroke.is_some() && mesh.warnings.is_empty(), "miter limit {limit}");
        }
    }

    #[test]
    fn a_shape_over_the_vertex_cap_is_made_coarser_and_a_hopeless_one_is_dropped_with_a_warning() {
        let circle = filled(Path::ellipse(Vec2::ZERO, Vec2::splat(1000.0)));
        let free = tessellate_shape(&circle, 8, &TessLimits::default());
        let free_count = free.vertex_count();
        assert!(free_count > 200 && free.warnings.is_empty());

        // A cap a bit under what the fine mesh needs: it is made coarser, says so, and respects the cap.
        let capped = tessellate_shape(&circle, 8, &TessLimits { max_vertices: free_count - 10 });
        assert!(capped.vertex_count() <= free_count - 10 && capped.vertex_count() > 8);
        assert!(matches!(capped.warnings.as_slice(), [TessWarning::Coarsened { factor }] if *factor >= 4.0), "{:?}", capped.warnings);

        // A cap that nothing can meet: the part is not drawn and the warning says how many vertices it had.
        let hopeless = tessellate_shape(&circle, 8, &TessLimits { max_vertices: 3 });
        assert!(hopeless.fill.is_none());
        assert!(matches!(hopeless.warnings.as_slice(), [TessWarning::TooComplex { vertices }] if *vertices > 3), "{:?}", hopeless.warnings);
    }
}
