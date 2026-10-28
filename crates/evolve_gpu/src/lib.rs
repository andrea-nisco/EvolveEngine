//! GPU renderer for EvolveEngine scenes, built on wgpu. The plan, phase by phase, is in `GPU_RENDERER_PLAN.md`; the rules every
//! part follows are in `GPU_SAFETY.md`.
//!
//! [`renderer::Renderer`] draws a scene into a texture: curves become triangles (made once per zoom level and kept, see
//! [`tess_cache`] and [`tessellate`]), [`draw_list`] decides what is drawn and in which order, [`layers`] and [`targets`] handle
//! groups with opacity and clip groups, and a frame that changed nothing is not drawn again. [`profile`] holds the quality profiles,
//! and `egui_glue` (feature `egui`) shows the pictures inside egui. The CPU renderer of `evolve_engine` is the reference: the
//! tests draw the same scene with both and compare the pictures ([`compare`]).
//!
//! It also contains the test bed: a device without a window (natively, or limited to what WebGL2 offers), offscreen drawing and
//! pixel readback ([`device`], [`readback`]). The crate builds for the browser too.

pub mod alloc;
pub mod compare;
pub mod cull;
pub mod device;
pub mod errors;
pub mod fast_map;
pub mod draw_list;
pub mod layers;
#[cfg(feature = "egui")]
pub mod egui_glue;
pub mod pipeline;
pub mod profile;
pub mod ramps;
pub mod ranges;
pub mod tess_cache;
pub mod tessellate;
pub mod readback;
pub mod renderer;
pub mod store;
pub mod targets;
pub mod table;
pub mod vertex;
#[cfg(test)]
mod test_draw;
#[cfg(all(test, feature = "egui"))]
mod egui_glue_tests;
#[cfg(test)]
mod picture_tests;
#[cfg(test)]
mod clip_tests;
#[cfg(test)]
mod layer_tests;
#[cfg(test)]
mod instance_tests;
#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod renderer_tests;
#[cfg(test)]
mod stroke_width_tests;

pub use device::{Gpu, Mode};

/// How much memory the program needs, in the browser, to hold `shapes` shapes with their triangles (measured on the stress scene:
/// 21,200 shapes need between 320 and 448 MB of WebAssembly memory, about 20 KB each; the pictures themselves are far smaller, 1.8 KB
/// a shape, the rest is the tessellator's working space and the allocator's slack). Clamped to 1 GB, as WebAssembly memory is 4 GB at
/// most and a scene that large needs another way anyway.
pub fn memory_for_shapes(shapes: usize) -> usize {
    shapes.saturating_mul(24_000).min(1 << 30)
}

/// For the browser: makes the program's memory as large as `bytes` in one step. WebAssembly memory grows only when asked to, and the
/// allocator asks for a little more every time it runs out; in a browser every one of those growths costs more than the one before
/// (the stress scene's first frame took 1.3 s, of which the work itself is 0.1 s, and a second run in the same page took 0.07 s,
/// because the memory was there by then). Growing once, to what the scene will need (`memory_for_shapes`), is the same work done in
/// one step: 1.28 s becomes 0.16 s for the frame plus 0.1 s for the growth. The memory stays with the program for its own use. One
/// byte in every page is written, so the browser has made the memory real. Does nothing off the web.
pub fn reserve_memory(bytes: usize) {
    if cfg!(target_arch = "wasm32") {
        // One byte in every page is written, so that the browser has made the memory real, not only promised (`black_box`: a block that
        // is made and dropped unused is code the compiler removes, and then nothing grows).
        let mut block = vec![0u8; bytes];
        for at in (0..bytes).step_by(4096) {
            block[at] = 1;
        }
        drop(std::hint::black_box(block));
    }
}
