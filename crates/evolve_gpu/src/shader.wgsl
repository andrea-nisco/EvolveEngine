// The scene shader (plan D5, D10, D11). One pipeline draws every fill and stroke.
//
// Every vertex carries the row of the shape table that holds its draw item: the world transform, the depth value, the
// premultiplied color, the stroke width and the opacity. The vertex shader
//   1. pushes stroke vertices out from the path by the half width (the vertex's normal is scaled so that this gives the
//      outline, miters included); fills have a zero normal and stay where they are,
//   2. applies the hairline rule: a stroke thinner than one pixel on the screen is drawn one pixel wide with its opacity
//      multiplied by the true width, which is what tiny-skia does and keeps thin lines from vanishing,
//   3. transforms to the screen with the item's world matrix and the camera, and puts the item's depth in z. Items are
//      drawn with the depth test "greater" and depth writes on, so later items cover earlier ones and an item cannot
//      blend over itself where its own triangles overlap.
// The fragment shader outputs the premultiplied color; the card blends it with premultiplied alpha. For a gradient (plan D16)
// the color comes from a ramp: the vertex shader passes the point in the shape's own space and the gradient's geometry, the
// fragment shader turns the point into a position along the gradient (linear: an affine function; radial: the length of an
// affine function), clamps it (pad), and reads the ramp's 256 colors with a lerp between neighbouring texels. The ramps were
// made by tiny-skia, so the stops are interpolated and premultiplied the way the CPU renderer does it.
//
// Loops: none. Everything here is straight-line code (GPU_SAFETY.md B1).

struct View {
    // The camera: 2x2 matrix as (x_axis.x, x_axis.y, y_axis.x, y_axis.y), world to pixels.
    matrix: vec4<f32>,
    // Where the point `center` is, in the pixels of the pass, in xy.
    translation: vec4<f32>,
    // Size of the target in pixels, in xy.
    size: vec4<f32>,
    // A point of the scene near what the pass shows, in xy. Everything is computed relative to it (plan 5.5): the position of a
    // vertex in the scene is a large number far from the origin, its pixel too at a deep zoom, and subtracting two large numbers
    // in single precision loses what is left. A scene point minus a scene point close to it is exact, and so is the small number
    // that is then scaled.
    center: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var table: texture_2d<f32>;
// The gradient ramps: 256 texels wide, one row per ramp, premultiplied.
@group(0) @binding(2) var ramps: texture_2d<f32>;

// Rows are eight texels wide, 128 rows to a line of the 1024-texel-wide table.
fn texel(row: u32, k: u32) -> vec4<f32> {
    let x = i32((row % 128u) * 8u + k);
    let y = i32(row / 128u);
    return textureLoad(table, vec2<i32>(x, y), 0);
}

struct VertexIn {
    @location(0) point: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) distance: f32,
    @location(3) row: u32,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // The point in the shape's own space (for gradients).
    @location(1) local: vec2<f32>,
    // The paint kind: 0 solid, 1 linear gradient, 2 radial gradient; and the gradient's two texels.
    @location(2) @interpolate(flat) kind: f32,
    @location(3) @interpolate(flat) gradient_a: vec4<f32>,
    @location(4) @interpolate(flat) gradient_b: vec4<f32>,
}

@vertex
fn vs(v: VertexIn) -> VertexOut {
    let t0 = texel(v.row, 0u);
    let t1 = texel(v.row, 1u);
    let color = texel(v.row, 2u);
    let t3 = texel(v.row, 3u);

    if (t1.w > 2.5) {
        // A layer's composite (plan D13): the vertex is a corner of the unit square, the row says where the rectangle is, in
        // the pixels of this pass, and how opaque it is. The color is white times the opacity.
        let pixel = t0.xy + v.point * t0.zw;
        var layer: VertexOut;
        layer.position = vec4<f32>(pixel.x / view.size.x * 2.0 - 1.0, 1.0 - pixel.y / view.size.y * 2.0, t1.z, 1.0);
        layer.color = color * t3.z;
        layer.local = v.point;
        layer.kind = t1.w;
        layer.gradient_a = t0;
        // The texels of the layer for every pixel of this pass (1 unless the layer was drawn smaller than it is shown).
        layer.gradient_b = vec4<f32>(t3.w, 0.0, 0.0, 0.0);
        return layer;
    }

    let world_matrix = mat2x2<f32>(t0.xy, t0.zw);
    let view_matrix = mat2x2<f32>(view.matrix.xy, view.matrix.zw);
    let to_pixels = view_matrix * world_matrix;

    let half_width = t3.x * 0.5;
    var offset = v.normal * half_width;
    var opacity = t3.z;

    let normal_length = length(v.normal);
    if (normal_length > 0.0) {
        // Half the stroke's width on the screen, in pixels, in the direction of this vertex's side.
        let pixel_half_width = length(to_pixels * (v.normal / normal_length)) * half_width;
        if (pixel_half_width > 0.0 && pixel_half_width < 0.5) {
            offset = offset * (0.5 / pixel_half_width);
            opacity = opacity * (2.0 * pixel_half_width);
        }
    }

    let local = v.point + offset;
    // (The translation is taken from the center first: both are large when the shape is far from the origin, the difference is not.)
    let relative = world_matrix * local + (t1.xy - view.center.xy);
    let pixel = view_matrix * relative + view.translation.xy;

    var out: VertexOut;
    out.position = vec4<f32>(pixel.x / view.size.x * 2.0 - 1.0, 1.0 - pixel.y / view.size.y * 2.0, t1.z, 1.0);
    out.color = color * opacity;
    out.local = local;
    out.kind = t1.w;
    if (t1.w > 0.5) {
        out.gradient_a = texel(v.row, 4u);
        out.gradient_b = texel(v.row, 5u);
    } else {
        out.gradient_a = vec4<f32>(0.0);
        out.gradient_b = vec4<f32>(0.0);
    }
    return out;
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    if (in.kind < 0.5) {
        return in.color;
    }
    // The position along the gradient.
    var t: f32;
    if (in.kind < 1.5) {
        t = in.gradient_a.x * in.local.x + in.gradient_a.y * in.local.y + in.gradient_a.z;
    } else {
        let m = mat2x2<f32>(in.gradient_a.xy, in.gradient_a.zw);
        t = length(m * in.local + in.gradient_b.xy);
    }
    // Pad: before the first stop and after the last the end colors go on. Texel i holds the color at (i + 0.5) / 256.
    let at = clamp(clamp(t, 0.0, 1.0) * 256.0 - 0.5, 0.0, 255.0);
    let row = i32(in.gradient_b.z);
    let low = i32(floor(at));
    let high = min(low + 1, 255);
    let ramp = mix(textureLoad(ramps, vec2<i32>(low, row), 0), textureLoad(ramps, vec2<i32>(high, row), 0), at - f32(low));
    // `in.color` is white times the opacity (and the thin-line factor), so this scales the ramp's premultiplied color.
    return ramp * in.color;
}

// The picture of a layer, drawn into the pass that uses it (plan D13). The rectangle's corner is in `gradient_a.xy`. Normally the
// layer and the pass have the same pixels and the texel under the pixel is read exactly (no filtering, so no blur). A layer that
// was drawn smaller than it is shown (reduced resolution, when the layer memory is full) is read with filtering. `ratio` is
// how many texels of the layer there are for a pixel of the pass. Either way the picture is scaled by the opacity.
@group(1) @binding(0) var layer_texture: texture_2d<f32>;
@group(1) @binding(1) var layer_sampler: sampler;

@fragment
fn fs_layer(in: VertexOut) -> @location(0) vec4<f32> {
    let corner = floor(in.gradient_a.xy + vec2<f32>(0.5));
    let ratio = in.gradient_b.x;
    if (abs(ratio - 1.0) < 0.0001) {
        return textureLoad(layer_texture, vec2<i32>(floor(in.position.xy) - corner), 0) * in.color;
    }
    let size = vec2<f32>(textureDimensions(layer_texture));
    return textureSampleLevel(layer_texture, layer_sampler, (in.position.xy - corner) * ratio / size, 0.0) * in.color;
}