// Shows the renderer's picture inside egui's own pass (plan D14, task 2.7): one triangle that covers the rectangle egui gives
// the paint callback, sampling the picture without filtering.
//
// The picture holds gamma-encoded colors, premultiplied by alpha, in an `Rgba8Unorm` texture, exactly like the bitmap of the
// CPU renderer. egui's own image shader would take those bytes for linear light and encode them a second time on the
// usual window formats, washing the colors out. This shader puts the bytes on the screen as they are:
//  - on a window format that is not sRGB (the usual one: egui blends in gamma space itself) the value is written as is;
//  - on an sRGB window format the card encodes what is written, so the value is decoded first (alpha taken out and put back
//    in, so that the premultiplied color is decoded as the color it stands for).
//
// Loops: none (GPU_SAFETY.md B1).

struct Params {
    // How much of the picture texture is the view (it is drawn into the top left corner of a texture with slack).
    uv_scale: vec2<f32>,
    // 1 when the window format is sRGB.
    srgb: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOut {
    // One triangle that covers the whole viewport: (-1,-1), (3,-1), (-1,3).
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var out: VertexOut;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5) * params.uv_scale;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    // A picture that is drawn smaller than the view is shown stretched, with filtering; the filter must not reach texels past the
    // part of the texture that holds the picture (the texture has slack), so the sample stays at least half a texel inside it. At
    // the view's own size every sample is at the middle of a texel and this changes nothing.
    let half_texel = vec2<f32>(0.5) / vec2<f32>(textureDimensions(picture));
    let uv = clamp(in.uv, half_texel, params.uv_scale - half_texel);
    let c = textureSample(picture, picture_sampler, uv);
    if (params.srgb == 0u) {
        return c;
    }
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(srgb_to_linear(c.rgb / c.a) * c.a, c.a);
}
