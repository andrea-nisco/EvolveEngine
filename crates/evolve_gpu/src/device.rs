//! Opening a graphics device.

use crate::alloc::{Alloc, Budgets};
use crate::errors::ErrorLog;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, OnceLock};

/// What kind of device to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The best adapter on this machine with its own limits (Vulkan, DX12 or Metal).
    Native,
    /// What a browser's WebGL2 offers: the OpenGL backend with wgpu's WebGL2 default limits (2 048-pixel textures,
    /// 16 KB uniform buffers, no storage buffers, no compute). Code that passes here is not using anything WebGL2
    /// lacks. It cannot show differences between browsers' drivers; the web canary (plan task 2.12) does that.
    WebGl2,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Native, Mode::WebGl2];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Native => "native",
            Mode::WebGl2 => "WebGL2 limits",
        }
    }
}

/// The environment variable that turns "skipped: no GPU" into a failure; see [`Gpu::for_each_test_mode`].
pub const REQUIRE_ENV: &str = "EVOLVE_REQUIRE_GPU";

/// Which devices a run has promised to have.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requirement {
    Nothing,
    Native,
    All,
}

#[cfg(not(target_arch = "wasm32"))]
impl Requirement {
    fn parse(value: Option<&str>) -> Requirement {
        match value.map(str::trim) {
            Some("1" | "native") => Requirement::Native,
            Some("all") => Requirement::All,
            _ => Requirement::Nothing,
        }
    }

    fn needs(self, mode: Mode) -> bool {
        match self {
            Requirement::Nothing => false,
            Requirement::Native => mode == Mode::Native,
            Requirement::All => true,
        }
    }
}

/// A device and its queue, with a description of the adapter they came from, the allocator that every buffer and texture
/// must come from (GPU_SAFETY.md C1) and the log of errors the card reported (D1). Cloning shares the device.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub mode: Mode,
    pub alloc: Alloc,
    pub errors: ErrorLog,
}

impl Gpu {
    /// Opens a native device without a window. Returns `None` when this machine has no usable adapter, so that tests
    /// and tools can skip their GPU part instead of failing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Option<Gpu> {
        Gpu::headless_in(Mode::Native)
    }

    /// Opens a device of the given kind without a window, or `None` when this machine cannot provide one.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless_in(mode: Mode) -> Option<Gpu> {
        let backends = match mode {
            Mode::Native => wgpu::Backends::all() & !wgpu::Backends::GL,
            Mode::WebGl2 => wgpu::Backends::GL,
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let info = adapter.get_info();
        let descriptor = match mode {
            Mode::Native => wgpu::DeviceDescriptor::default(),
            // Strictly the defaults, not the adapter's larger limits, so that a violation shows up in a test.
            Mode::WebGl2 => wgpu::DeviceDescriptor { required_limits: wgpu::Limits::downlevel_webgl2_defaults(), ..Default::default() },
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).ok()?;
        Some(Gpu::from_parts(device, queue, info, mode))
    }

    /// Waits until the card has finished everything submitted so far. For tests and tools, not for a frame loop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Wraps a device that already exists (the editor uses the one eframe made) in the allocator and the error log.
    /// This replaces the device's uncaptured-error handler and device-lost callback.
    pub fn from_parts(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo, mode: Mode) -> Gpu {
        let errors = ErrorLog::install(&device);
        let alloc = Alloc::new(device.clone(), Budgets::high());
        Gpu { device, queue, info, mode, alloc, errors }
    }

    /// Runs a GPU test once for every kind of device this machine can open (native, then WebGL2 limits), and says
    /// on the test's output which ones ran and which were skipped. A machine with no GPU runs nothing and the test
    /// passes ("skipped"), so tests never fail only because of missing hardware.
    ///
    /// The exception is a place that has promised a GPU: when the environment variable `EVOLVE_REQUIRE_GPU` is `native`
    /// (or `1`) a missing native device fails the test, and `all` also requires the WebGL2-limits device. The CI job
    /// that installs a software driver sets it, so that at least one place really runs these tests.
    ///
    /// GPU tests run **one at a time on one shared device per mode**: many devices opened at once from many threads made
    /// the OpenGL backend hang, and a device takes far longer to open than most tests take to run. Before each test the
    /// budgets are back to their defaults; after it, the test must have given back every buffer and texture it made (a
    /// free leak check) and the card must not have reported an error.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_each_test_mode(mut test: impl FnMut(&Gpu)) {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        static DEVICES: OnceLock<Mutex<HashMap<Mode, Option<Gpu>>>> = OnceLock::new();
        // A test that failed while holding the lock must not stop the others.
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let required = Requirement::parse(std::env::var(REQUIRE_ENV).ok().as_deref());
        for mode in Mode::ALL {
            let gpu = DEVICES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).entry(mode).or_insert_with(|| Gpu::headless_in(mode)).clone();
            match gpu {
                Some(gpu) => {
                    println!("GPU test on: {gpu}");
                    gpu.alloc.set_budgets(Budgets::high());
                    gpu.alloc.reset_peaks();
                    gpu.errors.take();
                    let before = gpu.alloc.snapshot();
                    test(&gpu);
                    // Errors the card reported while the test ran (they would have panicked inside wgpu) fail it here.
                    gpu.errors.assert_clean();
                    gpu.alloc.set_budgets(Budgets::high());
                    assert_eq!(gpu.alloc.snapshot(), before, "the test left graphics memory allocated (a leak)");
                }
                None if required.needs(mode) => {
                    panic!("no {} device on this machine, and {REQUIRE_ENV} says there must be one", mode.name())
                }
                None => println!("skipped: no {} device on this machine", mode.name()),
            }
        }
    }
}

impl fmt::Display for Gpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({:?}, {:?}, {})", self.info.name, self.info.backend, self.info.device_type, self.mode.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_webgl2_mode_really_has_webgl2_limits() {
        Gpu::for_each_test_mode(|gpu| {
            let limits = gpu.device.limits();
            match gpu.mode {
                Mode::WebGl2 => {
                    assert_eq!(limits.max_texture_dimension_2d, 2048, "WebGL2 textures are at most 2048 pixels");
                    assert_eq!(limits.max_uniform_buffer_binding_size, 16384, "WebGL2 uniform buffers are 16 KB");
                    assert_eq!(limits.max_storage_buffers_per_shader_stage, 0, "WebGL2 has no storage buffers");
                }
                Mode::Native => assert!(limits.max_texture_dimension_2d >= 8192, "a native device is not held to WebGL2 limits"),
            }
        });
    }

    #[test]
    fn a_promised_device_that_is_missing_is_a_failure_and_an_unpromised_one_is_a_skip() {
        assert_eq!(Requirement::parse(None), Requirement::Nothing);
        assert_eq!(Requirement::parse(Some("")), Requirement::Nothing);
        assert_eq!(Requirement::parse(Some("0")), Requirement::Nothing);
        assert_eq!(Requirement::parse(Some("1")), Requirement::Native);
        assert_eq!(Requirement::parse(Some(" native ")), Requirement::Native);
        assert_eq!(Requirement::parse(Some("all")), Requirement::All);
        for mode in Mode::ALL {
            assert!(!Requirement::Nothing.needs(mode));
            assert!(Requirement::All.needs(mode));
        }
        assert!(Requirement::Native.needs(Mode::Native));
        assert!(!Requirement::Native.needs(Mode::WebGl2), "WebGL2 limits need a GL adapter, which a runner may lack");
    }
}
