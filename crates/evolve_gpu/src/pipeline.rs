//! The render pipelines of the scene (plan tasks 2.5 and 4.1): layout, shader and fixed-function state.
//!
//! Items: premultiplied alpha blending, depth test "greater" with depth writes (cleared to 0, so every item passes against the
//! empty buffer and a later item passes against an earlier one), stencil test "equal" to the clip level, 4 samples, no culling
//! (a negative scale flips triangles). Clip masks: the same vertex shader, no colour and no depth, only the stencil.

use crate::targets::{COLOR_FORMAT, DEPTH_FORMAT, SAMPLES};
use crate::vertex::Vertex;

pub struct Pipeline {
    /// Paints items where the stencil equals the reference.
    pub pipeline: wgpu::RenderPipeline,
    /// Draws a clip mask into the stencil only, adding one.
    pub mask_push: wgpu::RenderPipeline,
    /// The same, taking one away.
    pub mask_pop: wgpu::RenderPipeline,
    /// Draws a layer's picture (group 1 holds its texture) where the stencil equals the reference.
    pub composite: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Group 1 of the composite: the layer's texture, and a sampler for layers that were drawn smaller than they are shown.
    pub layer_layout: wgpu::BindGroupLayout,
    pub layer_sampler: wgpu::Sampler,
}

/// Size of the view uniform: four `vec4<f32>`.
pub const VIEW_UNIFORM_BYTES: u64 = 64;
/// Every pass has its own view uniform (a layer's picture is the view shifted by its corner), at a multiple of this in one
/// buffer, chosen with a dynamic offset. 256 is the alignment every device offers (WebGL2 included).
pub const UNIFORM_SLOT_BYTES: u64 = 256;

impl Pipeline {
    pub fn new(device: &wgpu::Device) -> Pipeline {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(VIEW_UNIFORM_BYTES),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&layer_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("scene"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        // The stencil is the clip level (plan 4.1): items are drawn where it equals the reference, and never change it; a mask
        // adds one (push) or takes one away (pop) where it passes the same test.
        let stencil = |pass_op: wgpu::StencilOperation, write_mask: u32| {
            let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Equal, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op };
            wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask }
        };
        let make = |label: &'static str, layout: &wgpu::PipelineLayout, fragment: &'static str, colors: wgpu::ColorWrites, paint: bool, stencil: wgpu::StencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[Some(Vertex::LAYOUT)] },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: paint.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: colors,
                    })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    // Items: later ones pass against earlier ones and are recorded. Masks: no part in the depth test.
                    depth_write_enabled: Some(paint),
                    depth_compare: Some(if paint { wgpu::CompareFunction::Greater } else { wgpu::CompareFunction::Always }),
                    stencil,
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make("scene", &layout, "fs", wgpu::ColorWrites::ALL, true, stencil(wgpu::StencilOperation::Keep, 0));
        let mask_push = make("mask push", &layout, "fs", wgpu::ColorWrites::empty(), false, stencil(wgpu::StencilOperation::IncrementClamp, 0xff));
        let mask_pop = make("mask pop", &layout, "fs", wgpu::ColorWrites::empty(), false, stencil(wgpu::StencilOperation::DecrementClamp, 0xff));
        let composite = make("composite", &composite_layout, "fs_layer", wgpu::ColorWrites::ALL, true, stencil(wgpu::StencilOperation::Keep, 0));
        let layer_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("layer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Pipeline { pipeline, mask_push, mask_pop, composite, bind_group_layout, layer_layout, layer_sampler }
    }
}
