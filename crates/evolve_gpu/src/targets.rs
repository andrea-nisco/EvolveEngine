//! The textures a view is drawn into (plan D2, D4, task 2.6).
//!
//! The scene is drawn into textures that the renderer owns, not into egui's own pass, because egui's pass has no stencil and
//! no multisampling: a 4x multisampled color texture, a 4x multisampled depth-stencil texture, and a single-sample
//! texture that the multisampled one is resolved into and that is shown. The two multisampled ones are only ever used
//! inside the pass (stored as `Discard`, which keeps them in the fast memory of integrated and tile-based cards).
//!
//! Dragging the edge of a panel changes the view's size on every frame, and a texture cannot be resized, so the targets
//! are **grow-only with slack**: when the view gets bigger than the targets, new ones are made 25% larger than needed, and
//! they are made smaller again only after the view has needed much less for a few seconds. Between those the view is
//! drawn into the top left corner of the targets.

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use crate::readback::PICTURE_FORMAT;
use std::time::Duration;
use web_time::Instant;

/// Samples per pixel of the scene pass (1 or 4 are the only counts WebGL2 offers).
pub const SAMPLES: u32 = 4;
pub const COLOR_FORMAT: wgpu::TextureFormat = PICTURE_FORMAT;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

#[derive(Clone, Copy, Debug)]
pub struct TargetPolicy {
    /// How much larger than the view new targets are made (0.25 is a quarter).
    pub slack: f32,
    /// How long the view must need less than half the area of the targets before they are made smaller.
    pub shrink_after: Duration,
}

impl Default for TargetPolicy {
    fn default() -> Self {
        Self { slack: 0.25, shrink_after: Duration::from_secs(3) }
    }
}

impl TargetPolicy {
    /// Targets of exactly the size of the view, for tests that read pictures back.
    pub fn exact() -> Self {
        Self { slack: 0.0, shrink_after: Duration::from_secs(3) }
    }
}

struct Set {
    size: (u32, u32),
    msaa_color: Tracked<wgpu::Texture>,
    msaa_color_view: wgpu::TextureView,
    msaa_depth_view: wgpu::TextureView,
    _msaa_depth: Tracked<wgpu::Texture>,
    resolve: Tracked<wgpu::Texture>,
    resolve_view: wgpu::TextureView,
}

pub struct Targets {
    alloc: Alloc,
    set: Option<Set>,
    smaller_since: Option<Instant>,
    /// Changes whenever the textures are replaced; whoever shows the resolved picture must pick up the new one.
    generation: u64,
}

impl Targets {
    pub fn new(alloc: &Alloc) -> Targets {
        Targets { alloc: alloc.clone(), set: None, smaller_since: None, generation: 0 }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The size of the textures, if there are any.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.set.as_ref().map(|s| s.size)
    }

    pub fn msaa_color_view(&self) -> Option<&wgpu::TextureView> {
        self.set.as_ref().map(|s| &s.msaa_color_view)
    }

    pub fn msaa_depth_view(&self) -> Option<&wgpu::TextureView> {
        self.set.as_ref().map(|s| &s.msaa_depth_view)
    }

    pub fn resolve_view(&self) -> Option<&wgpu::TextureView> {
        self.set.as_ref().map(|s| &s.resolve_view)
    }

    /// The single-sample picture, for reading back or showing.
    pub fn resolve_texture(&self) -> Option<&wgpu::Texture> {
        self.set.as_ref().map(|s| &*s.resolve)
    }

    /// Bytes the targets take.
    pub fn bytes(&self) -> u64 {
        self.set.as_ref().map_or(0, |s| s.msaa_color.bytes() + s._msaa_depth.bytes() + s.resolve.bytes())
    }

    /// Makes sure the targets can hold a view of `view` pixels. Returns true when the textures were replaced.
    /// `now` is passed in so that the shrinking can be tested.
    pub fn ensure(&mut self, view: (u32, u32), policy: &TargetPolicy, now: Instant) -> Result<bool, AllocError> {
        let view = (view.0.max(1), view.1.max(1));
        let limit = self.alloc.device().limits().max_texture_dimension_2d;
        let padded = |n: u32| ((n as f32 * (1.0 + policy.slack)).ceil() as u32).clamp(n, limit.max(n));
        match &self.set {
            None => {}
            Some(set) if view.0 > set.size.0 || view.1 > set.size.1 => {}
            Some(set) => {
                // Big enough. Make them smaller only if the view has needed less than half the area for a while.
                let needed = padded(view.0) as u64 * padded(view.1) as u64;
                if (set.size.0 as u64 * set.size.1 as u64) > needed * 2 {
                    let since = *self.smaller_since.get_or_insert(now);
                    if now.duration_since(since) < policy.shrink_after {
                        return Ok(false);
                    }
                } else {
                    self.smaller_since = None;
                    return Ok(false);
                }
            }
        }
        self.smaller_since = None;
        // The old textures go first, so that the budget is not asked to hold both.
        self.set = None;
        self.set = Some(create(&self.alloc, (padded(view.0), padded(view.1)))?);
        self.generation += 1;
        Ok(true)
    }
}

fn create(alloc: &Alloc, size: (u32, u32)) -> Result<Set, AllocError> {
    let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
    let texture = |label: &'static str, format, samples, usage| {
        alloc.create_texture(
            Category::Targets,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            },
        )
    };
    let msaa_color = texture("scene color (multisampled)", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT)?;
    let msaa_depth = texture("scene depth and stencil (multisampled)", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT)?;
    let resolve = texture(
        "scene picture",
        COLOR_FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
    )?;
    let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Set {
        size,
        msaa_color_view: view(&msaa_color),
        msaa_depth_view: view(&msaa_depth),
        resolve_view: view(&resolve),
        msaa_color,
        _msaa_depth: msaa_depth,
        resolve,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Gpu;

    #[test]
    fn exact_targets_have_the_size_of_the_view_and_slack_adds_a_quarter() {
        Gpu::for_each_test_mode(|gpu| {
            let mut exact = Targets::new(&gpu.alloc);
            assert!(exact.ensure((300, 200), &TargetPolicy::exact(), Instant::now()).unwrap());
            assert_eq!(exact.size(), Some((300, 200)));
            let mut slack = Targets::new(&gpu.alloc);
            slack.ensure((300, 200), &TargetPolicy::default(), Instant::now()).unwrap();
            assert_eq!(slack.size(), Some((375, 250)));
            // The plan's formula: color and depth-stencil at 4 samples and the picture, 16 + 16 + 4 bytes per pixel.
            assert_eq!(exact.bytes(), 300 * 200 * (16 + 16 + 4));
        });
    }

    #[test]
    fn dragging_a_panel_edge_does_not_reallocate_on_every_frame() {
        Gpu::for_each_test_mode(|gpu| {
            let mut targets = Targets::new(&gpu.alloc);
            let policy = TargetPolicy::default();
            let start = Instant::now();
            assert!(targets.ensure((400, 300), &policy, start).unwrap());
            let generation = targets.generation();
            // The view wobbles in size by a few pixels for a hundred frames: nothing is replaced.
            for frame in 0..100u32 {
                let size = (400 - 20 + (frame * 7) % 40, 300 - 15 + (frame * 5) % 30);
                let replaced = targets.ensure(size, &policy, start + Duration::from_millis(frame as u64 * 16)).unwrap();
                assert!(!replaced, "frame {frame}, size {size:?}");
            }
            assert_eq!(targets.generation(), generation);
            // A larger view replaces them once, with slack, and then holds for the following frames.
            assert!(targets.ensure((600, 300), &policy, start).unwrap());
            assert_eq!(targets.size(), Some((750, 375)));
            assert!(!targets.ensure((700, 350), &policy, start).unwrap());
            assert_eq!(targets.generation(), generation + 1);
        });
    }

    #[test]
    fn targets_are_made_smaller_only_after_the_view_stayed_small_for_a_while() {
        Gpu::for_each_test_mode(|gpu| {
            let mut targets = Targets::new(&gpu.alloc);
            let policy = TargetPolicy { slack: 0.25, shrink_after: Duration::from_secs(3) };
            let t0 = Instant::now();
            targets.ensure((1000, 800), &policy, t0).unwrap();
            let big = targets.bytes();
            // A small view: nothing happens at once, or after one second.
            assert!(!targets.ensure((300, 200), &policy, t0).unwrap());
            assert!(!targets.ensure((300, 200), &policy, t0 + Duration::from_secs(1)).unwrap());
            assert_eq!(targets.size(), Some((1250, 1000)));
            // A moment when the view is big again restarts the wait.
            assert!(!targets.ensure((1000, 800), &policy, t0 + Duration::from_secs(2)).unwrap());
            assert!(!targets.ensure((300, 200), &policy, t0 + Duration::from_secs(4)).unwrap());
            assert!(!targets.ensure((300, 200), &policy, t0 + Duration::from_secs(6)).unwrap());
            // Three seconds after it became small (again), they shrink.
            assert!(targets.ensure((300, 200), &policy, t0 + Duration::from_secs(7)).unwrap());
            assert_eq!(targets.size(), Some((375, 250)));
            assert!(targets.bytes() < big / 5);
        });
    }

    #[test]
    fn targets_over_the_budget_or_the_device_limit_are_an_error_and_everything_is_given_back() {
        Gpu::for_each_test_mode(|gpu| {
            let before = gpu.alloc.snapshot();
            {
                let mut targets = Targets::new(&gpu.alloc);
                let limit = gpu.device.limits().max_texture_dimension_2d;
                // A view as wide as the device allows needs more slack than it can have: clamped to the limit.
                if limit <= 4096 {
                    assert!(targets.ensure((limit, 64), &TargetPolicy::default(), Instant::now()).unwrap());
                    assert_eq!(targets.size().unwrap().0, limit, "slack never goes past the device limit");
                }
                let error = Targets::new(&gpu.alloc).ensure((limit + 1, 10), &TargetPolicy::exact(), Instant::now()).expect_err("too wide");
                assert!(matches!(error, AllocError::TooLarge { .. }), "{error}");
            }
            assert_eq!(gpu.alloc.snapshot(), before);
        });
    }
}
