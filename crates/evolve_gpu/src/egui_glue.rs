//! Showing the renderer's pictures inside egui (plan D1, D2, D14; tasks 2.7, 2.8, 2.10). Needs the cargo feature `egui`.
//!
//! The renderer itself knows nothing about egui. This module is the thin layer on top:
//! - [`GpuContext`] wraps the device that eframe made (rule C1: its buffers and textures go through our allocator, its
//!   errors into our log) and puts the pipeline that shows pictures into egui's callback resources.
//! - [`GpuView`] is one view (the Scene view, the Game view): its own [`Renderer`], and a paint callback that draws the
//!   resolved picture into the rectangle egui gives it.
//!
//! The scene is drawn when the view is painted, in the UI code, and submitted at once; egui's own pass is submitted later
//! in the same frame, and a queue runs its work in order, so the picture is ready when egui draws it. (A paint callback has
//! to own what it uses and cannot borrow the scene, which is why the drawing is not done inside it.)
//!
//! **Idle frames** (task 2.10): when the scene's change stamps, the camera, the view size and the options are the same as
//! at the last paint, nothing is drawn and nothing is submitted; the callback shows the picture that is already there.
//! A view of zero size, or one that is not painted at all (a hidden tab), costs nothing (GPU_SAFETY.md A3, A4).

use crate::alloc::{AllocError, Category, Tracked};
use crate::device::{Gpu, Mode};
use crate::profile::{Governor, Quality, QualityProfile};
use crate::renderer::{watchdog_message, FrameStats, RenderError, RenderOptions, Renderer};
use egui_wgpu::{CallbackTrait, RenderState, ScreenDescriptor};
use evolve_engine::geometry::Affine2;
use evolve_engine::revision::Revs;
use evolve_engine::scene::Scene;
use evolve_engine::Vec2;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Bytes of the picture-showing pass's parameters: a `vec2`, two `u32`.
const PARAMS_BYTES: u64 = 16;

/// What the paint callback needs from egui's callback resources: the pipeline that shows a picture.
struct QuadResources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Shows a picture drawn at the view's size, texel for texel.
    sampler: wgpu::Sampler,
    /// Shows a picture drawn smaller than the view (a quality profile's render scale) stretched.
    smooth_sampler: wgpu::Sampler,
    srgb_target: bool,
}

/// The graphics device of the editor as the renderer sees it. Cheap to clone.
#[derive(Clone)]
pub struct GpuContext {
    pub gpu: Gpu,
    renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    /// What the user chose for the quality (see `profile.rs`), shared by every view of this device: a [`Quality`] as a byte.
    quality: Arc<AtomicU8>,
    /// The profile this device's adapter deserves before anything is measured.
    adapter_profile: QualityProfile,
}

impl GpuContext {
    /// Wraps the device of eframe's render state. Replaces the device's error handler (ours logs errors instead of
    /// panicking, and remembers a lost device so that views can switch to the CPU renderer).
    pub fn from_render_state(state: &RenderState) -> GpuContext {
        let gpu = Gpu::from_parts(state.device.clone(), state.queue.clone(), state.adapter.get_info(), Mode::Native);
        GpuContext::new(gpu, state.renderer.clone(), state.target_format)
    }

    /// For a device and an egui renderer that are not eframe's (the tests).
    pub fn new(gpu: Gpu, renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>, target_format: wgpu::TextureFormat) -> GpuContext {
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(PARAMS_BYTES) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("picture"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("picture"), source: wgpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()) });
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("picture"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            // egui's pass has no depth buffer and no multisampling unless it is switched on for the whole UI, which the
            // editor does not do.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("picture"), ..Default::default() }); // nearest
        let smooth_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("picture, smoothed"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        renderer.write().callback_resources.insert(QuadResources { pipeline, layout, sampler, smooth_sampler, srgb_target: target_format.is_srgb() });
        // The memory this device may use is what the adapter's profile allows, until the user chooses (plan 5.8).
        let adapter_profile = QualityProfile::for_device(&gpu);
        gpu.alloc.set_budgets(adapter_profile.budgets());
        GpuContext { gpu, renderer, quality: Arc::new(AtomicU8::new(Quality::Auto.to_byte())), adapter_profile }
    }

    /// What the user chose for the quality of every view of this device.
    pub fn quality(&self) -> Quality {
        Quality::from_byte(self.quality.load(Ordering::Relaxed))
    }

    /// The profile the adapter deserves before anything is measured.
    pub fn adapter_profile(&self) -> QualityProfile {
        self.adapter_profile
    }

    /// The best profile the choice allows: the adapter's for automatic, the chosen one otherwise.
    pub fn ceiling(&self) -> QualityProfile {
        self.quality().ceiling(self.adapter_profile)
    }

    /// Changes the quality of every view of this device. The memory caps of the new ceiling apply at once (memory held above them is
    /// given back as the views are drawn again; a scene that does not fit is drawn by the CPU renderer, with a notice); the views
    /// take the render scale and the time for shapes at their next paint.
    pub fn set_quality(&self, quality: Quality) {
        self.quality.store(quality.to_byte(), Ordering::Relaxed);
        self.gpu.alloc.set_budgets(quality.ceiling(self.adapter_profile).budgets());
    }

    /// The egui renderer (for tests that draw a whole egui frame).
    pub fn egui_renderer(&self) -> &Arc<egui::mutex::RwLock<egui_wgpu::Renderer>> {
        &self.renderer
    }
}

/// Frames closer together than this are one stretch of work: the card is awake for the second.
const BUSY_GAP: std::time::Duration = std::time::Duration::from_millis(100);

/// The milliseconds a frame of this profile may spend on shapes that are not needed at once (plan 5.4). The browser has one thread
/// for everything, so it gets half.
fn tessellation_ms(profile: QualityProfile) -> f64 {
    profile.tessellation_ms() * if cfg!(target_arch = "wasm32") { 0.5 } else { 1.0 }
}

/// The size a picture is drawn at for a view of `size` pixels and a render scale (at least one pixel each way; exactly `size` at 1).
pub(crate) fn scaled(size: (u32, u32), scale: f32) -> (u32, u32) {
    if scale >= 1.0 {
        return size;
    }
    (((size.0 as f32 * scale).round() as u32).clamp(1, size.0), ((size.1 as f32 * scale).round() as u32).clamp(1, size.1))
}

/// What the key of an idle frame is made of: the same stamps, camera, size and options give the same picture.
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    revs: Revs,
    view: Affine2,
    size: (u32, u32),
    camera_frame: bool,
}

/// What a call to [`GpuView::paint`] did.
#[derive(Debug)]
pub enum Painted {
    /// Nothing to show: the view has no size.
    Nothing,
    /// Nothing changed since the last paint: the picture that is there was shown again, nothing was drawn or submitted.
    Idle,
    /// The scene was drawn.
    Drawn(Box<FrameStats>),
}

/// One view of a scene, drawn by the GPU renderer and shown in egui.
pub struct GpuView {
    /// What the view is called in the watchdog's messages ("Scene view").
    name: &'static str,
    last_warning: Option<web_time::Instant>,
    context: GpuContext,
    renderer: Renderer,
    params: Tracked<wgpu::Buffer>,
    params_written: Option<[f32; 4]>,
    /// The group that shows the picture, for the picture it was made for and whether the sampler filters.
    bind_group: Option<((u64, bool), Arc<wgpu::BindGroup>)>,
    shown: Option<Shown>,
    /// Chooses the profile this view draws with from the user's choice and the times of its frames (plan D15).
    governor: Governor,
    /// When the view last drew: a frame soon after another finds the card awake, which its completion time is only fair about then.
    last_draw: Option<web_time::Instant>,
    /// Tests: the time every frame is taken to have taken, in place of the measured one.
    #[cfg(test)]
    pub(crate) pretend_frame_ms: Option<f64>,
}

struct PictureCallback {
    bind_group: Arc<wgpu::BindGroup>,
}

impl CallbackTrait for PictureCallback {
    fn paint(&self, _info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(quad) = resources.get::<QuadResources>() else { return };
        render_pass.set_pipeline(&quad.pipeline);
        render_pass.set_bind_group(0, &*self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }
}

impl GpuView {
    pub fn new(context: &GpuContext, name: &'static str) -> Result<GpuView, AllocError> {
        let params = context.gpu.alloc.create_buffer(
            Category::Other,
            &wgpu::BufferDescriptor { label: Some("picture params"), size: PARAMS_BYTES, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false },
        )?;
        let mut renderer = Renderer::new(&context.gpu)?;
        let governor = Governor::new(context.ceiling());
        renderer.set_tessellation_budget(Some(tessellation_ms(governor.current())));
        Ok(GpuView { name, last_warning: None, context: context.clone(), renderer, params, params_written: None, bind_group: None, shown: None, governor, last_draw: None, #[cfg(test)] pretend_frame_ms: None })
    }

    /// The profile this view draws with now: the user's choice, or for automatic the adapter's, lowered while frames are slow.
    pub fn profile(&self) -> QualityProfile {
        self.governor.current()
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Forgets what was last shown, so that the next paint draws again (after a setting that changes the picture).
    pub fn invalidate(&mut self) {
        self.shown = None;
    }

    /// Draws `scene` (unless nothing changed) and shows it in `rect` (egui points). `view` turns the picture's size in pixels
    /// into the transform from the scene to pixels, as the CPU renderer's views do.
    ///
    /// On `Err` nothing is shown and the caller should draw this view with the CPU renderer; the error says why.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        scene: &Scene,
        view: impl Fn(Vec2) -> Affine2,
        options: &RenderOptions,
    ) -> Result<Painted, RenderError> {
        let ppp = painter.ctx().pixels_per_point();
        let size = ((rect.width() * ppp).round().max(0.0) as u32, (rect.height() * ppp).round().max(0.0) as u32);
        if size.0 == 0 || size.1 == 0 || !rect.is_finite() {
            return Ok(Painted::Nothing);
        }
        // The quality: what the user chose, and the profile the governor makes of it. A profile below High draws the picture smaller
        // than the view and shows it stretched (the render scale), and spends less time on shapes that are not needed at once.
        let quality = self.context.quality();
        self.governor.follow(quality, self.context.adapter_profile());
        let profile = self.governor.current();
        self.renderer.set_tessellation_budget(Some(tessellation_ms(profile)));
        let drawn = scaled(size, profile.render_scale());
        let full_view = view(Vec2::new(size.0 as f32, size.1 as f32));
        let view_matrix = if drawn == size { full_view } else { Affine2::from_scale(Vec2::new(drawn.0 as f32 / size.0 as f32, drawn.1 as f32 / size.1 as f32)) * full_view };
        let key = Shown { revs: scene.revision(), view: view_matrix, size: drawn, camera_frame: options.camera_frame };
        let painted = if self.shown == Some(key) && self.renderer.picture().is_some() {
            Painted::Idle
        } else {
            // A failed attempt must not leave the old picture marked as current.
            self.shown = None;
            let busy = self.last_draw.is_some_and(|t| t.elapsed() < BUSY_GAP);
            self.last_draw = Some(web_time::Instant::now());
            let stats = self.renderer.render(scene, view_matrix, drawn, options)?;
            if quality == Quality::Auto {
                // What the frame cost: the processor's part, and the card's when it was already busy (a card that was idle takes
                // longer to wake up than to do a small frame, which says nothing about the scene).
                let card = if busy { self.renderer.last_completion_ms().unwrap_or(0.0) } else { 0.0 };
                let frame_ms = (stats.update_ms + stats.record_ms).max(card);
                #[cfg(test)]
                let frame_ms = self.pretend_frame_ms.unwrap_or(frame_ms);
                if self.governor.record(frame_ms).is_some() {
                    painter.ctx().request_repaint(); // the new profile shows at the next paint
                }
            }
            if stats.deferred > 0 {
                // Work was put off to keep the frame short: not shown as final, so that the next paint carries on.
                painter.ctx().request_repaint();
            } else {
                self.shown = Some(key);
            }
            self.watch(&scene.name, &stats);
            Painted::Drawn(Box::new(stats))
        };
        self.show(painter, rect, drawn, drawn != size);
        Ok(painted)
    }

    /// The watchdog (GPU_SAFETY.md B4), in development builds: a frame that took more than 100 ms on the CPU, or until the
    /// card was done (as last reported), is named on the console, at most once a second.
    fn watch(&mut self, scene_name: &str, stats: &FrameStats) {
        if !cfg!(debug_assertions) {
            return;
        }
        let Some(message) = watchdog_message(self.name, scene_name, stats.update_ms + stats.record_ms, self.renderer.last_completion_ms()) else { return };
        if self.last_warning.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) {
            eprintln!("{message}");
            self.last_warning = Some(web_time::Instant::now());
        }
    }

    /// Adds the paint callback that shows the picture.
    fn show(&mut self, painter: &egui::Painter, rect: egui::Rect, size: (u32, u32), smooth: bool) {
        let (Some(view), Some(target)) = (self.renderer.picture_view(), self.renderer.targets().size()) else { return };
        let generation = (self.renderer.picture_generation(), smooth);
        if self.bind_group.as_ref().is_none_or(|(seen, _)| *seen != generation) {
            self.params_written = None;
            let renderer = self.context.egui_renderer().read();
            let Some(quad) = renderer.callback_resources.get::<QuadResources>() else { return };
            let group = self.context.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("picture"),
                layout: &quad.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(if smooth { &quad.smooth_sampler } else { &quad.sampler }) },
                ],
            });
            self.bind_group = Some((generation, Arc::new(group)));
        }
        let srgb = self.context.egui_renderer().read().callback_resources.get::<QuadResources>().is_some_and(|q| q.srgb_target);
        let scale = [size.0 as f32 / target.0 as f32, size.1 as f32 / target.1 as f32, f32::from_bits(srgb as u32), 0.0];
        if self.params_written != Some(scale) {
            self.context.gpu.queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&scale));
            self.params_written = Some(scale);
        }
        if let Some((_, group)) = &self.bind_group {
            painter.add(egui_wgpu::Callback::new_paint_callback(rect, PictureCallback { bind_group: group.clone() }));
        }
    }
}
