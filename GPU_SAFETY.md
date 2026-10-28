# GPU safety rules

*Rules for everyone who writes GPU code in EvolveEngine, people and agents alike. They apply to the editor, the player,
tests, benchmarks and every game built with the engine. A change that breaks one of these rules is not merged. If a
rule blocks something legitimate, change the rule here first and say why in the plan's decision log.*

## 1. What can actually go wrong

A modern graphics card protects itself. Its own firmware limits temperature, power and voltage: when it gets too hot it
slows down, and if that is not enough it shuts off. An application using wgpu (Vulkan, DX12, Metal, OpenGL, WebGPU)
**cannot** change clocks, voltages, power limits or fan speeds; that needs vendor tools with special privileges, and
this project never uses them (rule E1). Running a card at 100% for hours is what games do every day.

What our code *can* cause, in order of how likely it is:

| what | how it shows | harm |
|---|---|---|
| A GPU job that runs too long | Screen freezes or goes black for a few seconds, then comes back ("driver reset", TDR on Windows); our app loses its device | None to hardware; our app must survive it (rule D3) |
| Out of video memory | Allocation errors, stutter, a crash | None to hardware |
| Uncapped frame rate on a simple scene (a menu, an empty view) | Thousands of frames per second, fans loud, coil whine, high power draw, heat | A healthy card throttles. A card with a manufacturing defect or blocked cooling can fail under this load, as happened with some cards in a 2021 game beta whose menus were uncapped. This is the one case worth real care, and rules A1 to A5 exist for it |
| A laptop or phone running hot | Throttling, battery drain, discomfort | None, but bad for users (rule A6) |

So the rules have two aims: **never ask the card for pointless work** (frame pacing), and **never give it unbounded
work** (limits, so a bug or a bad project file cannot hang the driver).

## 2. The rules

### A. Frame pacing: never draw more frames than can be seen
- **A1. Present mode.** Windows present with `Fifo` (wgpu `AutoVsync`). `Mailbox` and `Immediate` are not frame caps:
  with them the card still draws as fast as it can. They are allowed only behind an explicit setting that also turns on
  the CPU-side frame limiter of A2.
- **A2. Frame limiter.** The player has a frame limiter that is always on, even when vsync is off or ignored by the
  driver: default target = the display refresh rate, capped at 60 fps unless the end user raises it; the allowed range
  is 15 to 240 fps. Menus, loading screens, pause screens and empty scenes are limited like everything else.
- **A3. Editor draws only on change.** The editor stays in egui's reactive mode: it repaints on input or when something
  changed. A GPU view redraws only when the scene revision, the camera or the view size changed (plan task 2.10).
  Only a running Game view or playing animation asks for continuous frames, and then A1 applies.
- **A4. Hidden means zero frames.** A minimized window, a hidden tab or panel, or a view with zero size draws nothing
  and submits nothing.
- **A5. Offscreen work is bounded by count, not by time.** Headless rendering (tests, `--benchmark`, thumbnails, PNG
  export) has no vsync, so it must always have a fixed frame count and stop. `--benchmark` runs a fixed number of frames
  per view and exits; no mode renders offscreen in an endless loop.
- **A6. Mobile and laptops.** Exported games choose 30 or 60 fps (plan 11.5); on battery the default is the lower one.

### B. Bounded work: no single job can hang the card
- **B1. No unbounded shader loops.** Every loop in WGSL has a constant bound, or a bound clamped on the CPU before upload
  (blur kernel size, gradient stops, filter steps). A loop whose count comes straight from data is a bug.
- **B2. Project files are untrusted input.** Every number from a project, an imported SVG or a script that controls GPU
  work is clamped when it is read and again before it reaches the GPU code. The table below lists the limits; new
  features add their rows here.

  | value | limit (proposal; tune with measurements) |
  |---|---|
  | Texture or layer size | ≤ the device's `max_texture_dimension_2d`. A layer is its node's box clipped to the view, so never larger than the view (done, `draw_list.rs`); one that is larger than the device allows or does not fit the `Layers` budget is drawn at 1/2, then 1/4 of its size and shown stretched (done, `FrameStats.layers_reduced`); beyond that the view uses the CPU renderer |
  | Layers in one frame | ≤ 2 048 (`MAX_LAYERS`: each is a render pass); above it the view uses the CPU renderer (done) |
  | Nesting of layers (opacity, masks, filters) | ≤ 32; deeper levels are drawn without a new layer, the opacity going down into what is inside, and the frame counts them (done, `MAX_LAYER_DEPTH`, `FrameStats.layers_flattened`) |
  | Clip nesting | ≤ 250 of the stencil's 255 levels; deeper clips are not made and the frame counts them (done, `MAX_CLIP_LEVEL`, `FrameStats.clips_flattened`) |
  | Symbols inside symbols | ≤ 12 levels: a symbol that contains itself (a damaged file) stops there instead of going on for ever (done, `MAX_INSTANCE_DEPTH`) |
  | View uniforms | one 256-byte slot for every pass of a frame, so at most 2 049 slots (about half a megabyte), chosen with a dynamic offset (done) |
  | Blur radius | clamped to a maximum per quality profile; large radii use the downsampled path |
  | Gradient stops | ≤ 256 used; more are ignored (done, `ramps.rs`) |
  | Gradient ramps | up to the device's texture size (2 048 different gradients on WebGL2) and the budget of the Ramps category; then the view uses the CPU renderer (done) |
  | Instances per draw call | ≤ the device's limits; split into several calls above |
  | Vertices per shape after tessellation | capped (proposal: 4 million); above it the shape is drawn at a coarser tolerance, with a warning |
  | Text size on screen | clamped so a single glyph cannot need a huge texture |
  | Flattening tolerance (the zoom bucket) | 1e-4 to 100 units of the shape's own space; zoom buckets 2^-16 to 2^20 (done, `tessellate.rs`) |
  | Miter limit | 1 to 100; not a number gives 4 (done) |
  | Coordinates of a path | a shape with a coordinate that is not a finite number is not drawn, with a warning (done) |
  | Draw items per frame | at most 8.3 million, what the 24-bit depth buffer can tell apart; above it the view uses the CPU renderer (done) |
  | Shape table rows | up to the device's texture size (2 048 lines of 128 rows on WebGL2) and the budget; then the view uses the CPU renderer (done) |
  | View size | the device's texture size, and the `Targets` budget; slack never goes past the device limit (done) |
  | Memory per category | `Budgets::high()`: geometry 512 MB (raised from 192 on 2026-10-16: the real map needs about 150 MB and a change of zoom needs room beside it; open point P11), shape table 32 MB, ramps 16 MB, layers 64 MB, targets 768 MB, glyphs 64 MB, other 128 MB (done; to be tuned by profile, plan 5.8) |

- **B3. Bounded work per frame.** Per frame there is a budget for re-tessellation, uploads and layer passes (plan 5.4).
  Work above the budget waits for the next frame instead of making one giant frame.
- **B4. Watchdog in debug builds.** If the GPU time of a frame (timestamp queries where available) or the time to get a
  frame back exceeds 100 ms, log a warning naming the scene and the view. It catches runaway work early.

### C. Memory: stay inside a known budget
- **C1. One tracker.** Every buffer and texture is created through `evolve_gpu`'s allocator, which counts bytes per
  category (geometry, shape table, ramps, layers, targets, glyphs). No `create_buffer` or `create_texture` elsewhere.
- **C2. Budgets are enforced.** Each category has a budget from the quality profile (plan 5.8). Above it the renderer
  degrades (smaller layers, coarser tolerance, CPU fallback for the view) and warns; it never keeps allocating.
- **C3. Check limits before asking.** Sizes are checked against `device.limits()` before a create call. Creation that may
  fail is wrapped in an `OutOfMemory` error scope and handled.
- **C4. No leaks.** A test renders the same scenes for many frames, resizes the view back and forth, and checks that the
  tracker's totals return to the same values.

### D. Errors and recovery: the app survives, the desktop is untouched
- **D1. No panics from GPU errors in release builds.** An uncaptured-error handler logs the error; the view switches to
  the CPU renderer and shows a short message. In debug builds wgpu validation stays on and errors fail tests.
- **D2. Validation stays on in development.** wgpu's validation layer is never disabled to "make something work".
- **D3. Device lost is survivable.** From Phase 2: when the device is lost (driver reset, sleep, WebGL context loss),
  the views fall back to the CPU renderer instead of crashing. Full recovery (rebuilding GPU resources) is plan task 7.4.
- **D4. Surface errors.** `Outdated` and `Lost` surfaces are reconfigured; `Timeout` skips the frame; none of them
  panics.

### E. Things the code never does
- **E1.** Never use APIs or crates that read or change clocks, voltages, power limits, fan curves or other hardware
  settings (NVAPI, ADL/ADLX, NVML writes, overclocking libraries). Reading temperatures for a diagnostic panel is
  allowed only through read-only APIs, and only with the owner's approval.
- **E2.** No `unsafe` access to wgpu-hal or the native API, no unchecked shader modules, and no experimental wgpu
  features without an entry in the plan's decision log approved by the owner.
- **E3.** No GPU stress tests or long benchmark runs on the owner's machine without asking first. GPU tests stay small:
  small targets, few frames.

### F. Development and testing practice
- **F1.** Every test process that touches the GPU has a timeout (proposal: 60 s per test binary in CI).
- **F2.** `--benchmark` prints its total running time and refuses to run longer than a fixed limit (proposal: 5 minutes)
  unless asked with an explicit flag.
- **F3.** Agents do not leave the editor, the player or a benchmark running after a task. Every GPU process an agent
  starts is ended before it reports.
- **F4.** Each rule above that can be tested has a test (frame limiter, zero frames when hidden, clamps, budgets, leak
  test, device-lost fallback), and each such test gets a mutation check as the plan describes.

### G. Games built with the engine
- **G1.** Exported games ship with rules A1, A2, A4 and A6 on. A game's scripts can choose a target frame rate inside the
  allowed range; they cannot switch the limiter off. Only the end user can, in the game's settings, and the game then
  shows the frame rate.
- **G2.** The clamps of B2 and the budgets of C2 apply to scripts too: `instantiate` in a loop, a huge blur or deep
  nesting degrade the picture with a warning in the editor console; they do not reach the card unbounded.
- **G3.** Exported games expose the quality profile and the CPU fallback (plan 8.7).

## 3. Where this lands in the plan

Phase 2 gets these tasks, before the first GPU view is shown in the editor:
- 2.15 Present mode and frame pacing (A1, A3, A4) for the editor views; test that a hidden or zero-size view submits
  nothing.
- 2.16 Allocator with byte tracking and budgets (C1 to C3); leak test (C4).
- 2.17 Error handling: uncaptured-error handler, CPU fallback on device lost (D1, D3, D4).
- 2.18 Clamp table (B2) for everything Phase 2 draws; the table grows with each later phase.
- 2.19 Debug watchdog (B4) and benchmark time limit (F2); test timeouts in CI (F1).

Phase 8 (player) gets the frame limiter (A2), its settings and G1. Phase 9 and 10 add their rows to the clamp table.

## 4. What the owner can do

Hardware health depends mostly on the card's cooling, not on our code. Once in a while, look at the GPU temperature and
fan speed while the editor runs a game view (with the vendor's tool, GPU-Z or HWiNFO). If temperatures are high with
light work, clean the dust from the fans and heatsink. Keep the graphics driver up to date.
