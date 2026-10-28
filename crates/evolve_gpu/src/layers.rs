//! The pool of layer textures (plan D13, tasks 4.2 and 4.3).
//!
//! A layer is the picture of one node (a group with opacity, say) on its own, drawn in a pass before the pass that uses it and
//! shown there as a rectangle. It is as large as the node's box on the screen (clipped to the view), never the whole view.
//!
//! Sizes are rounded up to a **size class** (four classes to every doubling, so at most a quarter too large on a side) so
//! that textures can be reused from frame to frame while the sizes wobble. Per class there are
//! - the **layer textures**, single-sample, kept while their layer is in use (4 bytes a pixel; the budget of `Layers`), and
//! - one **scratch** set shared by every layer of the class: the 4x multisampled color and depth-stencil that a layer's pass
//!   draws in. Their contents are discarded when the pass ends, so one set serves all layers one after the other (32 bytes
//!   a pixel; counted with the other render targets).
//!
//! Every frame starts with [`LayerPool::begin_frame`], which takes all layers back; [`LayerPool::acquire`] hands them out again.
//! Classes that have not been used for a while are dropped, textures and scratch with them.

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use crate::fast_map::FastMap;
use crate::targets::{COLOR_FORMAT, DEPTH_FORMAT, SAMPLES};

/// The smallest side of a class.
const MIN_SIDE: u32 = 32;
/// A class is dropped when no layer of it was asked for in this many frames.
const KEEP_FRAMES: u64 = 240;

/// The class of one side of `n` pixels: `n` rounded up to a multiple of a quarter of the largest power of two not above it.
pub fn class_side(n: u32, limit: u32) -> u32 {
    let n = n.max(MIN_SIDE);
    let step = ((1u32 << n.ilog2()) / 4).max(1);
    (n.div_ceil(step) * step).min(limit.max(n))
}

/// The class of a layer of `size` pixels.
pub fn size_class(size: (u32, u32), limit: u32) -> (u32, u32) {
    (class_side(size.0, limit), class_side(size.1, limit))
}

/// A layer's texture, and the bind group that the composite reads it through.
pub struct Layer {
    class: (u32, u32),
    texture: Tracked<wgpu::Texture>,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl Layer {
    /// The size of the texture (its class).
    pub fn size(&self) -> (u32, u32) {
        self.class
    }

    /// Where the pass of the layer resolves to.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Group 1 of the composite pipeline.
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

/// The multisampled color and depth-stencil that the passes of a class draw in.
pub struct Scratch {
    _color: Tracked<wgpu::Texture>,
    color_view: wgpu::TextureView,
    _depth: Tracked<wgpu::Texture>,
    depth_view: wgpu::TextureView,
}

impl Scratch {
    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }
}

#[derive(Default)]
struct Class {
    scratch: Option<Scratch>,
    free: Vec<Layer>,
    last_used: u64,
}

pub struct LayerPool {
    alloc: Alloc,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    classes: FastMap<(u32, u32), Class>,
    in_use: Vec<Layer>,
    frame: u64,
}

impl LayerPool {
    /// `layout` is the composite pipeline's group 1, `sampler` the one its second binding wants.
    pub fn new(alloc: &Alloc, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler) -> LayerPool {
        LayerPool { alloc: alloc.clone(), layout: layout.clone(), sampler: sampler.clone(), classes: FastMap::default(), in_use: Vec::new(), frame: 0 }
    }

    /// Takes back every layer: the textures can be handed out again (the passes that used them are submitted).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        for layer in self.in_use.drain(..) {
            self.classes.entry(layer.class).or_default().free.push(layer);
        }
    }

    /// Drops the classes that have not been used for a while.
    pub fn end_frame(&mut self) {
        let (frame, in_use) = (self.frame, &self.in_use);
        self.classes.retain(|class, entry| entry.last_used + KEEP_FRAMES >= frame || in_use.iter().any(|l| l.class == *class));
    }

    /// A layer for `size` pixels: its number for [`LayerPool::get`], valid until the next `begin_frame`.
    pub fn acquire(&mut self, size: (u32, u32)) -> Result<usize, AllocError> {
        let limit = self.alloc.device().limits().max_texture_dimension_2d;
        let class = size_class(size, limit);
        let frame = self.frame;
        // The scratch first: if it cannot be had, nothing was taken.
        let entry = self.classes.entry(class).or_default();
        entry.last_used = frame;
        if entry.scratch.is_none() {
            entry.scratch = Some(create_scratch(&self.alloc, class)?);
        }
        let layer = match entry.free.pop() {
            Some(layer) => layer,
            None => create_layer(&self.alloc, &self.layout, &self.sampler, class)?,
        };
        self.in_use.push(layer);
        Ok(self.in_use.len() - 1)
    }

    /// Gives layer `number` a texture for `size` pixels in place of the one it has, when that is of another class: the old one goes
    /// back to the free layers of its class. The number stays valid. An error leaves the layer as it was. For a layer whose
    /// rectangle changed while its frame is drawn again.
    pub fn resize(&mut self, number: usize, size: (u32, u32)) -> Result<(), AllocError> {
        let limit = self.alloc.device().limits().max_texture_dimension_2d;
        let class = size_class(size, limit);
        if self.in_use[number].class == class {
            return Ok(());
        }
        let frame = self.frame;
        let entry = self.classes.entry(class).or_default();
        entry.last_used = frame;
        if entry.scratch.is_none() {
            entry.scratch = Some(create_scratch(&self.alloc, class)?);
        }
        let layer = match entry.free.pop() {
            Some(layer) => layer,
            None => create_layer(&self.alloc, &self.layout, &self.sampler, class)?,
        };
        let old = std::mem::replace(&mut self.in_use[number], layer);
        self.classes.entry(old.class).or_default().free.push(old);
        Ok(())
    }

    pub fn get(&self, number: usize) -> &Layer {
        &self.in_use[number]
    }

    /// The scratch set of a layer's class.
    pub fn scratch_of(&self, number: usize) -> &Scratch {
        self.classes[&self.in_use[number].class].scratch.as_ref().expect("a class in use has its scratch")
    }

    /// Layers handed out in this frame.
    pub fn in_use(&self) -> usize {
        self.in_use.len()
    }

    /// Pixels of the layers handed out in this frame (their classes' sizes).
    pub fn pixels_in_use(&self) -> u64 {
        self.in_use.iter().map(|l| l.class.0 as u64 * l.class.1 as u64).sum()
    }

    /// Bytes of layer textures and of scratch the pool holds (in use or kept).
    pub fn bytes(&self) -> u64 {
        let layers = self.in_use.iter().chain(self.classes.values().flat_map(|c| &c.free)).map(|l| l.texture.bytes());
        let scratch = self.classes.values().filter_map(|c| c.scratch.as_ref()).map(|s| s._color.bytes() + s._depth.bytes());
        layers.chain(scratch).sum()
    }

    /// Number of size classes held.
    pub fn classes(&self) -> usize {
        self.classes.len()
    }
}

fn create_layer(alloc: &Alloc, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, class: (u32, u32)) -> Result<Layer, AllocError> {
    let texture = alloc.create_texture(
        Category::Layers,
        &wgpu::TextureDescriptor {
            label: Some("layer"),
            size: wgpu::Extent3d { width: class.0, height: class.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        },
    )?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = alloc.device().create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("layer"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    Ok(Layer { class, texture, view, bind_group })
}

fn create_scratch(alloc: &Alloc, class: (u32, u32)) -> Result<Scratch, AllocError> {
    let texture = |label: &'static str, format| {
        alloc.create_texture(
            Category::Targets,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: class.0, height: class.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: SAMPLES,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        )
    };
    let color = texture("layer color (multisampled)", COLOR_FORMAT)?;
    let depth = texture("layer depth and stencil (multisampled)", DEPTH_FORMAT)?;
    Ok(Scratch {
        color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
        depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
        _color: color,
        _depth: depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Gpu;
    use crate::pipeline::Pipeline;

    #[test]
    fn sizes_round_up_to_four_classes_per_doubling_and_never_shrink() {
        let limit = 16384;
        assert_eq!(class_side(1, limit), 32);
        assert_eq!(class_side(32, limit), 32);
        assert_eq!(class_side(33, limit), 40);
        assert_eq!(class_side(100, limit), 112);
        assert_eq!(class_side(1080, limit), 1280);
        assert_eq!(class_side(1920, limit), 2048);
        assert_eq!(class_side(2048, limit), 2048);
        for n in (1..5000).step_by(7) {
            let c = class_side(n, limit);
            assert!(c >= n.max(MIN_SIDE) && c as f32 <= n.max(MIN_SIDE) as f32 * 1.25 + 1.0, "{n} -> {c}");
        }
        // The device's limit wins over the rounding (but a size within the limit still fits).
        assert_eq!(class_side(1900, 1920), 1920);
        assert_eq!(size_class((100, 1080), limit), (112, 1280));
    }

    #[test]
    fn layers_are_handed_out_taken_back_reused_and_forgotten_when_not_used_for_long() {
        Gpu::for_each_test_mode(|gpu| {
            let pipeline = Pipeline::new(&gpu.device);
            let mut pool = LayerPool::new(&gpu.alloc, &pipeline.layer_layout, &pipeline.layer_sampler);
            let before = gpu.alloc.snapshot();
            pool.begin_frame();
            let a = pool.acquire((100, 60)).unwrap();
            let b = pool.acquire((105, 62)).unwrap(); // the same class (112 x 64), a second texture
            let c = pool.acquire((300, 40)).unwrap();
            assert_eq!((pool.get(a).size(), pool.get(b).size(), pool.get(c).size()), ((112, 64), (112, 64), (320, 40)));
            assert_eq!((pool.in_use(), pool.classes()), (3, 2));
            assert_eq!(gpu.alloc.used(Category::Layers), 4 * (112 * 64 * 2 + 320 * 40));
            assert_eq!(gpu.alloc.used(Category::Targets), 32 * (112 * 64 + 320 * 40), "one scratch set per class, not per layer");
            // The next frame asks for the same and gets the same textures: nothing is made.
            let bytes = pool.bytes();
            pool.begin_frame();
            pool.acquire((100, 60)).unwrap();
            pool.acquire((100, 60)).unwrap();
            pool.acquire((300, 40)).unwrap();
            assert_eq!(pool.bytes(), bytes);
            // A frame that asks for less: the other class is kept for a while, then dropped.
            for _ in 0..KEEP_FRAMES + 2 {
                pool.begin_frame();
                pool.acquire((100, 60)).unwrap();
                pool.end_frame();
            }
            assert_eq!(pool.classes(), 1, "the unused class was dropped");
            assert_eq!(gpu.alloc.used(Category::Layers), 4 * 112 * 64 * 2, "one layer of the used class was kept, one went with nothing asking for it: {}", gpu.alloc.used(Category::Layers));
            drop(pool);
            assert_eq!(gpu.alloc.snapshot(), before, "dropping the pool gave everything back");
        });
    }

    #[test]
    fn a_layer_gets_a_texture_of_another_class_in_place_and_the_old_one_is_kept_for_the_next() {
        Gpu::for_each_test_mode(|gpu| {
            let pipeline = Pipeline::new(&gpu.device);
            let mut pool = LayerPool::new(&gpu.alloc, &pipeline.layer_layout, &pipeline.layer_sampler);
            pool.begin_frame();
            let a = pool.acquire((100, 60)).unwrap();
            let b = pool.acquire((300, 40)).unwrap();
            // The same class: nothing is made and nothing moves.
            let bytes = pool.bytes();
            pool.resize(a, (105, 62)).unwrap();
            assert_eq!((pool.get(a).size(), pool.bytes()), ((112, 64), bytes));
            // Another class: the layer keeps its number and has a texture of the new class (with the scratch of that class).
            pool.resize(a, (200, 100)).unwrap();
            assert_eq!((pool.get(a).size(), pool.get(b).size(), pool.in_use()), ((224, 112), (320, 40), 2));
            assert!(pool.bytes() > bytes);
            assert_eq!(gpu.alloc.used(Category::Layers), 4 * (224 * 112 + 112 * 64 + 320 * 40), "the old texture is kept free, not given back");
            // Back to the first class: the kept texture is used again and nothing new is made.
            let bytes = pool.bytes();
            pool.resize(a, (100, 60)).unwrap();
            assert_eq!((pool.get(a).size(), pool.bytes()), ((112, 64), bytes));
            // A texture that does not fit the budget is an error and the layer stays as it was.
            let used = gpu.alloc.used(Category::Layers);
            gpu.alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Layers, used));
            let refused = pool.resize(a, (1000, 800)).expect_err("a large texture does not fit");
            assert!(matches!(refused, AllocError::OverBudget { category: Category::Layers, .. }), "{refused}");
            assert_eq!((pool.get(a).size(), pool.in_use()), ((112, 64), 2));
            gpu.alloc.set_budgets(crate::alloc::Budgets::high());
            drop(pool);
        });
    }

    #[test]
    fn running_out_of_budget_is_an_error_that_leaves_the_pool_as_it_was() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            let pipeline = Pipeline::new(&gpu.device);
            let mut pool = LayerPool::new(&alloc, &pipeline.layer_layout, &pipeline.layer_sampler);
            alloc.set_budgets(crate::alloc::Budgets::high().with(Category::Layers, 4 * 64 * 64 * 2));
            pool.begin_frame();
            pool.acquire((64, 64)).unwrap();
            pool.acquire((64, 64)).unwrap();
            let refused = pool.acquire((64, 64)).expect_err("a third one does not fit");
            assert!(matches!(refused, AllocError::OverBudget { category: Category::Layers, .. }), "{refused}");
            assert_eq!(pool.in_use(), 2);
            let refused = pool.acquire((2000, 1000)).expect_err("nor a large one");
            assert!(matches!(refused, AllocError::OverBudget { .. }), "{refused}");
            assert_eq!(pool.in_use(), 2);
            alloc.set_budgets(crate::alloc::Budgets::high());
            drop(pool);
        });
    }
}
