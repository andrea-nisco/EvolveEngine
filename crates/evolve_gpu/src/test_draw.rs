//! Test-only: draws one convex quad with the card, with or without 4x MSAA, so that the picture comparison can be
//! checked against tiny-skia before the real renderer exists. Not part of the renderer.

use crate::alloc::Category;
use crate::compare::{compare_pictures, Rule};
use crate::device::Gpu;
use crate::readback::PICTURE_FORMAT;
use evolve_engine::tiny_skia as sk;

const SHADER: &str = "
struct Quad { corners_01: vec4<f32>, corners_23: vec4<f32>, size: vec4<f32>, color: vec4<f32> }
@group(0) @binding(0) var<uniform> quad: Quad;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corner = array<vec2<f32>, 4>(quad.corners_01.xy, quad.corners_01.zw, quad.corners_23.xy, quad.corners_23.zw);
    var order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let p = corner[order[i]];
    return vec4<f32>(p.x / quad.size.x * 2.0 - 1.0, 1.0 - p.y / quad.size.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> { return quad.color; }
";

/// Draws the quad (corners in pixels, y down) in an opaque premultiplied `color`, on a transparent picture of the
/// given size, and reads it back. `samples` is 1 or 4.
pub fn draw_quad(gpu: &Gpu, width: u32, height: u32, corners: [[f32; 2]; 4], color: [f32; 4], samples: u32) -> Vec<u8> {
    let uniform: [f32; 16] = [
        corners[0][0], corners[0][1], corners[1][0], corners[1][1],
        corners[2][0], corners[2][1], corners[3][0], corners[3][1],
        width as f32, height as f32, 0.0, 0.0,
        color[0], color[1], color[2], color[3],
    ];
    let buffer = gpu
        .alloc
        .create_buffer(
            Category::Other,
            &wgpu::BufferDescriptor { label: Some("quad"), size: std::mem::size_of_val(&uniform) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false },
        )
        .expect("a small uniform buffer");
    gpu.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
    let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quad"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quad"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("quad"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quad"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: PICTURE_FORMAT, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
        multiview_mask: None,
        cache: None,
    });

    let resolved = gpu.picture_texture(width, height);
    let resolved_view = resolved.create_view(&Default::default());
    let multisampled = (samples > 1).then(|| {
        gpu.alloc.create_texture(Category::Targets, &wgpu::TextureDescriptor {
            label: Some("multisampled"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: PICTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }).expect("a small multisampled texture")
    });
    let multisampled_view = multisampled.as_ref().map(|t| t.create_view(&Default::default()));

    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let (view, resolve_target, store) = match &multisampled_view {
            Some(view) => (view, Some(&resolved_view), wgpu::StoreOp::Discard),
            None => (&resolved_view, None, wgpu::StoreOp::Store),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quad"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..6, 0..1);
    }
    gpu.queue.submit([encoder.finish()]);
    gpu.read_picture(&resolved)
}

/// The same quad drawn by tiny-skia.
fn draw_quad_cpu(width: u32, height: u32, corners: [[f32; 2]; 4], color: [u8; 4]) -> Vec<u8> {
    let mut builder = sk::PathBuilder::new();
    builder.move_to(corners[0][0], corners[0][1]);
    for corner in &corners[1..] {
        builder.line_to(corner[0], corner[1]);
    }
    builder.close();
    let mut pixmap = sk::Pixmap::new(width, height).unwrap();
    let mut paint = sk::Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    pixmap.fill_path(&builder.finish().unwrap(), &paint, sk::FillRule::Winding, sk::Transform::identity(), None);
    pixmap.take()
}

const RED: [f32; 4] = [200.0 / 255.0, 60.0 / 255.0, 30.0 / 255.0, 1.0];

/// A rectangle with fractional edges, turned by 20 degrees about its middle, so edges at every angle are tested.
fn tilted_rectangle(dx: f32) -> [[f32; 2]; 4] {
    let (sin, cos) = 20f32.to_radians().sin_cos();
    let (cx, cy) = (60.3 + dx, 40.7);
    [(-35.0f32, -22.0f32), (35.0, -22.0), (35.0, 22.0), (-35.0, 22.0)].map(|(x, y)| [cx + x * cos - y * sin, cy + x * sin + y * cos])
}

#[test]
fn a_4x_msaa_quad_matches_tiny_skia_under_the_edge_rule() {
    Gpu::for_each_test_mode(|gpu| {
        let reference = draw_quad_cpu(120, 80, tilted_rectangle(0.0), [200, 60, 30, 255]);
        let gpu_picture = draw_quad(gpu, 120, 80, tilted_rectangle(0.0), RED, 4);
        let result = compare_pictures(&reference, &gpu_picture, 120, Rule::default());
        assert!(result.passes(), "an honest MSAA picture must pass: {result}");
        println!("MSAA against tiny-skia: {result}");
    });
}

/// Mutation check of the comparison on real card output: the same quad one pixel to the right must fail.
#[test]
fn a_quad_drawn_one_pixel_off_fails_the_edge_rule() {
    Gpu::for_each_test_mode(|gpu| {
        let reference = draw_quad_cpu(120, 80, tilted_rectangle(0.0), [200, 60, 30, 255]);
        let shifted = draw_quad(gpu, 120, 80, tilted_rectangle(1.0), RED, 4);
        let result = compare_pictures(&reference, &shifted, 120, Rule::default());
        assert!(!result.passes(), "a shift of one pixel went unnoticed: {result}");
    });
}

/// Without MSAA the card's edges are hard (all or nothing). That must show as a failure of the edge rule's mean or
/// edge limit on a shape this size, which is why the renderer uses 4x MSAA (plan D4).
#[test]
fn a_quad_without_msaa_is_visibly_worse_than_with_it() {
    Gpu::for_each_test_mode(|gpu| {
        let reference = draw_quad_cpu(120, 80, tilted_rectangle(0.0), [200, 60, 30, 255]);
        let worst = |samples| {
            let picture = draw_quad(gpu, 120, 80, tilted_rectangle(0.0), RED, samples);
            compare_pictures(&reference, &picture, 120, Rule::default()).worst_edge.0
        };
        assert!(worst(1) > worst(4), "hard edges ({}) should be further from exact coverage than 4x MSAA ({})", worst(1), worst(4));
    });
}
