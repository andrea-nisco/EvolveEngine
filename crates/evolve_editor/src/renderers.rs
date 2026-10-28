//! Which renderer draws the Scene and Game views (plan tasks 2.9, 2.11, 2.17).
//!
//! The CPU renderer (tiny-skia) is the reference and the fallback; the GPU renderer is chosen with `--renderer auto|cpu|gpu`
//! or *View → Renderer*. Until Phase 7 of `GPU_RENDERER_PLAN.md` the default is the CPU renderer.
//!
//! A view that is asked to use the GPU still falls back to the CPU renderer, with a small notice in the view, when
//! - the scene has something the GPU renderer cannot draw yet (the **feature gate**: a skinned clip mask, or a skinned shape inside a
//!   symbol instance; everything else, skinned shapes included, is drawn), so that no
//!   wrong picture is ever shown;
//! - the machine has no usable graphics device, or the device was lost (driver reset, sleep), or the card reported an
//!   error, or a resource could not be had (GPU_SAFETY.md D1, D3). The editor does not crash; it says so and carries on.

use eframe::egui_wgpu::RenderState;
use evolve_engine::revision::{Revs, Stamp};
use evolve_engine::scene::Scene;
use evolve_gpu::draw_list::Unsupported;
use evolve_gpu::egui_glue::{GpuContext, GpuView, Painted};
use evolve_gpu::profile::{Quality, QualityProfile};
use evolve_gpu::renderer::{RenderOptions, Renderer};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RendererChoice {
    /// The GPU renderer where it can draw the scene, the CPU renderer elsewhere (the default of the future).
    Auto,
    /// Always the CPU renderer (today's default).
    #[default]
    Cpu,
    /// The GPU renderer, with the CPU renderer only as the fallback, and a notice whenever it is used.
    Gpu,
}

impl RendererChoice {
    pub const ALL: [RendererChoice; 3] = [RendererChoice::Auto, RendererChoice::Cpu, RendererChoice::Gpu];

    /// Reads the value of `--renderer`.
    pub fn parse(text: &str) -> Option<RendererChoice> {
        match text.to_ascii_lowercase().as_str() {
            "auto" => Some(RendererChoice::Auto),
            "cpu" => Some(RendererChoice::Cpu),
            "gpu" => Some(RendererChoice::Gpu),
            _ => None,
        }
    }

    /// Finds `--renderer <value>` in the command line. `Err` says what is wrong with it.
    pub fn from_args(args: &[String]) -> Result<RendererChoice, String> {
        let Some(at) = args.iter().position(|a| a == "--renderer") else { return Ok(RendererChoice::default()) };
        let value = args.get(at + 1).ok_or("--renderer needs a value: auto, cpu or gpu")?;
        RendererChoice::parse(value).ok_or_else(|| format!("--renderer {value}: expected auto, cpu or gpu"))
    }
}

impl fmt::Display for RendererChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RendererChoice::Auto => "Automatic",
            RendererChoice::Cpu => "CPU",
            RendererChoice::Gpu => "GPU",
        })
    }
}

/// The choice, and the graphics device the GPU renderer would use.
pub struct RendererSettings {
    pub choice: RendererChoice,
    /// The quality asked for (plan D15); the context holds the one that is in force, this is what is remembered while there is none.
    quality: Quality,
    context: Option<GpuContext>,
    /// Why the GPU renderer is off although it was asked for (device lost, a reported error): once set it stays set.
    disabled: RefCell<Option<String>>,
    /// The message about that, until the status bar has shown it.
    announcement: RefCell<Option<String>>,
}

impl RendererSettings {
    /// With the device eframe made, if it runs on wgpu.
    pub fn new(choice: RendererChoice, render_state: Option<&RenderState>) -> RendererSettings {
        RendererSettings { choice, quality: Quality::default(), context: render_state.map(GpuContext::from_render_state), disabled: RefCell::default(), announcement: RefCell::default() }
    }

    /// With no graphics device at all (tests, and a machine where eframe could not make one).
    pub fn without_gpu(choice: RendererChoice) -> RendererSettings {
        RendererSettings { choice, quality: Quality::default(), context: None, disabled: RefCell::default(), announcement: RefCell::default() }
    }

    #[cfg(test)]
    pub fn with_context(choice: RendererChoice, context: GpuContext) -> RendererSettings {
        RendererSettings { choice, quality: Quality::default(), context: Some(context), disabled: RefCell::default(), announcement: RefCell::default() }
    }

    /// The quality of the GPU renderer's pictures: automatic (the card's own profile, lowered while frames are slow) or one profile.
    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Changes it for every view at once (the memory caps at once, the render scale at each view's next paint).
    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
        if let Some(context) = &self.context {
            context.set_quality(quality);
        }
    }

    /// The profile the graphics card deserves before anything is measured, if there is a card.
    pub fn adapter_profile(&self) -> Option<QualityProfile> {
        self.context.as_ref().map(|c| c.adapter_profile())
    }

    /// The graphics card, as text, if there is one.
    pub fn adapter(&self) -> Option<String> {
        self.context.as_ref().map(|c| c.gpu.to_string())
    }

    pub fn disabled_because(&self) -> Option<String> {
        self.disabled.borrow().clone()
    }

    /// The device the GPU renderer may use now: asked for, present, not lost, not in error. Every view asks this before it
    /// draws, and asking looks at the device's error log, so a lost device or a reported error is noticed by the next view
    /// that is painted and never reaches the card again.
    pub fn usable_context(&self) -> Option<&GpuContext> {
        self.check();
        if self.choice == RendererChoice::Cpu || self.disabled.borrow().is_some() {
            return None;
        }
        self.context.as_ref()
    }

    /// Looks at the device's error log. A lost device or a reported error switches the GPU renderer off for good (the
    /// views go on with the CPU renderer) and leaves a message for the status bar.
    fn check(&self) {
        if self.disabled.borrow().is_some() {
            return;
        }
        let Some(context) = self.context.as_ref() else { return };
        let problem = match context.gpu.errors.lost() {
            Some(why) => format!("the graphics device was lost ({why})"),
            None => match context.gpu.errors.take().first() {
                Some(error) => format!("the graphics card reported an error ({})", error.lines().next().unwrap_or("unknown")),
                None => return,
            },
        };
        let message = format!("GPU renderer switched off: {problem}. Drawing with the CPU renderer.");
        *self.disabled.borrow_mut() = Some(message.clone());
        *self.announcement.borrow_mut() = Some(message);
    }

    /// The message about a device that was lost or reported an error, once, for the status bar.
    pub fn poll(&mut self) -> Option<String> {
        self.check();
        self.announcement.borrow_mut().take()
    }
}

/// Words for what the GPU renderer cannot draw yet.
fn describe(unsupported: &BTreeMap<Unsupported, usize>) -> String {
    let names: Vec<&str> = unsupported
        .keys()
        .map(|what| match what {
            Unsupported::Skinned => "skinned shapes",
        })
        .collect();
    names.join(", ")
}

/// The GPU side of one view: its renderer, what the feature gate found for the scene it last saw, and how often it drew.
pub struct GpuTarget {
    view: GpuView,
    /// The stamps and journal the gate was made at, and what it found.
    gate: Option<(Revs, Stamp, BTreeMap<Unsupported, usize>)>,
    draws: u64,
    idle: u64,
    /// How many times the gate was made anew.
    gates: u64,
}

/// How the card draws a view. The camera's frame is not part of the picture: `RenderTarget` puts it under it, so that a camera that
/// moves or zooms every frame (a running game) leaves the picture of the scene as it was and costs the card nothing.
pub(crate) fn render_options() -> RenderOptions {
    RenderOptions { camera_frame: false, ..RenderOptions::default() }
}

/// What painting a view with the GPU renderer came to.
pub enum GpuOutcome {
    /// The GPU renderer showed the scene (drawing it, or reusing the picture of an unchanged scene).
    Shown,
    /// Nothing to show (the view has no size).
    Nothing,
    /// Use the CPU renderer for this view; the text is the notice to show.
    Fallback(String),
}

impl GpuTarget {
    pub fn new(context: &GpuContext, name: &'static str) -> Result<GpuTarget, String> {
        let view = GpuView::new(context, name).map_err(|e| format!("GPU renderer: {e}"))?;
        Ok(GpuTarget { view, gate: None, draws: 0, idle: 0, gates: 0 })
    }

    /// How many times the scene was really drawn, and how many paints reused the picture.
    #[cfg(test)]
    pub fn counts(&self) -> (u64, u64) {
        (self.draws, self.idle)
    }

    /// How many times the feature gate looked through the scene.
    #[cfg(test)]
    pub fn gates_made(&self) -> u64 {
        self.gates
    }

    pub fn paint(&mut self, painter: &eframe::egui::Painter, rect: eframe::egui::Rect, scene: &Scene, view: &dyn Fn(evolve_engine::Vec2) -> evolve_engine::Affine2) -> GpuOutcome {
        // The feature gate, remembered for as long as the scene's change stamps stay the same, or only a camera changed (a game moves its
        // camera every frame, and looking at every node for that would cost more than the frame).
        let revs = scene.revision();
        let epoch = scene.journal_epoch();
        if self.gate.as_ref().is_none_or(|(seen, was, _)| *seen != revs && !scene.only_cameras_changed_since(seen, *was)) {
            self.gate = Some((revs, epoch, Renderer::unsupported(scene)));
            self.gates += 1;
        }
        let unsupported = &self.gate.as_ref().expect("just set").2;
        if !unsupported.is_empty() {
            return GpuOutcome::Fallback(format!("CPU renderer: the GPU renderer cannot draw {} yet", describe(unsupported)));
        }
        match self.view.paint(painter, rect, scene, view, &render_options()) {
            Ok(Painted::Nothing) => GpuOutcome::Nothing,
            Ok(Painted::Idle) => {
                self.idle += 1;
                GpuOutcome::Shown
            }
            Ok(Painted::Drawn(_)) => {
                self.draws += 1;
                GpuOutcome::Shown
            }
            Err(error) => GpuOutcome::Fallback(format!("CPU renderer: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_renderer_option_is_read_from_the_command_line() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(RendererChoice::from_args(&args(&[])), Ok(RendererChoice::Cpu), "the default stays the CPU renderer until Phase 7");
        assert_eq!(RendererChoice::from_args(&args(&["--renderer", "gpu"])), Ok(RendererChoice::Gpu));
        assert_eq!(RendererChoice::from_args(&args(&["--play", "--renderer", "AUTO", "--other"])), Ok(RendererChoice::Auto));
        assert_eq!(RendererChoice::from_args(&args(&["--renderer", "cpu"])), Ok(RendererChoice::Cpu));
        assert!(RendererChoice::from_args(&args(&["--renderer"])).is_err());
        assert!(RendererChoice::from_args(&args(&["--renderer", "vulkan"])).unwrap_err().contains("expected auto, cpu or gpu"));
    }

    #[test]
    fn without_a_device_or_with_the_cpu_choice_no_gpu_context_is_offered() {
        for choice in RendererChoice::ALL {
            assert!(RendererSettings::without_gpu(choice).usable_context().is_none());
        }
        assert!(RendererSettings::without_gpu(RendererChoice::Gpu).adapter().is_none());
    }

    #[test]
    fn the_quality_choice_is_remembered_even_without_a_graphics_device() {
        let mut settings = RendererSettings::without_gpu(RendererChoice::Gpu);
        assert_eq!(settings.quality(), Quality::Auto, "automatic unless told otherwise");
        for quality in Quality::ALL {
            settings.set_quality(quality);
            assert_eq!(settings.quality(), quality);
        }
        assert!(settings.adapter_profile().is_none(), "no card, no profile");
    }

    #[test]
    fn the_card_does_not_draw_the_cameras_frame() {
        assert!(!render_options().camera_frame, "the view paints it under the picture (canvas::paint_camera_frame)");
    }

    #[test]
    fn the_gate_names_what_is_missing_in_words() {
        let mut found = BTreeMap::new();
        found.insert(Unsupported::Skinned, 3);
        assert_eq!(describe(&found), "skinned shapes");
    }
}
