//! The one place where graphics memory is allocated (GPU_SAFETY.md, rules C1 to C3).
//!
//! Every buffer and texture of the renderer is created through [`Alloc`], which
//! - counts the bytes of everything alive, per [`Category`], so that the totals can be shown, tested and compared;
//! - refuses a request that would go over the budget of its category, or over the device's own limits, *before* asking
//!   the card, and says so with an error the caller can act on (smaller layers, a coarser tolerance, the CPU renderer);
//! - asks the card inside error scopes, so that an out-of-memory or invalid request comes back as an error value and not
//!   as a crash.
//!
//! Nothing outside this module may call `create_buffer` or `create_texture` on a device. (Pipelines, bind groups and
//! shader modules are small and not counted.) The bytes of a texture are an estimate from its format, size, mip levels and
//! sample count; the real amount depends on the driver, but estimates are what budgets are made of.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

/// What a resource is for. Each category has its own budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// Vertex and index buffers of the tessellated shapes.
    Geometry,
    /// The shape table (one row per draw item).
    ShapeTable,
    /// Gradient color ramps.
    Ramps,
    /// Offscreen layers for opacity, masks and filters.
    Layers,
    /// The views' own render targets (multisampled color and depth, and the resolved picture).
    Targets,
    /// Glyph atlases.
    Glyphs,
    /// Small things: uniform buffers, staging, test resources.
    Other,
}

impl Category {
    pub const ALL: [Category; 7] =
        [Category::Geometry, Category::ShapeTable, Category::Ramps, Category::Layers, Category::Targets, Category::Glyphs, Category::Other];

    fn index(self) -> usize {
        Category::ALL.iter().position(|&c| c == self).expect("every category is in ALL")
    }
}

/// The most bytes each category may hold at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets([u64; Category::ALL.len()]);

const MB: u64 = 1 << 20;

impl Budgets {
    /// Budgets for a desktop with a discrete card (the "High" quality profile of the plan, section 5.8; to be tuned by
    /// measurement). The real map of the stress test (21,000 shapes, all in view) needs about 150 MB of geometry, and a
    /// change of zoom needs room for the new set while the old one is replaced, so the budget is a good deal larger; two
    /// 4K views need about 600 MB of targets.
    pub fn high() -> Budgets {
        Budgets::of(&[
            (Category::Geometry, 512 * MB),
            (Category::ShapeTable, 32 * MB),
            (Category::Ramps, 16 * MB),
            (Category::Layers, 64 * MB),
            (Category::Targets, 768 * MB),
            (Category::Glyphs, 64 * MB),
            (Category::Other, 128 * MB),
        ])
    }

    /// The same number of bytes for every category (for tests, and for devices that must stay small).
    pub fn uniform(bytes: u64) -> Budgets {
        Budgets([bytes; Category::ALL.len()])
    }

    fn of(entries: &[(Category, u64)]) -> Budgets {
        let mut budgets = [0; Category::ALL.len()];
        for &(category, bytes) in entries {
            budgets[category.index()] = bytes;
        }
        Budgets(budgets)
    }

    pub fn get(&self, category: Category) -> u64 {
        self.0[category.index()]
    }

    pub fn with(mut self, category: Category, bytes: u64) -> Budgets {
        self.0[category.index()] = bytes;
        self
    }
}

/// Why a resource was not created.
#[derive(Clone, Debug, PartialEq)]
pub enum AllocError {
    /// The request is bigger than the device allows.
    TooLarge { what: &'static str, requested: u64, limit: u64 },
    /// The category would go over its budget. Nothing was asked of the card.
    OverBudget { category: Category, requested: u64, used: u64, budget: u64 },
    /// The card said it is out of memory.
    OutOfMemory,
    /// The card or the driver refused the request (an invalid description, or an internal error).
    Rejected(String),
}

impl fmt::Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocError::TooLarge { what, requested, limit } => write!(f, "{what} of {requested} is over the device limit of {limit}"),
            AllocError::OverBudget { category, requested, used, budget } => {
                write!(f, "{category:?} memory: {requested} bytes more would pass the budget of {budget} (already using {used})")
            }
            AllocError::OutOfMemory => write!(f, "the graphics card is out of memory"),
            AllocError::Rejected(why) => write!(f, "the graphics card refused the request: {why}"),
        }
    }
}

impl std::error::Error for AllocError {}

struct State {
    used: [u64; Category::ALL.len()],
    peak: [u64; Category::ALL.len()],
    budgets: Budgets,
    live: usize,
}

struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the allocation counters are never poisoned")
    }

    fn release(&self, category: Category, bytes: u64) {
        let mut state = self.lock();
        let i = category.index();
        state.used[i] = state.used[i].saturating_sub(bytes);
        state.live -= 1;
    }
}

/// A buffer or texture whose bytes are counted. Dereferences to the resource; dropping it gives the bytes back.
pub struct Tracked<T> {
    resource: T,
    category: Category,
    bytes: u64,
    shared: Arc<Shared>,
}

impl<T> Tracked<T> {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn category(&self) -> Category {
        self.category
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.resource
    }
}

impl<T> Drop for Tracked<T> {
    fn drop(&mut self) {
        self.shared.release(self.category, self.bytes);
    }
}

/// The allocator of one device. Clones share the same counters.
#[derive(Clone)]
pub struct Alloc {
    device: wgpu::Device,
    shared: Arc<Shared>,
}

impl Alloc {
    pub fn new(device: wgpu::Device, budgets: Budgets) -> Alloc {
        let state = State { used: [0; Category::ALL.len()], peak: [0; Category::ALL.len()], budgets, live: 0 };
        Alloc { device, shared: Arc::new(Shared { state: Mutex::new(state) }) }
    }

    /// The device the resources are made on (for things that are not buffers or textures, like command encoders).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn budgets(&self) -> Budgets {
        self.shared.lock().budgets
    }

    pub fn set_budgets(&self, budgets: Budgets) {
        self.shared.lock().budgets = budgets;
    }

    /// Bytes alive in `category` now.
    pub fn used(&self, category: Category) -> u64 {
        self.shared.lock().used[category.index()]
    }

    /// The most bytes `category` ever held at once.
    pub fn peak(&self, category: Category) -> u64 {
        self.shared.lock().peak[category.index()]
    }

    /// Forgets the peaks: each category's peak becomes what it holds now.
    pub fn reset_peaks(&self) {
        let mut state = self.shared.lock();
        state.peak = state.used;
    }

    /// Bytes alive in all categories.
    pub fn total_used(&self) -> u64 {
        self.shared.lock().used.iter().sum()
    }

    /// How many buffers and textures are alive.
    pub fn live_resources(&self) -> usize {
        self.shared.lock().live
    }

    /// The bytes of every category, for comparing before and after (leak tests) and for the benchmark report.
    pub fn snapshot(&self) -> Vec<(Category, u64)> {
        let state = self.shared.lock();
        Category::ALL.iter().map(|&c| (c, state.used[c.index()])).collect()
    }

    pub fn create_buffer(&self, category: Category, desc: &wgpu::BufferDescriptor) -> Result<Tracked<wgpu::Buffer>, AllocError> {
        let limit = self.device.limits().max_buffer_size;
        if desc.size > limit {
            return Err(AllocError::TooLarge { what: "buffer", requested: desc.size, limit });
        }
        self.guarded(category, desc.size, |device| device.create_buffer(desc))
    }

    pub fn create_texture(&self, category: Category, desc: &wgpu::TextureDescriptor) -> Result<Tracked<wgpu::Texture>, AllocError> {
        let limits = self.device.limits();
        let side = desc.size.width.max(desc.size.height);
        if side > limits.max_texture_dimension_2d {
            return Err(AllocError::TooLarge { what: "texture side", requested: side as u64, limit: limits.max_texture_dimension_2d as u64 });
        }
        if desc.size.depth_or_array_layers > limits.max_texture_array_layers {
            return Err(AllocError::TooLarge {
                what: "texture layers",
                requested: desc.size.depth_or_array_layers as u64,
                limit: limits.max_texture_array_layers as u64,
            });
        }
        self.guarded(category, texture_bytes(desc), |device| device.create_texture(desc))
    }

    /// Reserves the bytes, asks the card inside error scopes, and gives the bytes back if the card said no.
    fn guarded<T>(&self, category: Category, bytes: u64, create: impl FnOnce(&wgpu::Device) -> T) -> Result<Tracked<T>, AllocError> {
        self.reserve(category, bytes)?;
        let scopes = ErrorScopes::push(&self.device);
        let resource = create(&self.device);
        match scopes.pop() {
            None => Ok(Tracked { resource, category, bytes, shared: self.shared.clone() }),
            Some(error) => {
                self.shared.release(category, bytes);
                Err(error)
            }
        }
    }

    fn reserve(&self, category: Category, bytes: u64) -> Result<(), AllocError> {
        let mut state = self.shared.lock();
        let i = category.index();
        let budget = state.budgets.get(category);
        if state.used[i].saturating_add(bytes) > budget {
            return Err(AllocError::OverBudget { category, requested: bytes, used: state.used[i], budget });
        }
        state.used[i] += bytes;
        state.peak[i] = state.peak[i].max(state.used[i]);
        state.live += 1;
        Ok(())
    }
}

/// An out-of-memory scope and a validation scope around one creation call.
struct ErrorScopes {
    out_of_memory: wgpu::ErrorScopeGuard,
    validation: wgpu::ErrorScopeGuard,
}

impl ErrorScopes {
    fn push(device: &wgpu::Device) -> ErrorScopes {
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        ErrorScopes { out_of_memory, validation }
    }

    /// The error the card reported inside the scopes, if any.
    fn pop(self) -> Option<AllocError> {
        let validation = wait(self.validation.pop());
        let out_of_memory = wait(self.out_of_memory.pop());
        out_of_memory.map(|e| classify(&e)).or(validation.map(|e| classify(&e)))
    }
}

/// Turns what wgpu reported into what the caller can act on.
fn classify(error: &wgpu::Error) -> AllocError {
    match error {
        wgpu::Error::OutOfMemory { .. } => AllocError::OutOfMemory,
        other => AllocError::Rejected(other.to_string()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn wait<T>(future: impl std::future::Future<Output = T>) -> T {
    pollster::block_on(future)
}

/// A browser cannot wait for a future inside a synchronous call; there the error scopes are skipped and errors reach the
/// uncaptured-error handler (`errors.rs`) instead.
#[cfg(target_arch = "wasm32")]
fn wait<T>(_future: impl std::future::Future<Output = Option<T>>) -> Option<T> {
    None
}

/// Bytes a texture takes, estimated: every mip level, every layer, every sample.
pub fn texture_bytes(desc: &wgpu::TextureDescriptor) -> u64 {
    let per_texel = bytes_per_texel(desc.format);
    let mut total = 0u64;
    for level in 0..desc.mip_level_count.max(1) {
        let width = (desc.size.width >> level).max(1) as u64;
        let height = (desc.size.height >> level).max(1) as u64;
        total += width * height * desc.size.depth_or_array_layers.max(1) as u64 * per_texel;
    }
    total * desc.sample_count.max(1) as u64
}

fn bytes_per_texel(format: wgpu::TextureFormat) -> u64 {
    use wgpu::TextureFormat as F;
    match format {
        F::Depth16Unorm => 2,
        F::Depth24Plus | F::Depth24PlusStencil8 | F::Depth32Float => 4,
        F::Depth32FloatStencil8 => 8,
        F::Stencil8 => 1,
        other => other.block_copy_size(None).unwrap_or(4) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Gpu;

    fn buffer(size: u64) -> wgpu::BufferDescriptor<'static> {
        wgpu::BufferDescriptor { label: Some("test"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX, mapped_at_creation: false }
    }

    fn texture(side: u32, format: wgpu::TextureFormat, samples: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("test"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }
    }

    #[test]
    fn texture_sizes_follow_the_plans_numbers() {
        use wgpu::TextureFormat as F;
        assert_eq!(texture_bytes(&texture(1000, F::Rgba8Unorm, 1)), 4_000_000);
        assert_eq!(texture_bytes(&texture(1000, F::Rgba8Unorm, 4)), 16_000_000, "four samples per pixel");
        assert_eq!(texture_bytes(&texture(1000, F::Depth24PlusStencil8, 4)), 16_000_000);
        assert_eq!(texture_bytes(&texture(16, F::Rgba32Float, 1)), 16 * 16 * 16);
        let mut mipped = texture(4, F::Rgba8Unorm, 1);
        mipped.mip_level_count = 3;
        assert_eq!(texture_bytes(&mipped), (16 + 4 + 1) * 4, "every mip level counts");
        // The plan (section 5.8) says: at 1080p the MSAA color, the MSAA depth-stencil and the resolve are about 75 MB.
        let size = wgpu::Extent3d { width: 1920, height: 1080, depth_or_array_layers: 1 };
        let at_1080p = |format, samples| texture_bytes(&wgpu::TextureDescriptor { size, ..texture(1, format, samples) });
        let total = at_1080p(F::Rgba8Unorm, 4) + at_1080p(F::Depth24PlusStencil8, 4) + at_1080p(F::Rgba8Unorm, 1);
        assert!((70_000_000..80_000_000).contains(&total), "{total}");
    }

    #[test]
    fn bytes_are_counted_while_a_resource_lives_and_returned_when_it_is_dropped() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            let before = alloc.snapshot();
            let first = alloc.create_buffer(Category::Geometry, &buffer(1000)).expect("a small buffer");
            let second = alloc.create_texture(Category::Targets, &texture(64, wgpu::TextureFormat::Rgba8Unorm, 1)).expect("a small texture");
            assert_eq!((first.bytes(), second.bytes()), (1000, 64 * 64 * 4));
            assert_eq!(alloc.used(Category::Geometry), 1000);
            assert_eq!(alloc.used(Category::Targets), 64 * 64 * 4);
            assert_eq!(alloc.total_used(), 1000 + 64 * 64 * 4);
            assert_eq!(alloc.live_resources(), 2);
            drop(first);
            assert_eq!(alloc.used(Category::Geometry), 0);
            assert_eq!(alloc.live_resources(), 1);
            drop(second);
            assert_eq!(alloc.snapshot(), before, "everything came back");
            assert_eq!(alloc.peak(Category::Geometry), 1000, "the peak remembers the most");
        });
    }

    #[test]
    fn a_request_over_the_budget_is_refused_without_asking_the_card_and_leaves_no_trace() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            alloc.set_budgets(Budgets::uniform(10_000).with(Category::Layers, 500));
            let kept = alloc.create_buffer(Category::Geometry, &buffer(6000)).unwrap();
            let refused = alloc.create_buffer(Category::Geometry, &buffer(6000)).err().expect("over the budget");
            assert_eq!(refused, AllocError::OverBudget { category: Category::Geometry, requested: 6000, used: 6000, budget: 10_000 });
            assert_eq!(alloc.used(Category::Geometry), 6000, "the refused request reserved nothing");
            assert_eq!(alloc.live_resources(), 1);
            // Another category has its own budget, which can be smaller.
            assert!(matches!(alloc.create_buffer(Category::Layers, &buffer(501)), Err(AllocError::OverBudget { .. })));
            assert!(alloc.create_buffer(Category::Layers, &buffer(500)).is_ok());
            // The refused one fits once something is freed.
            drop(kept);
            assert!(alloc.create_buffer(Category::Geometry, &buffer(6000)).is_ok());
        });
    }

    #[test]
    fn a_request_over_the_device_limits_is_refused_in_both_modes() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            alloc.set_budgets(Budgets::uniform(u64::MAX / 4));
            let limits = gpu.device.limits();
            let too_wide = limits.max_texture_dimension_2d + 1;
            let refused = alloc.create_texture(Category::Targets, &texture(too_wide, wgpu::TextureFormat::Rgba8Unorm, 1)).err().expect("too wide");
            assert!(matches!(refused, AllocError::TooLarge { what: "texture side", .. }), "{refused}");
            let refused = alloc.create_buffer(Category::Other, &buffer(limits.max_buffer_size + 1)).err().expect("too big");
            assert!(matches!(refused, AllocError::TooLarge { what: "buffer", .. }), "{refused}");
            assert_eq!(alloc.total_used(), 0);
            // In WebGL2-limits mode a 4096 texture is over the limit, natively it is fine.
            let result = alloc.create_texture(Category::Targets, &texture(4096, wgpu::TextureFormat::Rgba8Unorm, 1));
            assert_eq!(result.is_ok(), gpu.mode == crate::device::Mode::Native, "mode {:?}: {:?}", gpu.mode, result.err());
        });
    }

    #[test]
    fn an_invalid_request_comes_back_as_an_error_value_and_gives_its_bytes_back() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            let invalid = wgpu::BufferDescriptor { usage: wgpu::BufferUsages::empty(), ..buffer(256) };
            let refused = alloc.create_buffer(Category::Other, &invalid).err().expect("a buffer without usage flags is invalid");
            assert!(matches!(refused, AllocError::Rejected(_)), "{refused}");
            assert_eq!(alloc.used(Category::Other), 0);
            assert_eq!(alloc.live_resources(), 0);
            // The scope took the error, so the uncaptured-error log has nothing and the test does not fail.
            assert!(gpu.errors.take().is_empty());
        });
    }

    /// GPU_SAFETY.md C1: nothing but this module creates buffers or textures, so that every byte is counted and budgeted.
    /// The one other file that may is `errors.rs`, whose test makes an invalid buffer on purpose.
    #[test]
    fn no_code_outside_the_allocator_creates_buffers_or_textures() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        // Calls on a device. (`alloc.create_buffer(..)` is the allowed way.)
        let forbidden = ["device.create_buffer", "device.create_texture", "create_buffer_init", "create_texture_with_data"];
        let mut offences = Vec::new();
        for dir in ["evolve_gpu/src", "evolve_editor/src", "evolve_player/src"] {
            for entry in std::fs::read_dir(crates.join(dir)).unwrap_or_else(|e| panic!("{dir}: {e}")) {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if !name.ends_with(".rs") || name == "alloc.rs" || name == "errors.rs" {
                    continue;
                }
                for (number, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    if forbidden.iter().any(|f| line.contains(f)) && !line.trim_start().starts_with("//") {
                        offences.push(format!("{dir}/{name}:{}: {}", number + 1, line.trim()));
                    }
                }
            }
        }
        assert!(offences.is_empty(), "create buffers and textures through evolve_gpu::alloc::Alloc (GPU_SAFETY C1):
{}", offences.join("
"));
    }

    #[test]
    fn what_wgpu_reports_is_mapped_to_what_the_caller_can_act_on() {
        let source: Box<dyn std::error::Error + Send + Sync> = "x".into();
        assert_eq!(classify(&wgpu::Error::OutOfMemory { source }), AllocError::OutOfMemory);
        let source: Box<dyn std::error::Error + Send + Sync> = "x".into();
        assert!(matches!(classify(&wgpu::Error::Validation { source, description: "bad".into() }), AllocError::Rejected(_)));
    }
}
