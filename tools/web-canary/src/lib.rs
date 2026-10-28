//! The web canary (plan task 2.12): a page that runs the `evolve_gpu` renderer in a browser, on WebGL2 or on WebGPU, and
//! reports what it saw. Its only purpose is to find WebGL2 problems now instead of in Phase 8: shader or driver differences,
//! limits, errors the browser's graphics layer reports, and frame times on a real browser and card.
//!
//! The page (`index.html`) loads a project file (a scene made with `evolve_editor --gen-scene`, or any `.evolve`), draws three
//! views of it (the whole scene, a region, a close-up) for a fixed number of frames each, shows the picture on the canvas, and
//! writes a report into the page and the console. The work is bounded (GPU_SAFETY.md B1, F1): a fixed number of frames, then it
//! stops; it never loops forever.
//!
//! Build it with `tools/build-web-canary.ps1` (or `.sh`), serve `dist/web-canary` with any static web server, and open
//! `index.html?backend=webgl&scene=scene.evolve` (or `backend=webgpu`).
#![cfg(target_arch = "wasm32")]

use evolve_engine::geometry::{Affine2, Vec2};
use evolve_engine::scene::Scene;
use evolve_gpu::device::{Gpu, Mode};
use evolve_gpu::readback::PICTURE_FORMAT;
use evolve_gpu::renderer::{FrameStats, RenderOptions, Renderer};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// Frames at the start of each view that are not counted in the averages (the first one tessellates everything).
const WARM_UP: u32 = 5;

fn window() -> Result<web_sys::Window, String> {
    web_sys::window().ok_or_else(|| "no window".to_string())
}

/// Adds a line to the report on the page (the element with id `out`) and to the console.
fn say(line: &str) {
    web_sys::console::log_1(&JsValue::from_str(line));
    if let Some(element) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("out")) {
        let old = element.text_content().unwrap_or_default();
        element.set_text_content(Some(&format!("{old}{line}\n")));
    }
}

fn js(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

const BLIT: &str = "
@group(0) @binding(0) var picture: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole target.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // The picture is premultiplied; over black that is just its color. Rows run the same way in both textures.
    let c = textureLoad(picture, vec2<i32>(pos.xy), 0);
    return vec4<f32>(c.rgb, 1.0);
}
";

struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    group: Option<(u64, wgpu::BindGroup)>,
}

impl Blit {
    fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Blit {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("blit"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Blit { pipeline, layout, group: None }
    }
}

/// One of the three views of the scene, with what was measured for it.
struct View {
    name: &'static str,
    matrix: Affine2,
    /// CPU time of `render` (walk, update, record, submit), per counted frame, in milliseconds.
    cpu: Vec<f64>,
    /// Time between two animation frames (what the browser really delivered), per counted frame.
    interval: Vec<f64>,
    first_frame_ms: f64,
    /// What the first frame was made of (making the triangles, finding the items, rows and chunks, encoding and submitting).
    first_stats: Option<FrameStats>,
    stats: Option<FrameStats>,
}

struct Canary {
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    size: (u32, u32),
    scene: Scene,
    renderer: Renderer,
    options: RenderOptions,
    blit: Blit,
    views: Vec<View>,
    current: usize,
    frame: u32,
    frames_per_view: u32,
    last_timestamp: Option<f64>,
    skipped: String,
    started: web_time::Instant,
}

/// What a sorted list of numbers says in one line.
fn describe(values: &[f64]) -> String {
    if values.is_empty() {
        return "no frames".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    format!("median {:.2}  p95 {:.2}  worst {:.2}", at(0.5), at(0.95), sorted[sorted.len() - 1])
}

impl Canary {
    /// One animation frame. Returns false when the run is over.
    fn step(&mut self, timestamp: f64) -> bool {
        let interval = self.last_timestamp.map(|last| timestamp - last);
        self.last_timestamp = Some(timestamp);
        if self.current >= self.views.len() {
            return false;
        }
        let view = self.views[self.current].matrix;
        let started = web_time::Instant::now();
        let stats = match self.renderer.render(&self.scene, view, self.size, &self.options) {
            Ok(stats) => stats,
            Err(error) => {
                say(&format!("RENDER ERROR in view '{}': {error}", self.views[self.current].name));
                return false;
            }
        };
        let cpu = started.elapsed().as_secs_f64() * 1000.0;
        self.present();

        let frame = self.frame;
        let view = &mut self.views[self.current];
        if frame == 0 {
            view.first_frame_ms = cpu;
            view.first_stats = Some(stats.clone());
        }
        if frame >= WARM_UP {
            view.cpu.push(cpu);
            view.interval.extend(interval);
        }
        view.stats = Some(stats);
        self.frame += 1;
        if self.frame >= self.frames_per_view {
            self.frame = 0;
            self.current += 1;
        }
        self.current < self.views.len()
    }

    /// Shows the renderer's picture on the canvas.
    fn present(&mut self) {
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                say(&format!("no surface texture this frame: {}", match other {
                    wgpu::CurrentSurfaceTexture::Timeout => "timeout",
                    wgpu::CurrentSurfaceTexture::Occluded => "occluded",
                    wgpu::CurrentSurfaceTexture::Outdated => "outdated",
                    wgpu::CurrentSurfaceTexture::Lost => "lost",
                    _ => "validation error",
                }));
                return;
            }
        };
        let Some(picture) = self.renderer.picture_view() else { return };
        let generation = self.renderer.picture_generation();
        if self.blit.group.as_ref().is_none_or(|(g, _)| *g != generation) {
            let group = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blit"),
                layout: &self.blit.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(picture) }],
            });
            self.blit.group = Some((generation, group));
        }
        let target = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blit") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit.pipeline);
            pass.set_bind_group(0, &self.blit.group.as_ref().expect("just made").1, &[]);
            pass.draw(0..3, 0..1);
        }
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.queue.present(texture);
    }

    fn report(&self) {
        say("");
        say(&format!("--- report after {:.1} s ---", self.started.elapsed().as_secs_f64()));
        if !self.skipped.is_empty() {
            say(&format!("NOTE: the scene has things the GPU renderer cannot draw yet ({}); they were left out, so the picture is incomplete and the numbers are for timing only.", self.skipped));
        }
        for view in &self.views {
            say(&format!("view '{}'", view.name));
            say(&format!("  first frame (everything made): {:.1} ms", view.first_frame_ms));
            if let Some(first) = &view.first_stats {
                say(&format!(
                    "    of which: making triangles {:.1}, finding items {:.1}, rows/vertices/chunks {:.1}, the rest of preparing {:.1}, encoding and submitting {:.1} ms; {} shapes made, {:.1} MB of geometry sent",
                    first.tessellate_ms,
                    first.collect_ms,
                    first.list_ms,
                    (first.update_ms - first.tessellate_ms - first.collect_ms - first.list_ms).max(0.0),
                    first.record_ms,
                    first.shapes_tessellated,
                    first.geometry_bytes_uploaded as f64 / 1_048_576.0
                ));
            }
            say(&format!("  CPU time of a frame, ms: {}", describe(&view.cpu)));
            say(&format!("  time between frames, ms: {}   (a 60 Hz screen cannot show less than 16.7)", describe(&view.interval)));
            if let Some(stats) = &view.stats {
                say(&format!(
                    "  {} items in {} draw calls, {} triangles, {} shapes made and {} culled in the last frame",
                    stats.items, stats.draw_calls, stats.triangles, stats.shapes_tessellated, stats.shapes_culled
                ));
            }
        }
        say("graphics memory held by the renderer:");
        for (category, bytes) in self.renderer.memory() {
            if bytes > 0 {
                say(&format!("  {category:?}: {:.1} MB", bytes as f64 / 1_048_576.0));
            }
        }
        let errors = self.gpu.errors.take();
        say(&format!("errors reported by the graphics layer: {}", errors.len()));
        for error in errors.iter().take(10) {
            say(&format!("  {error}"));
        }
        if let Some(why) = self.gpu.errors.lost() {
            say(&format!("DEVICE LOST: {why}"));
        }
        say("done.");
        // A page that wants the report (a test driver, say) defines `window.canaryDone(text)`.
        if let Some(window) = web_sys::window() {
            let text = window.document().and_then(|d| d.get_element_by_id("out")).and_then(|e| e.text_content()).unwrap_or_default();
            if let Ok(done) = js_sys::Reflect::get(&window, &JsValue::from_str("canaryDone")) {
                if let Some(done) = done.dyn_ref::<js_sys::Function>() {
                    let _ = done.call1(&JsValue::NULL, &JsValue::from_str(&text));
                }
            }
        }
    }
}

/// Opens the device, loads the scene, and starts the bounded run. `backend` is `webgl` or `webgpu`.
#[wasm_bindgen]
pub async fn start(canvas_id: String, scene_url: String, backend: String, frames_per_view: u32) -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|info| say(&format!("PANIC: {info}"))));
    match run(&canvas_id, &scene_url, &backend, frames_per_view.clamp(10, 2000)).await {
        Ok(()) => Ok(()),
        Err(message) => {
            say(&format!("ERROR: {message}"));
            Err(JsValue::from_str(&message))
        }
    }
}

async fn run(canvas_id: &str, scene_url: &str, backend: &str, frames_per_view: u32) -> Result<(), String> {
    let window = window()?;
    let document = window.document().ok_or("no document")?;
    let canvas: web_sys::HtmlCanvasElement =
        document.get_element_by_id(canvas_id).ok_or("no canvas on the page")?.dyn_into().map_err(|_| "that element is not a canvas".to_string())?;
    let size = (canvas.width().max(1), canvas.height().max(1));

    let (backends, mode) = match backend {
        "webgpu" => (wgpu::Backends::BROWSER_WEBGPU, Mode::Native),
        _ => (wgpu::Backends::GL, Mode::WebGl2),
    };
    say(&format!("backend: {}, canvas {} x {}", if mode == Mode::WebGl2 { "WebGL2" } else { "WebGPU" }, size.0, size.1));
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| format!("could not use the canvas: {e}"))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() })
        .await
        .map_err(|e| format!("no graphics adapter: {e}"))?;
    let info = adapter.get_info();
    say(&format!("adapter: {} ({:?}, {:?}), driver '{}'", info.name, info.backend, info.device_type, info.driver));
    let limits = match mode {
        Mode::WebGl2 => wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        Mode::Native => adapter.limits(),
    };
    say(&format!("largest texture {} px, largest buffer {} MB", limits.max_texture_dimension_2d, limits.max_buffer_size / 1_048_576));
    let (device, queue) =
        adapter.request_device(&wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() }).await.map_err(|e| format!("could not open the device: {e}"))?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).or(caps.formats.first().copied()).ok_or("the canvas supports no format")?;
    surface.configure(
        &device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        },
    );
    say(&format!("canvas format {format:?} (the renderer's own pictures are {PICTURE_FORMAT:?})"));
    let gpu = Gpu::from_parts(device, queue, info, mode);

    // The scene: a project file fetched from the same server.
    say(&format!("loading {scene_url} ..."));
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(scene_url)).await.map_err(js)?.dyn_into().map_err(|_| "no response".to_string())?;
    if !response.ok() {
        return Err(format!("could not load {scene_url}: HTTP {}", response.status()));
    }
    let text = JsFuture::from(response.text().map_err(js)?).await.map_err(js)?.as_string().ok_or("the file is not text")?;
    let loading = web_time::Instant::now();
    let project = evolve_engine::io::parse_project(&text).map_err(|e| format!("could not read the project: {e}"))?;
    drop(text);
    let scene = project.scenes.get(project.start).or(project.scenes.first()).ok_or("the project has no scene")?.clone();
    say(&format!("scene '{}': {} objects, read in {:.0} ms", scene.name, scene.all_ids().len(), loading.elapsed().as_secs_f64() * 1000.0));

    // Grow the program's memory once, to what this scene needs (see `evolve_gpu::reserve_memory`).
    let growing = web_time::Instant::now();
    let reserved = evolve_gpu::memory_for_shapes(scene.all_ids().len());
    evolve_gpu::reserve_memory(reserved);
    say(&format!("memory made ready ({} MB): {:.0} ms", reserved >> 20, growing.elapsed().as_secs_f64() * 1000.0));

    let unsupported = Renderer::unsupported(&scene);
    let skipped = unsupported.iter().map(|(what, count)| format!("{what:?} x{count}")).collect::<Vec<_>>().join(", ");
    let options = RenderOptions { camera_frame: false, skip_unsupported: !unsupported.is_empty(), ..RenderOptions::default() };

    // Three views, like the benchmark's: the whole scene, a region and a close-up.
    let bounds = scene.selection_bounds(&scene.all_ids()).ok_or("the scene has nothing to draw")?;
    let (center, extent) = (bounds.center(), bounds.size());
    let fit = (size.0 as f32 / extent.x.max(1e-3)).min(size.1 as f32 / extent.y.max(1e-3));
    let make = |name: &'static str, factor: f32| View {
        name,
        matrix: Affine2::from_translation(Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5)) * Affine2::from_scale(Vec2::splat(fit * factor)) * Affine2::from_translation(-center),
        cpu: Vec::new(),
        interval: Vec::new(),
        first_frame_ms: 0.0,
        first_stats: None,
        stats: None,
    };
    let views = vec![make("whole scene", 1.0), make("region (about 10% visible)", 3.16), make("close-up (about 0.5% visible)", 14.0)];

    let renderer = Renderer::new(&gpu).map_err(|e| format!("could not start the renderer: {e}"))?;
    let blit = Blit::new(&gpu.device, format);
    let canary = Rc::new(RefCell::new(Canary {
        gpu,
        surface,
        size,
        scene,
        renderer,
        options,
        blit,
        views,
        current: 0,
        frame: 0,
        frames_per_view,
        last_timestamp: None,
        skipped,
        started: web_time::Instant::now(),
    }));

    // The animation loop: one view after the other, `frames_per_view` frames each, then the report and stop.
    let callback: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let again = callback.clone();
    let state = canary.clone();
    *callback.borrow_mut() = Some(Closure::new(move |timestamp: f64| {
        let more = state.borrow_mut().step(timestamp);
        if more {
            if let Some(window) = web_sys::window() {
                let _ = window.request_animation_frame(again.borrow().as_ref().expect("the callback exists").as_ref().unchecked_ref());
            }
        } else {
            state.borrow().report();
            again.borrow_mut().take(); // the loop is over: the closure can go
        }
    }));
    window.request_animation_frame(callback.borrow().as_ref().expect("just set").as_ref().unchecked_ref()).map_err(js)?;
    say(&format!("running {} views of {frames_per_view} frames ...", 3));
    Ok(())
}
