# GPU renderer plan

*Written for the project owner and future contributors. This file is the single source of truth for the GPU renderer:
tick the boxes as work is finished, write results into the phase's "Result" line, and add dated entries to the logs at
the end. If the plan changes, change it here and say why in the decision log.*

**Scope rule (D9).** This plan is about the **engine and editor**: giving EvolveEngine a GPU renderer that makes any
project fast. IronKrieg is used only as a *test scene*, because it is large and real. Features that exist only
because the IronKrieg game needs them (shield composition, soldiers, fog of war, map rules) belong to the game's own
plan, not here. A feature is in this plan only if it can be explained without mentioning IronKrieg.

**Reference rule (D6).** The CPU renderer is the reference. Every visual feature exists in the CPU renderer first, or
lands in both renderers in the same task. A GPU feature without a CPU reference cannot be tested and is not accepted.

**Safety rule ([GPU_SAFETY.md](GPU_SAFETY.md)).** Every task that touches the GPU follows that file: frame pacing (a frame
is never drawn that cannot be seen), bounded work (every value that reaches the card is clamped), a tracked memory
budget, survivable errors, and things the code never does (such as touching clocks or power). It is a reference for
every phase, not only for Phase 2, and a change that breaks one of its rules is not accepted.

**Status of Phase 3 (2026-10-17):** done, except two things that are not the code's to finish: 3.1 waits for the owner to look
for shimmering while zooming thin lines (MANUAL_TESTS.md, "After Phase 3"), and 3.10 (pixel-width strokes) waits for the answer to
P4 (Q2). Gradients, opacity on a single shape, the miter-limit bevels, zoom-bucket hysteresis and the whole-project regression
suite are in and tested on T1 in both device modes; see the progress log of 2026-10-17.

**Status of Phase 2 (2026-10-16):** tasks 2.1 to 2.11, 2.15 to 2.19 are done and tested; the GPU renderer can be tried in the
editor (`--renderer gpu`, or the *Renderer* box in the status bar, or *View → Renderer*) on scenes of solid fills and strokes,
such as the IronKrieg map. The owner's first hands-on try (2026-10-16) found the editor slow on the real map; the causes were
found with timings of the real editor frame and fixed (see the progress log of that date): a quadratic loop in the editor's
overlays and in play mode, the geometry going over its budget at a change of zoom (and then silently using the CPU renderer),
and nothing being culled. **Second try done (2026-10-16): "100 times smoother", a little lag remains now and then; its cause is not known yet (suspects: P12, the undo snapshot at mouse release, autosave).** Open: 2.12 (web canary), 2.13 (benchmark on T2), 2.14
(checkpoint), which need the T2 laptop (P2).

**Status of Phase 0:** nearly done: 0.1 to 0.6 and 0.8, 0.9, 0.11, 0.12 finished (2026-10-15); 0.7 (needs one CI run, the
job is prepared) and 0.10 (needs the owner to name the T2 laptop) are parked as open points P1 and P2 in
[OPEN_POINTS.md](OPEN_POINTS.md) and do not block Phase 1. Backend decided: wgpu (2026-10-15).
Plan revised 2026-10-15 after review (see decision log; old to new phase numbers in appendix A).

- [ ] Phase 0: GPU test bed
- [x] Phase 1: Change tracking (2026-10-15)
- [ ] Phase 2: Minimal GPU renderer
- [ ] Phase 3: Shape parity
- [ ] Phase 4: Groups and effect layers
- [ ] Phase 5: Speed at scale, on every hardware tier
- [ ] Phase 6: Dynamic content
- [ ] Phase 7: Editor integration (GPU becomes the default)
- [ ] Phase 8: Player, web and desktop platforms
- [ ] Phase 9: Advanced vector features (dashes, blend modes, masks, filters, patterns, images)
- [ ] Phase 10: Text
- [ ] Phase 11: Mobile
- [ ] Phase 12: Beyond the baseline (open-ended)

---

## 1. Goal and "done"

**Goal:** the editor and the player draw large vector scenes on the graphics card, with the same picture as the CPU
renderer (within the comparison rule of section 6), on desktop, laptop, web (including WebGL2) and later phones,
falling back to the CPU renderer when no usable GPU exists. Within about a year the GPU renderer also covers the
full set of SVG-style features: dashes, blend modes, masks, filters and text.

**Why:** the CPU renderer (tiny-skia) redraws every visible shape on every repaint. On the imported IronKrieg map
(1 234 filled shapes, 20 206 stroked lines, 679 000 points) the full view takes about 72 ms at 1080p, about 14 fps
(**measured**, BENCHMARKS.md). 85% of that frame is rasterizing, which the CPU cannot do faster and a graphics card is
built for.

### Hardware tiers

The engine must run well on everything, so targets are set per tier. Each tier has one named reference machine
(chosen in task 0.10) on which its numbers are measured.

| tier | what | reference machine |
|---|---|---|
| T1 | desktop with a discrete GPU | owner's machine, NVIDIA RTX 3060 (Vulkan) |
| T2 | laptop with integrated graphics | to choose in 0.10 (proposal: Intel Iris Xe or AMD Radeon 680M class) |
| T3 | web player, WebGL2, on the T2 laptop | same laptop, current Chrome and Firefox; Safari on a Mac |
| T3w | web player, WebGPU, on the T2 laptop | same laptop, browsers with WebGPU |
| T4 | phones (Phase 11) | to choose in Phase 11 (proposal: a mid-range Android from about 2023, an iPhone 12 class) |

### Targets (proposed; confirmed with real numbers at checkpoint 2.14 and again at the end of Phase 5)

| scene | T1 | T2 | T3 / T3w | T4 |
|---|---|---|---|---|
| Illustration scene (typical game content, section 1 scenes), 1080p | 60 fps | 60 fps | 60 fps | 60 fps at native resolution, quality profile Low |
| Stress scene (map), whole view, 1080p | 60 fps (GPU part under 4 ms) | 30 fps | 30 fps | not a target |
| Stress scene, whole view, 4K | 30 fps | not a target | not a target | not a target |
| Animated scene (Phase 6) | 60 fps | 60 fps | 60 fps | 60 fps |
| Small scenes | not slower than the CPU renderer, every tier |
| Editor with the stress scene open | panning, zooming, selecting and editing stay smooth on T1 and T2 |
| Frame where nothing changed | under 0.5 ms of CPU, no uploads (battery on laptops) |

**Done means:** the targets above hold on their tiers; the picture matches the CPU renderer on the regression scenes
under the rule of section 6; every feature the CPU renderer has is on the GPU; the editor and player use the CPU
renderer instead of failing when no usable GPU exists.

### Benchmark scenes

- **Stress scene:** `--gen-scene large` (about 21 000 shapes, 660 000 points; polylines, solid colors). Owned by the repo.
- **Illustration scene (new, 0.9):** `--gen-scene illustration`. The content an SVG-first engine actually draws:
  cubic curves, linear and radial gradients, about 30% semi-transparent shapes, nested groups with opacity and clips,
  symbol instances with color overrides. Owned by the repo.
- **IronKrieg map:** the real-world check, imported.
- **Animated scene (Phase 6):** the sample's tentacle, pulsing sun, rolling ball, plus a generated many-sprites scene.

### Words used in this plan
- **Tessellation:** turning a shape outline (curves and corners) into triangles, the only thing a graphics card draws.
- **Draw call:** one instruction to the card, "draw this block of triangles". Thousands per frame get slow because
  each has CPU overhead; a few dozen are fine.
- **Batching:** gluing many shapes into one block of triangles so they cost one draw call.
- **MSAA:** anti-aliasing by sampling several points per pixel at the edges (here 4). The 4 samples are combined
  ("resolved") into a normal picture at the end.
- **Offscreen texture:** a picture the card draws into instead of the screen, which can then be shown or reused.
- **Stencil:** an extra per-pixel (per-sample with MSAA) counter the card keeps while drawing; used for clipping.
- **Depth value:** a number per shape that the card compares per sample; here used to keep painter's order and to
  stop a shape from blending over itself.
- **Instancing:** drawing the same block of triangles many times with a different position or color each time.
- **Shape table:** a texture used as a table, one row per draw item (transform, paint, opacity, stroke width, depth).
  Vertices carry a row number instead of their own color or transform.
- **Effect layer:** an offscreen texture a group is drawn into, so that something can be done to it as a whole
  (opacity, mask, filter, blend mode) before it is put back into the picture.
- **Hairline:** a stroke thinner than one screen pixel. tiny-skia draws it one pixel wide with reduced opacity.
- **Effective scale:** camera zoom times the shape's own largest scale. Decides how finely curves are tessellated.
- **Tile-based GPU:** the kind of GPU in phones and Apple machines. It draws the screen in small tiles kept in fast
  on-chip memory; splitting a frame into many passes is expensive for it.

## 2. Starting point

**The current renderer** ([render.rs](crates/evolve_engine/src/render.rs)) draws into a bitmap; the editor and player
turn that bitmap into an egui image every frame.

**Feature inventory (task 0.8, made 2026-10-15 by reading `render.rs`, `style.rs`, `scene.rs`, `symbol.rs`, `rig.rs`,
`animation.rs`, `morph.rs`, `game_camera.rs` and searching the whole workspace).** Everything below is what the GPU
must reproduce for the default to be flipped (Phase 7). tiny-skia draws every path with anti-aliasing on.

*Shapes* (`Shape`: path, optional fill, optional stroke, fill rule, optional skin):
- Paths: subpaths of anchors with in and out handles (a segment is a straight line when both handles are zero,
  otherwise a cubic Bézier), open or closed. Subpaths with fewer than 2 anchors are dropped. An open subpath is
  filled as if closed. Compound shapes (several subpaths) are normal.
- Fill: nonzero or even-odd. Fill paint: solid color or gradient.
- Stroke: width in the shape's own units, transformed with the shape (so non-uniform scale gives non-uniform widths);
  a width of 0 or less draws nothing; join miter / round / bevel with a miter limit; cap butt / round / square.
  Stroke paint: solid color or gradient. Thin strokes: tiny-skia's hairline rule (D5c). Fill is drawn first, stroke
  on top, each blended separately (a half-transparent stroke shows the fill under its inner half).
- Colors: 8-bit RGBA, straight alpha in the file, blended as stored (non-linear) with premultiplied alpha (D14).
- Gradients: **linear and radial only**, defined in the shape's local space with their own extra transform; any number
  of stops (offsets clamped to 0..1); **no focal point and no spread modes: always pad**. A gradient with no usable
  stops draws nothing for that paint.
- Skinned shapes: the outline is bent by bones in scene (world) space at draw time, from the bind-pose path.
  Animated paths (morphing) arrive the same way, as a replaced path in a posed copy of the scene.
- Culling: shapes provably outside the target are skipped; the picture does not change (tested).

*Groups and nodes*:
- Groups draw their children in order, last on top. Hidden nodes and nodes with opacity 0 or less draw nothing.
- **Opacity below 1 on any node (group, shape or instance) makes the node a whole**: it is drawn into a layer and the
  layer is composited with that opacity. For a leaf shape with both fill and stroke this means the stroke covers the
  fill's inner half *before* the fade, which a per-item opacity would not reproduce (see task 3.6).
- **Clip groups** (`clip` flag): the top-most child must be a shape and is the mask; the other children are drawn
  and shown through the mask's *fill silhouette* (its fill rule, anti-aliased edge). The mask's own fill and stroke
  colors, opacity, stroke and visibility play no part. If the top child is not a shape the flag is ignored. A clip
  group's own opacity applies on top.
- **Symbol instances**: draw the symbol's master group under the instance's transform and opacity, with an optional
  fill color and stroke color override; an override replaces *every* fill (including gradients) or stroke paint
  inside by a solid color, shapes without fill or stroke stay without; outer instances win over inner ones.
- Camera nodes draw nothing but the frame: `render` first fills the camera's frame (a possibly rotated rectangle)
  with its background color and draws content on top; outside the frame the picture stays transparent.

*Not present in the CPU renderer or in the data model*: dashes, blend modes, masks other than clip groups, filters
(blur, shadows), patterns, raster images, text, focal points, spread modes other than pad, pixel-width strokes. SVG
import ignores images, text, filters, masks, clip paths and dashes (`io.rs`). So the previous plan was right and the
owner's impression of "many more features" does not match the code: the CPU renderer's extra power is the layers,
which are the expensive part.

**Sorting (the result of 0.8).** Nothing is moved into 3.11 or 4.9: both stay empty because the CPU renderer has no
shape-level or group-level feature beyond the ones already in tasks 3.1 to 3.8 and 4.1 to 4.7. Dashes, blend modes,
masks, filters, patterns, images and text remain Phases 9 and 10, each first added to the CPU renderer (reference
rule). Q7 is answered: no focal point, pad only. Two plan tasks were sharpened by the inventory: 3.6 and 4.4
(opacity on a leaf shape with fill and stroke) and 4.1 (exact clip semantics).

**What the CPU renderer costs on typical content (measured 2026-10-15, BENCHMARKS.md).** The illustration scene takes
2.8 s per frame at 1080p, and still 1.9 s when 0.5% of it is visible, because every node with opacity below 1 gets a
full-screen layer (about 8 ms each). Without opacity groups the scene takes 0.35 s. The GPU renderer's layer system
(D13) must therefore make opacity groups cheap by design (shortcuts 4.4, bounds-sized layers).

**What the editor runs on.** The editor build contains both OpenGL (glow) and wgpu; eframe picks wgpu when both are
present, so the editor already runs on wgpu 30 (egui-wgpu 0.36). The player (desktop and web) is built with glow only.

**Tessellation probe (measured 2026-10-15).** A throwaway program tessellated the imported test scene with the lyon
crate (1.0.x), CPU only, release build:

| | time | vertices | triangles | memory |
|---|---|---|---|---|
| 1 234 filled shapes (even-odd) | 43 ms | 422 000 | 419 000 | 8.4 MB |
| 20 206 strokes, round joins and caps, tolerance 2 units | 20 ms | 653 000 | 612 000 | 12.6 MB |
| same, tolerance 0.05 units (very zoomed in) | 33 ms | 1 085 000 | 1 043 000 | 21 MB |

One full tessellation of the scene is under 100 ms and about 25 to 30 MB, and the card draws about 1 to 1.5 million
triangles. This is what makes the approach credible. Note the third row: the scene is all polylines, yet the vertex
count grew by 66% with the tolerance. The extra vertices are the **round joins and caps**, which are curves. So
polylines with round joins also depend on zoom (D3).

**WebGL2 facts that bind the design (D8).** No compute shaders and no storage buffers. Uniform buffers are only
guaranteed 16 KB. wgpu's default WebGL2 limits allow 2D textures of 2048 × 2048 (request the adapter's real limits
when higher). MSAA sample counts are 1 or 4. No timestamp queries (no GPU timing on the web). Wasm in the browser is
single-threaded by default. The WebGL context can be lost at any time. Download size counts.

## 3. Design decisions

**D1. wgpu 30.** Settled. New code lives in the crate `evolve_gpu`, so `evolve_engine` keeps its promise of having no
graphics or UI dependency. The core of `evolve_gpu` renders into any wgpu texture and **does not depend on egui**; the
egui glue is a thin module behind a cargo feature `egui`. This keeps a later non-egui player shell (phones, Phase 11)
possible. The player depends on `evolve_gpu` too.

**D2. Draw into our own offscreen texture, on eframe's device.** egui's own render pass has no stencil and no
multisampling unless they are switched on for the whole UI. So the scene is drawn into textures we control: a 4x MSAA
color target, a 4x MSAA depth-stencil target, and a single-sample resolve texture that egui shows. The renderer uses
**the same wgpu device and queue as eframe** (from eframe's render state); the headless device of 0.2 is for tests
only. The scene pass is recorded in egui-wgpu's paint-callback `prepare` step so it runs in the right place in the
frame. When the target is reallocated, the texture is re-registered with egui.

**D3. Geometry is tessellated on the CPU with lyon and cached in the shape's own coordinates.** Moving, recoloring or
animating the transform of a shape never re-tessellates it. A shape is re-tessellated only when (a) its outline or
stroke geometry changes, known from the change tracking of D7, or (b) its *effective scale* crosses a power-of-two
bucket. (b) applies to every shape that contains curves, **including round joins and caps**. The flattening tolerance
is 0.25 screen pixels divided by the effective scale. Rejected: tessellating on the GPU with compute shaders (WebGL2
has no compute; the probe shows CPU tessellation is cheap).

**D4. Anti-aliasing: 4x MSAA, with attachments that are never written back to memory.** The MSAA color and depth-stencil
attachments use `StoreOp::Discard` after the resolve (and transient / memoryless attachments if the pinned wgpu
exposes them), which makes MSAA cheap on integrated and tile-based GPUs. MSAA has only 5 coverage levels per pixel,
while tiny-skia computes exact coverage; edges will therefore differ a little, and the comparison rule of section 6
accepts that. A higher-quality analytic mode is a Phase 12 option.

**D5. One stroke method; the width is applied in the vertex shader.** lyon's stroke tessellator gives, for each
vertex, the point on the path, a normal, and the distance along the path. Vertices store these, and the vertex shader
computes `point + normal × half_width`. Consequences:
- (a) *World-width strokes* (today's behavior): the offset is applied in the shape's own space, then transformed. Same
  semantics as today, including non-uniform scale.
- (b) *Pixel-width strokes* (optional data-model setting): the offset is applied in screen space. Same code path, a
  flag in the shape row. Needs a project format version bump (Q2).
- (c) *Hairlines*: when the stroke's width on screen is under 1 pixel, the shader draws it 1 pixel wide and multiplies
  its alpha by the real width, as tiny-skia does. Without this the stress scene, where most strokes are thinner than a
  pixel at the whole view, would look dim, broken and shimmering, and would not match the CPU.
- (d) *Dashes* later (Phase 9) can use the stored distance along the path.
Join and cap geometry is built by lyon, so miter, bevel and round joins and the miter limit stay exact.

**D6. The CPU renderer stays.** It is the reference for tests (reference rule at the top), the renderer for thumbnails
and PNG export, and the fallback when no usable GPU exists (remote desktop, very old machines). (SVG export writes the
scene and does not use either renderer.) The editor gets a renderer choice: automatic, CPU, GPU.

**D7. Change tracking with stamps (built in Phase 1, `evolve_engine::revision`).** Every change to a node takes a fresh
*stamp* from one counter shared by the whole process, so a stamp names one version of some content and no two
versions ever share one, even in two copies of a scene that were changed separately. Copies carry the stamps of the
original until they change, and undo restores old stamps with the old content. So **equal stamp means equal content**,
which makes stamps safe cache keys. There are five stamps per node and per scene, one per kind of change: geometry,
transform, paint, structure, data (the table is in `revision.rs`). The scene's own five stamps answer "did anything
of this kind change?" in constant time. A renderer compares per-node stamps against its cache during the walk of the
scene that it does anyway (0.08 ms for 21 000 nodes). The undo check, the GPU cache and the picking index all read
them. Writes are tracked by a guard returned from `Scene::find_mut`; see the audit below for why and its limits.

**D8. WebGL2 is the baseline and a performance target, not only a fallback.** The baseline renderer uses only vertex
and index buffers, uniform buffers under 16 KB, and textures (including float data textures read with `textureLoad`).
No storage buffers, no compute, no timestamp queries, no reliance on threads. Features that need more (GPU culling,
indirect drawing) are optional extras, never required. Every phase runs its GPU tests in a *WebGL2-limits mode*
(0.12) and checks the web canary (2.12).

**D9. Engine, not game.** See the scope rule at the top.

**D10. The shape table.** Every draw item (the fill of a shape, the stroke of a shape, a layer composite, an instance)
has one row in a float data texture (RGBA32Float, 1 024 texels wide, 4 texels per row):
transform (6 floats), depth (1), paint kind and gradient reference (1), premultiplied color (4), stroke width (1),
flags such as width mode (1), opacity (1), spare (1). Vertices carry the row number.
- Moving a shape, recoloring it or changing its opacity rewrites one row; uploads are batched into dirty row ranges.
- A 2 048 × 2 048 table holds about one million rows, inside the WebGL2 limits.
- Uniform buffers are not used for this: 16 KB on WebGL2 cannot hold 20 000 transforms.

**D11. Painter's order with depth values.** Draw items get strictly increasing depth values in document order; the
depth test is "strictly greater" with depth writes on. Two effects:
- A shape cannot blend over itself. lyon strokes overlap themselves at the inner side of joins and where a path
  crosses itself; with a semi-transparent stroke those overlaps would show as darker spots. tiny-skia fills the stroke
  outline as one shape, so the depth test is needed for parity.
- Fully opaque items may be drawn out of order and still come out right (each sample keeps the topmost item). This
  makes instancing of opaque multi-shape symbols safe (4.5) and allows a front-to-back pass to save fill rate (5.6).
Depth and stencil share one `Depth24PlusStencil8` target, available on WebGL2.

**D12. Clipping with the stencil buffer; soft masks with layers.** On a 4x MSAA target the stencil is per sample, so a
stencil clip is anti-aliased like every other edge. Nested clips increment and decrement the stencil value (up to 255
levels). Layers are needed only for soft masks (luminance or alpha masks, Phase 9).

**D13. Effect layers, one mechanism for every group effect.** Opacity groups, masks, filters and blend modes all go
through the same system: a pool of offscreen textures, sized to the group's bounds on screen (plus filter margins), with
a memory cap per quality profile. Layers whose content does not depend on what is behind them (opacity groups, masks,
most filters) are drawn in **prepasses before the main pass**, and the main pass only composites them as textured quads
at their depth. Only effects that read the backdrop (non-trivial blend modes) split the main pass. This keeps the
number of passes low, which matters for WebGL2 and tile-based GPUs. When the pool is full, layers are drawn at reduced
resolution (down to 50%), with a warning; they are never silently skipped, because that gives wrong colors.

**D14. Color: blend in the same space as tiny-skia.** tiny-skia blends the stored (non-linear) sRGB values directly,
with premultiplied alpha. The GPU targets therefore use `Rgba8Unorm`, not `Rgba8UnormSrgb` (an sRGB target would blend
in linear light and every transparent edge and gradient would differ). egui's image shader expects a particular
encoding for the textures it shows; a test (2.7) checks that a 50% gray pixel reaches the screen as 128. If egui's
shader gets in the way, the final quad is drawn by our own paint callback with our own trivial shader.

**D15. Quality profiles.** High, Balanced and Low. They set the layer pool cap, the maximum layer resolution, blur
quality (Phase 9) and a render-scale factor. Chosen automatically from the adapter type and the measured frame time,
with a manual override in the editor settings and in exported games. MSAA stays 4x in all profiles (it is cheap with
D4) unless measurements on T2 or T4 say otherwise.

**D16. Gradients are color ramps baked by tiny-skia.** Each gradient becomes a 256-texel row in a ramp atlas (a texture
array, so there is room beyond 2 048 gradients). The row is produced by asking **tiny-skia itself** to draw a 256 × 1
gradient, so stop interpolation and premultiplication match by construction. The fragment shader only computes the
position along the gradient (linear, radial, and radial with a focal point if the data model has one) and applies the
spread mode.

**D17. Batching with chunked index lists.** Geometry lives in one big vertex buffer and one big index buffer, managed
by a sub-allocator (shapes change size when edited). Drawing order lives in **chunks**: each chunk is an index list
that concatenates the triangles of consecutive draw items in document order, and is drawn with one draw call. A
geometry or order change rebuilds only its chunk. Items whose geometry changes every frame (skinning, morphing) get
small chunks of their own. Expected draw calls on the stress scene: tens, not thousands.

**D18. Vello was considered and not chosen as the baseline.** Vello is the main Rust alternative for GPU vector
rendering. Its compute version cannot run on WebGL2. Its hybrid version (CPU tiling, GPU fine rasterization, with a
WebGL backend) was at version 0.0.x in 2026 and described by its authors as not yet production-ready. It also redoes
path processing on the CPU every frame, which is the cost this plan moves off the CPU, while lyon plus caching does it
once per change. Its analytic anti-aliasing is better than MSAA; it is revisited in Phase 12 as a possible
high-quality mode.

## 4. The phases

Sizes are relative (S, M, L, XL), not time estimates. Task numbers (0.1, 0.2...) are for reference in the logs.
**Every phase ends with** the checks of section 6: clippy clean, tests green (including the WebGL2-limits mode), the
benchmark re-run on the tiers named in the phase, and a dated entry in the progress log.

### Phase 0: GPU test bed (S)
**Goal:** be able to run and test GPU code, headless, on every tier, before writing any renderer.
**Why first:** every later phase is verified by comparing pictures and timing frames. Without this, we guess.

- [x] 0.1 Create the `evolve_gpu` crate (wgpu 30, lyon, bytemuck) and add it to the workspace. Nothing else.
- [x] 0.2 A function that opens a device without a window (`request_adapter` with no surface) and returns `None`
      when no adapter exists. Every GPU test calls it and prints "skipped: no GPU" instead of failing.
- [x] 0.3 A function that draws into an offscreen texture and reads the pixels back, so a picture can be compared
      with the CPU renderer in a test. Test: draw a colored rectangle, read it back, check pixels.
- [x] 0.4 Picture comparison helper: reports the worst pixel and the fraction of differing pixels. (Its rule is
      replaced by the edge-aware rule in 0.11.)
- [x] 0.5 Benchmark scenes owned by the repo: `evolve_editor --gen-scene <kind> <out>`, kind `large` (about 20 000
      shapes and 700 000 points). Keep the IronKrieg import as the real-world scene.
- [x] 0.6 Prepare `--benchmark` for the GPU: it names the graphics card, reports the CPU time of walking the scene, and
      has empty GPU columns (record and submit time, GPU time, draw calls, bytes uploaded, VRAM), filled in task 2.13.
- [ ] 0.7 CI: check what each runner offers, do not assume. *(Prepared 2026-10-15, open point P1. Two jobs in
      `.github/workflows/ci.yml`: `gpu-probe` (non-blocking) runs the GPU tests with output on all three OSes, and
      `gpu-required` runs them on Linux with Mesa's software Vulkan driver and `EVOLVE_REQUIRE_GPU=native`, which makes a
      missing adapter a failure instead of a "skipped" pass, so that one place really runs every GPU test. Neither has run yet.
      Open until their logs have been read after a push and the decision is written here. Risk: the normal `test` jobs
      also run the GPU tests; if WARP or the macOS runner gives a different picture, that job turns red and the cause is
      found the same way.)* Linux runners can install Mesa's software Vulkan driver
      (lavapipe); Windows runners can use WARP through DX12; check what macOS runners expose. Decide what runs where.
      Tests must pass on a software adapter or through the "skipped" path, and nothing may crash without an adapter.
      Timing tests stay local, never in CI.
- [x] 0.8 **CPU renderer feature inventory.** Read `render.rs` and the data model and list every visual feature with
      its exact options (fill rules, joins, caps, miter limit, gradient kinds, focal point, spread modes, dashes,
      blend modes, masks, filters and which primitives, text, raster images, patterns, anything else). Update
      section 2. Sort each feature: shape-level features the CPU has become tasks in Phase 3 (task 3.11); group-level
      features the CPU has become tasks in Phase 4 (task 4.9); features neither renderer has stay in Phases 9 and 10.
- [x] 0.9 Illustration scene: `--gen-scene illustration` as described in section 1 (curves, gradients, transparency,
      nested opacity and clip groups, symbols with overrides). Deterministic from a seed. Record its CPU timings.
- [ ] 0.10 Choose the T2 reference laptop (integrated GPU) and record the CPU-renderer baseline on it for all scenes.
      Note the browsers to be used for T3 / T3w.
- [x] 0.11 Edge-aware comparison rule (section 6) in the comparison helper: edge mask built from the CPU image,
      strict limit on interior pixels, loose limit on edge pixels, bound on the average error. Mutation check: move a
      rectangle by one pixel and confirm the rule fails; draw the same rectangle with MSAA and confirm it passes.
- [x] 0.12 WebGL2-limits mode for tests: open a device with the GL backend and wgpu's WebGL2 default limits where a
      GL adapter exists (skip otherwise), and run every GPU test in this mode as well.

**Done when:** a test draws on the card, reads back and passes (or skips cleanly) in normal and WebGL2-limits mode; CI
behavior is known; the inventory is in section 2; the benchmark runs on the three generated or imported scenes on T1
and T2.
**Result (2026-10-15, 0.1 to 0.6):** this machine's card: NVIDIA GeForce RTX 3060, Vulkan. `evolve_gpu` has 5 tests:
the readback test uses a width whose rows are not aligned to 256 bytes and fails if the padding is wrong (checked by
breaking it); the comparison helper has 4 tests. `--gen-scene large` writes 21 200 shapes and 660 000 points (57 MB,
same editor costs as the IronKrieg map; CPU renderer: 90 / 34 / 13 ms for the whole / region / close-up views at
1080p). Scene walk with world matrices: 0.07 ms, so walking the tree each frame is no problem.
**Result (2026-10-15, 0.8, 0.9, 0.11, 0.12; 0.7 and 0.10 still open):**
- *0.8 inventory:* in section 2. The CPU renderer has fewer features than the owner remembered: no dashes, blend modes,
  masks other than clip groups, filters, patterns, images, text, focal points or spread modes. Its distinctive
  behavior is whole-node opacity (layers), which tightened tasks 3.6, 4.1 and 4.4. Q7 answered.
- *0.9 illustration scene:* `--gen-scene illustration`, 4 976 shapes + 3 300 instances, 35% semi-transparent, nested
  opacity groups, 30 clip groups, 1 055 overridden instances (tests check every feature and determinism). **CPU
  baseline on T1's machine: 2 788 / 1 930 / 1 873 ms** (whole / region / close-up) at 1080p, about 0.4 fps, almost
  all of it full-screen layers for opacity groups (528 ms with every opacity set to 1). Details in BENCHMARKS.md.
- *0.11 comparison rule:* `compare::compare_pictures` with the edge-aware `Rule` (change step 12, interior limit 3,
  edge limit 96, 0.1% of interior pixels may exceed, mean under 1). 8 unit tests, one per rule part, plus tests on
  real card output (`test_draw.rs`): a 20-degree tilted rectangle drawn with 4x MSAA passes against tiny-skia (worst
  edge difference 64 of 96, mean 0.33, interior untouched, on Vulkan and on GL); the same rectangle one pixel to the
  right fails; without MSAA the worst edge difference is larger than with it. Mutation check done: treating every
  pixel as an edge made two tests fail.
- *0.12 WebGL2-limits mode:* `Mode::WebGl2` opens the OpenGL backend with wgpu's `downlevel_webgl2_defaults` (2 048
  textures, 16 KB uniforms, no storage buffers); `Gpu::for_each_test_mode` runs a test in every mode the machine can
  open and prints "skipped" for the rest. On the development machine both modes exist (Vulkan, and GL on the same
  RTX 3060); a test checks that the limits really are WebGL2's. All GPU tests use it.
- *0.7 CI:* see the task. *0.10:* waiting for the owner.

### Phase 1: Change tracking (M)
**Goal:** know cheaply which nodes changed, and how, since the last frame. Needed by the GPU cache (Phase 2), the undo
check and picking.
**Why before the renderer:** without it, the GPU view either shows stale shapes after an edit or re-tessellates the
whole scene (60 to 100 ms) after every change.

- [x] 1.1 Audit: list every place that mutates the scene (editor tools, inspector, undo/redo, import, scripts, physics,
      animation playback, `instantiate`). Write the list into this file. Decide the counter kinds (proposal: geometry,
      transform, paint, structure). *(Done 2026-10-15; the list and the decisions are below. Five kinds: the proposal
      plus `data`, for name, locked, script, body and bone settings, which are compared by the undo history but never
      drawn.)*
- [x] 1.2 Revision counters on nodes, plus a global scene revision. Mutation goes through functions that bump them;
      direct field writes from outside the model are removed or wrapped. *(Stamps instead of counters, a guard
      instead of removing field access; see "Decisions" below.)*
- [x] 1.3 A "changes since revision R" query that returns the changed nodes and the kinds of change, without walking
      the whole scene. *(Revised: `Scene::revision()` and `Scene::changed_since(&before)` give the kinds of change
      anywhere in the scene in O(1). Which nodes changed is found by comparing per-node stamps during the renderer's
      own walk, not by a change log; reasons below.)*
- [x] 1.4 Use it for the editor's undo check (3.6 ms per repaint today). Measure. *(3.6 ms to 0.0 ms.)*
- [x] 1.5 Tests: each mutation path bumps the right counter; mutation check: remove one bump and confirm a test fails.
      *(18 audit tests in `revision_tests.rs`, 5 history tests, one idle-frame test over the real panels. 15 removed
      stamps were tried, 15 caught; the audit and the idle test were each shown to fail on a deliberate bug.)*

**Audit (1.1), 2026-10-15.** Everything that changes a scene, by where it lives, and how it is now tracked:

- *Model methods that edit the tree* (`scene.rs`): `add`, `insert`, `remove`, `move_node`, `shift_order`, `group`,
  `ungroup`, `duplicate`, `combine`, `boolean`, `release_compound`, `center_origin`, `make_clip_group`. Structure
  changes stamp the parent and the scene; nodes that are moved, merged or copied carry their own stamps.
- *Symbols* (`symbol.rs`): `make_symbol`, `update_symbol`, `place_instance`, `symbol_copy`, `detach_instance`,
  `delete_symbol`, `rename_symbol` (new; the editor wrote the name directly), instance color overrides.
- *Rig* (`rig.rs`): `add_bone`, `set_bone_world`, `solve_ik` (runs every frame in a game), `copy_ik_bones_to`,
  `create_ik`, `bind_skin`, `unbind_skin`, `paint_skin`.
- *Animation* (`animation.rs`): `apply_pose` (every frame while a clip plays: transform, opacity, morphed path, fill
  and stroke color); recording keys edits `clips`, which are not part of the picture.
- *Clipboard* (`clipboard.rs`): `paste_nodes`, `spawn` (also called by scripts), `copy_nodes` (writes world transforms
  into the copies).
- *Game cameras* (`game_camera.rs`): `new_camera`, `upgrade_legacy_view` (on load, goes through `insert`).
- *Running games*: physics (`translate_world`, every frame for every moving body), scripts (`set_position`,
  `move_by`, `set_rotation`, `set_scale`, `set_opacity`, `set_visible`, `set_fill`, `set_stroke`), `runtime.update`
  (clips, scripts, physics, `spawn`).
- *Whole-scene replacement*: undo and redo (`history.rs`), opening a file, switching scenes (`app.rs`,
  `scenes_ui.rs`), loading a project (`io.rs`), the clip preview and the runtime's working copy. These swap in a
  clone or a freshly read scene, so their stamps are those of the clone, or new ones. Nothing special is needed.
- *Editor tools*: Select (drag: `apply_world_delta`), Node (anchor and handle drags, insert, smooth/corner, split,
  delete anchor), Pen, Pencil and shape drawing (new shapes through `add`), Objects panel (rename, visibility, lock,
  reorder and re-parent through `move_node`), Inspector (name, clip flag, opacity, camera, instance overrides, fill,
  stroke and fill rule of selected shapes, physics body, script), Rig panel (bone length and angle, IK list),
  Assets panel (symbol rename), Timeline (clips).
- *Builders* (no need to track, they make new scenes): `sample.rs`, `bench_scene.rs`, SVG import
  (through `add`).
- *Outside the node tree*, compared by the history directly because they are small and not drawn: scene name, `clips`,
  `ik`, `next_id`, the legacy size and background of old files.

**Decisions of Phase 1 and their reasons.**
- *Stamps from a process-wide counter, not per-node counters.* With counters that start at zero in every copy, two
  copies that change differently can show the same number (this is the classic undo problem: go back, edit, and the
  renderer cannot tell that this version is not the one it saw before). A shared counter cannot repeat.
- *No change log.* A log of "node X changed at R" needs a cursor per consumer and a way to say "R is not in my
  history" after undo; the renderer already walks the whole scene every frame (task 2.4), so per-node stamp
  comparison costs nothing extra and is always right, including after undo. The scene-level stamps give the O(1)
  "nothing changed" answer that idle frames (2.10) and the undo check need.
- *A guard, not private fields.* `Scene::find_mut` returns `NodeMut`. Reading is free. Writing a field directly
  stamps everything (always correct, one re-tessellation of one node at worst). Precise methods (`set_transform`,
  `set_opacity`, `set_visible`, `set_solid_fill_color`, `path_mut`, ...) stamp only what they change and are used on
  every per-frame path (physics, scripts, poses, rig, dragging). Making `Node`'s fields private would have meant
  hundreds of edits for readers too.
- *Limits that remain.* `Node` and `Scene` fields are still public, so code can write around the guard. Protection
  is: the audit tests, the debug-build cross-check in the undo history (it compares for real and complains on the
  console, and fails in tests), and the rules in CONTRIBUTING.md. A panel that hands `&mut node.field` to a widget
  would stamp on every frame; the idle-frame test runs the real panels on every node and catches that.
- *What a stamp cannot say alone* (for Phase 2): a **skinned** shape is drawn bent by bones that are other nodes, so its
  picture also depends on their transform stamps (treat as dynamic, Phase 6); a **group's** transform and opacity
  change its descendants' world matrices and layers; an **instance** also depends on its symbol's artwork (compare the
  symbol root's stamps); STRUCTURE stamps are only comparable for the same node id (a duplicated group has new child
  ids), while geometry, transform, paint and data stamps may be compared across nodes (equal stamp means equal content,
  so duplicated shapes can share tessellation).

**Done when:** every mutation path is covered by a test, and the undo check is near zero when nothing changed.
**Result (2026-10-15):** done. The undo check on the large generated scene went from **3.6 ms to 0.0 ms** per repaint
(illustration scene 0.2 to 0.0 ms); the project file format is unchanged (stamps are not saved; the frozen-fixture tests
pass). `cargo test --workspace` green (engine 127, editor 30, GPU 13), clippy clean. The tests found two real mistakes
in the first version: `set_bone_world` stamped every bone's geometry and data on every IK frame, and `move_node`
(re-parenting into a transformed group) forgot the scene-level transform stamp (`ungroup` had the same gap and was
fixed with it). Reading the editor found a third kind: the physics, script, inspector, camera and symbol-name panels
handed `&mut` fields to widgets on every frame, which would have counted as an edit on every repaint; they now edit a
copy and write back on a change, and the idle-frame test guards that. The first versions of the tests also missed
copies that inherit a stamp (`release_compound`, `detach_instance`, `paste_nodes`), which showed that the audit needed
the global rule "one stamp never stands for two contents". Two things found on the way are not caused by this phase: deleting or grouping 2 000 to 4 000 selected shapes in the large
scene takes 0.4 to 1.3 s (open point P9), and the `Scene` pane and `Game` pane were not part of the idle-frame test
(they need a graphics context). One new thing to know: any code that destructured `find_mut`'s result, like
`Some(Node { kind: .. }) = scene.find_mut(id)`, now needs `&mut node.kind` instead.

### Phase 2: Minimal GPU renderer (L)
**Goal:** the stress and illustration scenes drawn on the GPU with solid-color fills and strokes, batched from day one,
behind a switch, measured on T1, T2 and a WebGL2 canary. First real numbers.

**Order of work (agreed 2026-10-15; the task numbers below are only names).** Safety first, no drawing before the
foundation exists:
1. *Foundation, no drawing:* 2.16 (allocator with byte tracking and budgets); the core of 2.17 (uncaptured-error handler,
   OutOfMemory error scopes inside the allocator); from 2.19 the CI timeouts and the `--benchmark` time limit. Small, every
   later task uses them, and the timeouts must exist before more GPU tests reach CI.
2. *Geometry on the CPU side:* 2.1 (vertex format and sub-allocator), 2.2 (tessellation cache). The first rows of the clamp
   table (2.18) belong here: the per-shape vertex cap and the tolerance limits. Test: editing one shape re-tessellates only
   that shape, using the Phase 1 stamps.
3. *First pictures, headless only:* 2.3 (shape table), 2.4 (draw list and chunks), 2.5 (pipeline), 2.6 (targets). Then picture
   tests against the CPU renderer on simple scenes in both device modes, and a headless benchmark of the stress scene with a
   fixed frame count.
4. *Into the editor:* 2.15 (frame pacing, before anything is shown), 2.7 (color handshake), 2.8 (egui glue), 2.9 (feature
   gate), 2.10 (idle frames), 2.11 (renderer switch). Now that a headless device exists, extend the idle-frame test of the
   editor panels to the Scene and Game panes. Finish 2.17 (CPU fallback on device lost) and the debug watchdog of 2.19.
   **Then stop and let the owner try it by hand.**
5. *Measure:* 2.12 (web canary), 2.13 (benchmark), 2.14 (checkpoint; needs the T2 laptop, open point P2).

- [x] 2.1 Vertex format and geometry store (D17): one vertex format for fills and strokes, `point: f32×2`,
      `normal: f32×2` (zero for fills), `distance: f32`, `row: u32` (24 bytes). One vertex buffer and one `u32` index
      buffer, managed by a sub-allocator (size classes plus a free list); compact when more than 30% is free.
      *(Done 2026-10-15: `vertex.rs`, `ranges.rs`, `store.rs`. The index buffer holds the index lists of the chunks (task
      2.4), not of single shapes; per-shape indices stay on the CPU, relative to their vertices. Growing and compacting
      replace the buffer and change its `generation`, so the draw list knows to rebuild.)*
- [x] 2.2 Tessellation cache (D3): for each shape, build the lyon path from its anchors (all subpaths, lines and
      curves); tessellate the fill (nonzero / even-odd) and the stroke (join, cap, miter limit) with a vertex
      constructor that records point, normal and distance. Tolerance from the effective scale bucket. Cache keyed by
      the geometry revision (Phase 1) and the bucket. In this phase a bucket change re-tessellates synchronously.
      Clamp-table rows (GPU_SAFETY B2): the per-shape vertex cap (above it, a coarser tolerance and a warning) and the
      tolerance limits. Test: editing one shape re-tessellates only that shape (Phase 1 stamps). *(Done 2026-10-15:
      `tessellate.rs`, `tess_cache.rs`.)*
- [x] 2.3 Shape table (D10): the RGBA32Float texture, row allocation with a free list, dirty-range uploads. Paint is
      solid color only in this phase. *(Done 2026-10-15: `table.rs`, 1 024 texels wide, 256 rows to a line; uploads
      whole changed lines, neighbouring lines in one copy; grows by doubling up to the device limit.)*
- [x] 2.4 Draw list: walk the scene in document order each frame (0.07 ms measured), produce draw items (fill, then
      stroke) with increasing depth values (D11), and keep the chunked index lists (D17) up to date by comparing
      the per-node stamps of Phase 1 during the same walk (D7). *(Done 2026-10-15: `draw_list.rs`. Chunks hold 512
      consecutive items, so editing one shape rebuilds one chunk; rows are kept per item for as long as it exists. Items the
      GPU path cannot draw yet are counted, see the feature gate in 2.9.)*
- [x] 2.5 Pipeline: one render pipeline. Vertex shader reads the row with `textureLoad`, applies the transform and the
      camera, applies the stroke width (world width, and the hairline rule of D5c). Fragment shader outputs the
      premultiplied color times opacity (and times the hairline factor). Premultiplied alpha blending; depth test
      "strictly greater", depth write on; depth cleared to 0. *(Done 2026-10-15: `shader.wgsl`, `pipeline.rs`. Depth
      values are multiples of 2^-23, so there can be up to 8.3 million items.)*
- [x] 2.6 Targets (D2, D4): `Rgba8Unorm` 4x MSAA color and `Depth24PlusStencil8` 4x MSAA, both stored with
      `Discard`; single-sample `Rgba8Unorm` resolve. Sized to the view in physical pixels, **grow-only with 25% slack**,
      shrunk only after the view has been smaller for a few seconds. Dragging a panel edge must not reallocate every
      frame. *(Done 2026-10-15: `targets.rs`, `renderer.rs`.)*
- [x] 2.7 Color handshake (D14): a test renders an egui frame that shows a GPU texture filled with 50% gray into an
      offscreen target, reads it back, and expects 128 (± 1). If it fails, draw the final quad with our own callback.
      *(Done 2026-10-15. egui's own texture shader would encode our gamma-encoded `Rgba8Unorm` picture a second time on the
      usual window format, so the final quad is drawn by our own paint callback and shader (`quad.wgsl`), which puts the
      bytes on the screen as they are, on a window format that is sRGB and on one that is not. Tests: 50% gray gives 128 on
      both, and the GPU picture matches the CPU bitmap shown through egui's own image path, both in both device modes.)*
- [x] 2.8 egui glue (feature `egui`): a `GpuView` that uses eframe's device and queue, records the scene pass in the
      paint callback's `prepare` step, and shows the resolve texture; re-registers the texture after reallocation.
      The Scene and Game views draw it where they draw the bitmap today. *(Done 2026-10-15: `egui_glue.rs`. One change
      from the wording: the scene pass is drawn and submitted when the view is painted, not inside the callback's `prepare`,
      because a paint callback has to own what it uses and cannot borrow the scene; egui's own pass is submitted later in the
      frame and a queue runs in order, so the picture is ready when egui draws it. The picture texture is not registered
      with egui (that is why the callback exists), the bind group is made again when the targets are replaced.)*
- [x] 2.9 Feature gate: `supports(scene)` lists the features the GPU path does not have yet. When a view's content uses
      one, that view falls back to the CPU renderer and shows a small notice. This makes the switch safe in every
      phase and is removed only when nothing is left unsupported. *(Done 2026-10-15: `Renderer::unsupported(scene)`
      names gradients, opacity, clip groups, symbol instances and skinned shapes; `renderers.rs` remembers the answer for as
      long as the scene's stamps stay the same and shows the notice. Tested: a scene that becomes unsupported falls back on
      the next frame and comes back when fixed.)*
- [x] 2.10 Idle frames: if neither the scene revision nor the camera nor the view size changed, reuse the last resolve
      texture and submit nothing. Extend the idle-frame test of the editor panels (`idle_tests.rs`) to the Scene and Game
      panes, which need a graphics device and so could not be covered before. *(Done 2026-10-15: the key is the scene's
      stamps, the camera matrix, the view size and the options; an unchanged frame draws nothing and sends nothing, a view
      with no size costs nothing. `idle_tests.rs` now runs the Scene and Game panes too, and `gpu_view_tests.rs` checks
      that five idle frames draw no more than the first.)*
- [x] 2.11 The switch: `evolve_editor --renderer auto|cpu|gpu` and a *View → Renderer* menu entry. Default stays CPU
      until Phase 7. *(Done 2026-10-15. The choice is not remembered between runs yet (task 7.5).)*
- [x] 2.12 Web canary: a small wasm page (`tools/web-canary`) that renders the generated scenes with `evolve_gpu` on
      WebGL2 and on WebGPU, with a frame-time display. Run it on the T2 laptop. Its only purpose is to find WebGL2
      problems now instead of in Phase 8. *(Built and run 2026-10-17 on T1 in headless Edge: WebGL2 on the real card (ANGLE
      Direct3D 11), WebGL2 on a software driver (SwiftShader) and WebGPU all ran the scenes with **no errors from the
      graphics layer**, and the picture is right (saved from the canvas and looked at). The run on the **T2 laptop** is
      still to do (P2): see MANUAL_TESTS.md, "The web canary". Numbers: BENCHMARKS.md. Found: the cold first frame (every
      shape tessellated) takes 1.5 s in the browser against 0.19 s natively, see Phase 5 note.)*
- [ ] 2.13 *(T1 and the canary done 2026-10-17, tables in BENCHMARKS.md; T2 waits for the laptop, P2)* Benchmark the three camera views of the stress and illustration scenes on T1, T2 and the canary; fill the
      GPU columns; record CPU and GPU side by side in BENCHMARKS.md, one table per tier. Add a **timing-only "draw what
      you can" mode** (`--benchmark ... --gpu-draw-supported`): in Phase 2 the feature gate (2.9) would send the
      illustration scene back to the CPU renderer, so this mode draws the supported items on the GPU and skips the rest, and
      the report says clearly that its picture is incomplete and what was skipped. Timing only; never a picture test.
- [ ] 2.14 **Checkpoint:** revisit the targets of section 1 with the real numbers, per tier, and update them.
- [x] 2.15 Present mode and frame pacing for the editor views (GPU_SAFETY A1, A3, A4); test that a hidden or zero-size view
      submits nothing. Must be done before the first GPU view is shown in the editor. *(Done 2026-10-15: the window
      presents with `AutoVsync` set explicitly, with a test that it is not `Mailbox` or `Immediate`; the editor asks for
      continuous frames only while a game runs or a clip plays (checked in the code, `anim.rs` and `game.rs`); a view with
      no size draws nothing and submits nothing.)*
- [x] 2.16 Allocator with byte tracking and budgets per category (C1 to C3); no `create_buffer` or `create_texture`
      outside it; leak test (C4: same totals after many frames and resizes). *(Done 2026-10-15: `evolve_gpu::alloc`, see
      the result below. The leak test of C4 is written together with the renderer, because it needs frames to render.)*
- [x] 2.17 Error handling (D1, D3, D4): uncaptured-error handler, CPU fallback when the device is lost, no panics from
      GPU errors in release builds; validation stays on in development (D2). Split: the handler and the OutOfMemory
      error scopes come first (with the allocator, 2.16); the CPU fallback on device lost comes with the editor glue.
      *(Done 2026-10-15: `evolve_gpu::errors`, and in the editor `renderers.rs`: a lost device, an error reported by the
      card, or a resource that cannot be had (budget, device limit) switches the GPU renderer off for good, shows the
      message in the status bar and in each view, and the views carry on with the CPU renderer. Tested with a simulated
      loss, a simulated error and a tiny budget. Full recovery that rebuilds GPU resources is task 7.4.)*
- [x] 2.18 Clamp table (B2) for everything Phase 2 draws, clamped when read and again before upload; every later phase adds
      its rows to the table in GPU_SAFETY.md. *(Rows done with 2.2: tolerance 1e-4 to 100 units, scale buckets 2^-16 to
      2^20, miter limit 1 to 100, 4 million vertices per shape with up to six 4x coarsenings, non-finite coordinates drop
      the shape with a warning; texture and buffer sizes against the device limits are in the allocator; at most 8.3 million
      draw items per frame (depth buffer); shape table rows up to the device's texture size and the budget. All in the table
      of GPU_SAFETY.md. Each later phase adds its rows.)*
- [x] 2.19 Debug watchdog for frames over 100 ms (B4), time limit for `--benchmark` (F2), timeouts for GPU test processes in
      CI (F1). *(Done 2026-10-15: the benchmark limit, the CI timeouts, and the watchdog: in development builds a frame
      that took over 100 ms on the CPU, or until the card was done (measured with the queue's work-done callback, so nothing
      waits), is named on the console with the view and the scene, at most once a second.)*
**Risks:** the WebGL2 path may show driver differences (float textures, integer attributes); found early by 2.12.
**Done when:** the scenes render on the GPU on T1, T2 and the canary, simple scenes pass the picture tests, and
BENCHMARKS.md shows CPU and GPU together per tier.
**Result:** *(fill in)*

### Phase 3: Shape parity (M)
**Goal:** every shape-level feature of the CPU renderer looks the same on the GPU, at every zoom.

- [ ] 3.1 Hairlines (D5c): a regression scene of strokes from 0.1 to 3 pixels wide at three zooms, compared with
      tiny-skia. Manual check for shimmering while zooming (MANUAL_TESTS.md). *(The picture test exists since Phase 2,
      `strokes_thinner_than_a_pixel_look_like_tiny_skias_hairlines`, on both device modes; the manual shimmer check is still
      to do: tick this when the owner has looked.)*
- [x] 3.2 Zoom buckets (D3): effective scale = camera zoom × the largest scale of the shape's world matrix; power-of-two
      buckets with hysteresis (no flapping when the zoom sits on a boundary). Test: a symbol scaled up 10× and a round
      join seen at deep zoom stay smooth. *(Done 2026-10-17: a shape stays in its bucket while the scale is within 10% of its
      edges (`bucket_holds`), tested on the cache (a wobble around 1.0 and 2.0 makes nothing again) and with a shape scaled by
      its own transform ten times and viewed six times closer. That picture test found that strokes were flattened up to
      three times coarser than the nominal tolerance (an edge 0.75 px too far in at a deep zoom), so strokes now use 0.35 of
      the fill tolerance, 13% more vertices on the map. A symbol is Phase 4; the transform stands in for it.)*
- [x] 3.3 Gradient ramp atlas (D16): texture array of 256-texel rows, rows baked by tiny-skia, freed when unused. *(Done 2026-10-17,
      `ramps.rs`: one 2D texture of 256 texels by 64 rows that doubles up to the device's size (2 048 gradients on WebGL2, an
      error and the CPU renderer beyond), not a texture array: array layers are limited to 256 on WebGL2. Ramps are shared by
      gradients with equal stops and freed when no item used them in a frame. At most 256 stops are used.)*
- [x] 3.4 Gradient fills: linear and radial (the data model has no focal point and only pad, see section 2), the
      gradient's own transform, any number of stops, a gradient with no usable stops draws nothing. Paint kind and ramp
      row in the shape row. Instance color overrides replace the paint by a solid color. *(Done 2026-10-17. The shape table
      row grew from 4 to 8 texels for the gradient's geometry; the shader gets the point in the shape's own space and computes
      the position along the gradient from an affine function (linear) or the length of one (radial). Which gradients are
      really one color (a single stop, start = end, radius 0) or nothing, or whose transform cannot be inverted (tiny-skia
      paints the last color) is decided by asking tiny-skia. Instance overrides come with instances, Phase 4.)*
- [x] 3.5 Gradient strokes, same mechanism. *(Done 2026-10-17, with thin and wide strokes, round joins and caps.)*
- [x] 3.6 Opacity on a leaf shape is the row's opacity, no layer, **only when the shape has a fill or a stroke but not
      both** (found by the inventory: the CPU fades fill and stroke as one whole, so the stroke covers the fill's inner
      half first; per-item opacity would let the fill show through the stroke). A leaf shape with both and opacity
      below 1 goes through the layer system of Phase 4 (shortcut 4.4 does not apply), **unless both paints are opaque**
      (review, 2026-10-15): then it needs no layer. Give the fill and the stroke the same depth value and draw the
      stroke first; the depth test "strictly greater" rejects the fill wherever the stroke is, so each pixel gets
      exactly one of the two at the shape's opacity, which is what the layer would give (the stroke covers the fill's inner
      half). With a semi-transparent stroke the layer would blend stroke over fill, so that case still needs the layer.
      Regression scene: a fat half-transparent stroke over a fill, compared with the CPU renderer; mutation checks: apply
      opacity per item with different depth values, and confirm the test fails; give the two the same depth but draw the fill
      first, and confirm it fails. *(Done 2026-10-17, `draw_list.rs`: see the top of that file and the tests `a_fat_stroke_over_a_fill_with_opacity_covers_the_fill_like_the_layer_does` and `the_gate_lets_a_single_shape_with_opacity_through_only_where_no_layer_is_needed`. Both mutation checks of the review are caught.)*
- [x] 3.7 The camera: background fill, the frame, and the same treatment of what is outside it as the CPU renderer. *(Done early, in Phase 2: `the_camera_frame_background_matches_render` and `the_camera_and_the_zoom_of_the_view_match`.)*
- [x] 3.8 Self-overlapping semi-transparent strokes (D11): regression scene with sharp joins, self-crossing paths and
      50% alpha. Mutation check: switch the depth test off and confirm the test fails. *(Done early, in Phase 2:
      `a_semi_transparent_stroke_does_not_blend_over_itself`; the mutation "the depth test is off" is caught.)*
- [x] 3.9 Regression suite: every existing renderer test, the sample project, stroke style grids, a gradient grid, and
      the three benchmark scenes at three zooms, rendered both ways and compared with the edge-aware rule, in normal
      and WebGL2-limits mode. *(Done 2026-10-17. `gpu_parity_tests.rs`: both scenes of the sample project and the stress and
      illustration scenes at three zooms each, with what the GPU cannot draw yet taken out of both sides (by
      `Renderer::unsupported_nodes`), in both device modes. The two benchmark scenes need a looser average (3.5 levels
      instead of 1): they are dense hairlines and thousands of piled-up see-through strokes, where the card rounds to 8 bits
      after every blend; the picture is about 2 levels darker on average. The stroke style grid and the gradient tests are
      in `picture_tests.rs`. A failed comparison leaves both pictures as PNG files when `EVOLVE_DUMP_PICTURES` names a folder.)*
- [ ] 3.10 *(Waits for P4, the answer to Q2: not started.)* Pixel-width strokes (D5b), only if Q2 is answered yes: the stroke setting in the data model, CPU renderer
      support (width computed from the zoom at render time), the format version bump with a frozen fixture of the
      current version first (CONTRIBUTING.md), the shader flag, a regression scene, the CHANGELOG entry.
- [x] 3.12 *(Done 2026-10-17, and larger than it looked: see below.)* Exact reversals with a miter join: where a path turns back on itself by exactly 180 degrees, tiny-skia cuts the
      join flat (a bevel) and lyon draws a spike (found by the first picture tests, 2026-10-15; the near-reversal and the
      miter limit agree). Fix in the tessellator (for example by treating such a vertex as a bevel), then remove the `ignore`
      from `an_exact_reversal_with_a_miter_join_matches`.
- [x] 3.11 Shape-level features found by the inventory (0.8) that the CPU already has beyond the tasks above: **none**
      (checked 2026-10-15, section 2). Dashes, patterns and raster images stay in Phase 9 and are added to the CPU
      renderer first.

**Done when:** the suite passes for every shape-level feature of the inventory, on T1 and in WebGL2-limits mode.
**Result:** *(2026-10-17)* Every shape-level feature of the inventory is drawn by the GPU renderer and matches the CPU renderer
in both device modes: solid colors, linear and radial gradients (fills and strokes, with transforms, any stops, the degenerate
cases), opacity on a single shape without a layer (only a fill or only a stroke; both when both are opaque), every join and cap,
miter limits, hairlines, zoom buckets with hysteresis, the camera frame. Groups with opacity, clip groups, symbols and skinned
shapes are still left to the CPU renderer (Phases 4 and 6) and are counted by the feature gate node by node. 3.1 (the owner's
look at shimmering) and 3.10 (P4) are open. About 40 new tests and 60 mutation checks (all caught but one equivalent mutant
each in two places, listed in the log). The whole-map frame is back to about 7 to 9 ms on T1, of which 6 to 7 ms is CPU
bookkeeping (Phase 5).

### Phase 4: Groups and effect layers (L)
**Goal:** the features that need more than "draw triangles", built on one layer system.

- [x] 4.1 Stencil clipping (D12): the clip shape is drawn into the stencil only (no color, no depth), incrementing;
      the clipped content is tested against the current level; the level is popped afterwards. Nested clips. Exact
      semantics from the inventory: the top child must be a shape and only its fill silhouette counts (its fill rule;
      its colors, stroke, opacity and visibility do not); if the top child is not a shape the flag is ignored; the
      group's own opacity applies on top. Compare the anti-aliased clip edge with tiny-skia.
      *Done 2026-10-16 (the group's own opacity comes with 4.4).* Frames are lists of passes of ops (`draw_list.rs`), two extra
      pipelines push and pop masks, the cache makes a fill-only mesh for a mask and culls the whole group by the mask's box.
      7 picture tests against tiny-skia in both device modes (anti-aliased circle and turned rectangle, masks with no fill, a
      stroke only, 20% opacity, hidden, even-odd, nested, content drawn after, mask far away or half outside, zoomed in,
      non-shape top child, empty masks, gradients and opacity inside), 2 renderer tests (a far mask makes nothing of the
      content; idle frame and a moved mask upload nothing), 9 mutation checks all caught.
- [x] 4.2 Effect layer system (D13): pool of MSAA layer targets plus resolves, keyed by size class; layer bounds =
      group bounds on screen, clipped to the view, plus effect margins; memory cap per quality profile; reduced
      resolution when the pool is full, with a warning.
      *Done 2026-10-16 (`layers.rs`).* Sizes are rounded up to four classes per doubling. Per class: the layers' single-sample
      textures (4 bytes a pixel, the `Layers` budget, 64 MB on High) and one MSAA color and depth-stencil scratch set that every
      layer of the class draws in one after the other (32 bytes a pixel, counted with the targets), so a live layer costs 4 bytes a
      pixel and not 36. Layer boxes come from the control points of the shapes, their strokes and 3 px of margin, clipped to the view
      (and to a clip's mask). When a layer does not fit the budget or the device it is drawn at half, then a quarter, of its size and
      shown with filtering, and `FrameStats.layers_reduced` says so; it is never left out. More than 2,048 layers in one frame
      (`MAX_LAYERS`, each is a pass) is refused and the view goes to the CPU renderer (GPU_SAFETY.md B2). The caps of the Balanced and
      Low profiles come with the profiles (5.8, 5.9).
- [x] 4.3 Layer scheduling (D13): layers that do not read the backdrop are drawn in prepasses before the main pass;
      the main pass composites them as quads with their own depth value. Nested layers are scheduled innermost first.
      Clips inside a layer use the layer's own stencil.
      *Done 2026-10-16.* A frame is a list of passes (layers inner first, the main pass last), each with its own view uniform (one
      256-byte slot, dynamic offset) because a layer's picture is the view shifted by its corner. A composite is an ordinary item (a
      quad with a row) drawn by a second pipeline whose fragment shader reads the layer's texel exactly under the pixel, so the layer
      is not resampled; each pass has its own depth and stencil.
- [x] 4.4 Opacity groups through layers, with two shortcuts that skip the layer: a group with a single leaf child *that
      has a fill or a stroke but not both, or has both with opaque paints, see 3.6* (the opacity goes into the child's row), and a group whose children
      do not overlap (checked with their bounds, strokes included). Layers are sized to the node's bounds on screen,
      never to the whole view: the CPU renderer's full-screen layers are what makes the illustration scene take 2.8 s.
      *Done 2026-10-16.* The shortcut is applied recursively (a group with one child, or whose children's boxes plus a 1.5 px
      anti-aliasing margin are apart, passes its opacity down, and so does a clip group). A shape with both parts and a see-through
      paint, and any group whose children can touch, gets a layer the size of its box. The illustration scene (216 groups with opacity,
      3,300 instances, 15 clip groups) needs 276 layers at the whole view (28 in the region view), 2.2 megapixels in all.
- [x] 4.5 Symbol instances: the symbol's art is tessellated once; instances are rows pointing to the same geometry,
      carrying their transform and color overrides. Instanced draws are used for single-shape symbols and for symbols
      whose parts are all opaque (safe thanks to D11). Otherwise instances are drawn in document order, because
      instancing a multi-part symbol draws part A of every instance before part B and breaks overlaps.
      *Done 2026-10-16, with one part moved to Phase 5 (5.12).* The art is tessellated **once** for all instances (the cache keeps one
      entry per node and bucket, and a second one when instances of different sizes need another bucket in the same frame), each
      instance is its own set of items (a scope number in the key) in document order with its colour overrides (outer wins) and its own
      opacity layer, instances out of the view are culled one by one, and nesting is limited to 12 levels (a symbol that contains itself
      does not hang). What is *not* done is drawing the instances with one call: every vertex carries the shape-table row of its item,
      so each instance has its own copy of the vertices on the card. Drawing in document order is what makes overlapping instances of
      a see-through symbol right, and it is what the tests check; see 5.12 for the rest.
- [x] 4.6 Nested cases: opacity inside clip inside opacity, layers inside instances, deep nesting; behavior at the pool
      cap.
      *Done 2026-10-16:* faded groups three deep, a clip with opacity, a faded group inside a clip's content, a clip inside a faded
      group, faded instances of symbols whose art is clipped, instances inside instances with overrides at both levels, a layer
      drawn at half and at a quarter of its size inside and outside another layer, and the refusals (layer memory, layer count).
- [x] 4.7 Regression scenes for clips, opacity groups, overlapping instances of a two-part semi-transparent symbol,
      and nesting, with mutation checks (for example: instance a semi-transparent two-part symbol and confirm the
      overlap test fails).
      *Done 2026-10-16:* `clip_tests.rs` (7), `layer_tests.rs` (13), `instance_tests.rs` (9), the layer pool's own tests and renderer
      tests, in both device modes; whole-project parity (the sample and both generated scenes) now draws everything but the skinned
      tentacle. Mutation checks, all caught after the missing ones were added: clips 9, layers 14, instances 8, reduced resolution 7.
- [ ] 4.8 Measure the layer cost on T2 and the canary (number of passes, layer memory).
      *T1 measured 2026-10-16 (BENCHMARKS.md, "After Phase 4"): the complete illustration scene takes 9.9 ms of CPU and 6.8 ms on the
      card at the whole view (the CPU renderer 2.8 s). Still to do: T2 and the browser, which need the owner (open point P2).*
- [x] 4.9 Group-level features found by the inventory (0.8) that the CPU already has beyond opacity and clip groups:
      **none** (checked 2026-10-15, section 2). Blend modes, masks and filters stay in Phase 9, CPU first.

**Risks:** layer count and memory in deeply nested scenes; mitigated by the shortcuts of 4.4, the cap and reduced
resolution.
**Done when:** the suite includes these scenes and they match, in normal and WebGL2-limits mode.
**Result:** *(2026-10-16, except 4.8's T2 and browser measurements)* The GPU renderer draws clip groups, groups, shapes and instances with
opacity, and symbol instances with overrides; the only thing the feature gate still sends to the CPU renderer is a skinned shape (the
sample's tentacle, Phase 6). Pictures match tiny-skia in both device modes, with 37 new picture and behaviour tests and 38 mutation
checks. The complete illustration scene, the one the CPU renderer needs 2.8 s for, takes about 17 ms (9.9 CPU + 6.8 card) at the
whole view on T1. Costs: the whole-map pan frame is about a millisecond and a half slower than after Phase 3 (7 to 8.6 ms against
6), see BENCHMARKS.md. Left for later: drawing instances with one call (5.12), layer caps per quality profile (5.8, 5.9).

### Phase 5: Speed at scale, on every hardware tier (L)
**Goal:** the section 1 targets met on T1, T2, T3 and T3w, and the editor responsive with the stress scene open.

- [ ] 5.1 Measure on every tier and find the limit of each: CPU record time, uploads, vertex work, fill rate, layers.
      Write the findings here before optimizing.
      **T1 findings, 2026-10-20** (`the_cost_of_each_kind_of_frame`, an ignored probe; 1080p, RTX 3060, release; average over 30
      frames of the CPU side of `prepare`; stages are the cache walk / finding the items / rows, vertices and chunks):

      | frame | stress scene | illustration | real map | what it showed |
      |---|---|---|---|---|
      | nothing changed (same view again) | 6.8 ms (2.5 / 2.5 / 1.8) | 6.7 (1.0 / 2.8 / 2.8) | 7.2 | **a frame costs the same whether or not anything changed** |
      | pan 3 px | 6.9 | 6.9 (384 kB of table rows: layer places) | 7.0 | the walks again, nothing new to make |
      | zoom in or out 2% | 7.2 / 7.3 | **22 / 20** (105 shapes made, 350 chunks rebuilt, **13 MB** sent) | 7.3 / 7.4 | instance-heavy scenes pay for every instance when a shared shape is made again (5.12) |
      | one shape moved | 7.4 (one 16 kB table line) | 6.5 | 7.3 | the edit is free, the walks are not |
      | one outline edited | 8.9 (**2.1 MB** sent) | 6.6 (0.6 MB) | 7.9 (1.5 MB) | a whole chunk's index list is rebuilt for one shape |

      Limits found: (1) the three walks over every node, about 100 ns a shape each (hash lookups, cull arithmetic, building items,
      signing chunks), run on every frame; target for a frame where nothing changed is 0.5 ms and for a camera move or one edit
      about 1 ms (5.3); (2) a shape made again costs one upload per instance and rebuilds the chunks that hold them; (3) editing
      one shape rewrites a whole chunk (up to 2 MB). The card is not the limit on T1 for the map (1.1 ms), but for the
      illustration scene (6.8 ms, 276 layers redrawn every frame). Not measured: T2, the web (open point P2).
- [x] 5.2 Chunk tuning (D17): chunk size, rebuild cost after an edit, number of draw calls (goal: under 100 on the
      stress scene).
      *Done 2026-10-20, no change needed.* 512 items per chunk gives 42 draw calls for the stress scene and the real map (goal under
      100); an edited outline rewrites only its part of its chunk's index list (12 kB and 0.08 ms, see 5.3), so a smaller chunk would only
      add draw calls. Kept at 512.
- [ ] 5.3 Idle and edit costs: a frame where nothing changed under 0.5 ms of CPU with no uploads; a single-shape edit
      re-uploads only its geometry, its row and its chunk.
      *Done for plain frames 2026-10-20, not yet for frames with layers, instances, clips or fading.* Two ways to skip the walks:
      **a frame made from the last one** (`LastFrame` in `renderer.rs`) when the scene has the same stamps and the view only moved
      (same scale and turn) inside the area the last frame kept shapes for: only the view uniform is written; and **patching** when the
      scene's new journal (`Scene::nodes_changed_since`, `revision.rs`: which node got which kind of change, bounded, a copy of a
      scene has an empty one and a name of its own so that undo can never be mistaken for an edit) names the nodes that changed: for
      shapes at full opacity with solid colors in a frame that is plain, a node index finds each, its row is written (the depth stays,
      so the order does not change), and when its triangles were made again only its vertices and its part of the chunk's index list
      are written (`DrawList::patch`). Anything else (a node that is not a shape, a group moved, a part that appeared, a hidden or
      faded shape, the tree changed, gradients, a copy of the scene) makes the whole frame. Measured on the stress scene and the real
      map: unchanged or panned 6.9 ms -> 0.01 ms; one shape moved 7.4 -> 0.06-0.2 ms; one outline edited 8.9 ms and 2.1 MB -> 0.08 ms
      and 12 kB. In the editor a drag frame went from about 22 to about 5 ms. Tests: a randomized differential test (120 frames
      of random edits, pans, hides, fades, strokes added and removed, deletions, rescales into another zoom bucket, group moves,
      byte-identical to a new renderer every frame, in both device modes), the undo case, a faded group's children, 8 + 12 mutation
      checks (one equivalent mutant: the geometry flag next to the stamp check). Open: frames with layers or instances (the
      illustration scene still takes 6.6 ms for an edit), gradients, clips.
      *Zoom and layers, 2026-10-20 (later).* **A zoom is drawn from the last frame too** (`LastFrame::covers`, `scale_ratio`): a frame
      of plain shapes keeps the range of zoom factors in which every shape's triangles stay good (`Collected::window`, from
      `tessellate::bucket_window`: a bucket holds from 0.9 of its lower edge to 1.1 of its upper one, so a window is usually 0.5 to
      1.1 or wider), and a view that is the old one made larger or smaller inside that window, by at most a factor of 2, and lies 4
      pixels inside the area that was kept, is drawn from it. A 2% zoom on the stress scene and the real map went from 6.2 ms to
      0.01 ms. A shape made again by an edit narrows the window, and an edit is only brought up to date at the zoom of the last
      frame. **Frames with layers or instances are reused for moves** (`LayerMap` in `draw_list.rs`): each layer keeps its box on the
      screen before it was clipped to the view; when the view moves the box moves, the rectangle of the layer is cut again, the
      composite's line of the table is written again, and a layer whose rectangle changed past its texture's size class gets a
      texture of the new class (`LayerPool::resize`); a layer that leaves the view, or one that was out of it and comes in (nothing
      inside it was collected), makes the frame whole. Frames with a layer drawn smaller, or with work waiting, are never reused. The
      illustration scene: a pan 6.4 ms -> 0.09 ms, the same view again 6.3 -> 0.04 ms. The layers are still drawn again on the card in
      every frame (5.13). Tests: pictures byte-identical to a renderer of the same history that never reuses (`Pair` in
      `renderer_tests.rs`), in both device modes, for zooms inside and outside the window and the range, for the inset, for edits
      at another zoom, for layers entering, leaving and changing class, for layers inside layers, for instances and for a view of
      another size; 15 + 14 mutation checks (one equivalent mutant: a polygon's window, which the walk already leaves out).
- [ ] 5.4 Background re-tessellation for zoom buckets: a thread pool on desktop; on the web (single thread) the work is
      split across frames with a time budget (proposal: 2 ms per frame). The old geometry is shown until the new one is
      ready. **Measured 2026-10-17 (web canary):** tessellating the 21,000 shapes of the stress scene once takes 1.5 s in the
      browser (0.19 s natively), so on the web the *first* frame needs this too (show something at once and fill in), and a
      zoom bucket change is a 1.5 s freeze there, not 130 ms. Also check whether the wasm build is slower than it should be
      (optimization level, `lyon` settings) before building the spreading.
      *Desktop part done 2026-10-20 (a thread pool was not needed).* `TessCache::set_budget`: once an update has spent its time (4 ms
      in the editor's views, 2 ms in the browser) a shape that only needs another bucket keeps its old triangles (`stale`, a little
      too coarse or fine) and a shape that is not in the view yet (only in the margin kept ready) waits; a shape in the view that has
      no triangles at all is always made at once, so a frame never has holes. The frame says `deferred` and the view draws again
      (a repaint is requested and the view is not marked as shown). Real map, 20 zoom steps in the editor: worst frame 169 ms -> 45 ms
      (zoom out 134 -> 36), average 25 -> 18 ms. Tests: no holes on the first frame, spread over several frames, ends exactly as a
      new renderer draws, in-view shapes made with no time left, 7 mutation checks.
      *Web part, 2026-10-20: the slowness was memory, not the tessellator.* Measured in headless Edge (WebGL2, SwiftShader), stress
      scene: the same tessellation takes 76 ms when run again in the same page, and a diagnostic that dropped its triangles ran at 3 ms
      per 2,000 shapes after the first batch, while one that kept them slowed down batch by batch (3 -> 230 ms) every time the program's
      memory had to grow. A WebAssembly program asks the browser for a little more memory each time it runs out, and in the browser each
      growth costs more than the last. Waiting for the optimized compile (a `warm_up` call) changed nothing, so it was removed.
      Fix: `evolve_gpu::reserve_memory(bytes)` grows the memory once, at the start, and `memory_for_shapes(n)` sizes it (24 KB a shape,
      clamped to 1 GB; the scene needs 320-448 MB, because the working space is far more than the 1.8 KB of triangles each shape
      leaves). Reserved sizes against the cold first frame: none 1,280 ms, 192 MB 1,280 ms, 320 MB 624 ms, 448 MB 168 ms,
      768 MB 155 ms; with the sized reservation (485 MB, 93 ms to make) the first frame is 158 ms, 1.3 s -> 0.25 s in all. Not
      measured: Andrea's own Chrome (P2). Open: why a shape needs 20 KB of memory (it is also a native cost: see 5.8), and the
      region view's first frame (400 ms: 250 ms of it building the items, in the wasm build), which the "frames that do not walk the
      scene" work does not reach. The "show shapes as they are made" mode is not needed for this scene any more.
- [x] 5.5 Precision at large coordinates: a floating origin. World positions are stored relative to an origin near the
      camera; when the camera moves far away, the origin is moved and the rows are rewritten (rare). Test: zoom deep
      into a far corner of a very large scene and check that edges do not wobble.
      *Done differently, 2026-10-20, after measuring.* The scenes we have reach 10,300 units from the origin (the real map 10,271,
      the stress scene 10,322, the illustration 4,312) and the editor zooms up to 1000x. `how_far_from_the_origin_can_a_shape_be_before_it_wobbles`
      (an ignored measurement, GPU and CPU renderer against the same drawing at the origin): at 10,000 units straight-sided shapes are
      exact up to 100x and within a quarter of a pixel at 500x, as the CPU renderer's are; at 100,000 to 1,000,000 units both renderers
      wobble. So no floating origin with rewritten rows: the vertex shader now works **relative to a point near what it shows** (a
      centre chosen in double precision on the CPU for each pass, and where it lands in pixels), so that it never subtracts two large
      numbers: 2 to 3 times better far out (mean error at 10^6 units 0.446 -> 0.131 of 255), and a test with a mutation check holds it.
      Left as it is, because it is under a quarter of a pixel at the map's own distance even at 500x: the curves' flattening (lyon
      works in f32 at absolute coordinates, so a curve's points are on a grid of 0.001 units at 10^4). If a project needs more, the
      next step is to tessellate each shape relative to its own centre (the row then carries the centre).
- [ ] 5.6 Opaque front-to-back pass (D11), only if fill rate is a limit (T2 at 1080p, T1 at 4K): opaque items drawn
      front to back first, then the rest in order.
- [ ] 5.7 Culling. **Done early (2026-10-16): view culling** (`cull.rs`): a shape whose control-point box, grown by its stroke's
      reach and a margin, cannot touch the view is neither tessellated nor drawn; the picture is the same (tested with
      byte equality on both device modes, and with wide miter strokes starting far outside). Still open: per-chunk bounds,
      spatially coherent chunks only if the measurements ask for it. GPU culling stays an optional extra (D8).
- [x] 5.8 Memory: two budgets, reported by the benchmark. *Content* (geometry, shape table, ramps, glyphs later):
      proposal under 64 MB for the stress scene. *Targets*: by formula, since they depend on the view size (at 1080p
      the MSAA color and depth-stencil plus the resolve are about 75 MB; at 4K about 300 MB; two views open double it).
      Layer pool caps per profile (proposal: 64 MB High, 32 MB Balanced, 16 MB Low).
      *Done 2026-10-20 (the numbers are proposals; T2 may change them: P2).* `profile.rs`: `QualityProfile::budgets()` gives the caps per
      kind of resource for High / Balanced / Low (geometry 512 / 256 / 128 MB, shape table 32 / 16 / 8, ramps 16 / 8 / 4, layers 64 /
      32 / 16, targets 768 / 384 / 192, glyphs 64 / 32 / 16, other 128 / 64 / 32); High is exactly the budget the renderer had before.
      A device starts with the caps of its adapter's profile and the user's choice changes them (`GpuContext::set_quality`). The
      benchmark reports the profile of the card and splits the memory it holds into content, layers and targets
      (`GPU memory: content / layers / targets`). Measured at 1080p: stress scene, whole view, content 68 MB, targets 75 MB (the
      formula above); the region view 21 MB; the close-up 1 MB. The real map: 81 MB, 32 MB, 3 MB. So the proposal of under 64 MB of
      content for the stress scene is **not met** for the whole view (68 MB for 1.1 million vertices, 43.6 MB of them sent in the first
      frame, the rest slack of the buffers' size classes); it is met for every view that does not show all 21,000 shapes at once. The
      browser's heap is a separate matter (the 20 KB a shape in 5.4).
- [x] 5.9 Quality profiles (D15): automatic choice from the adapter type and measured frame time, manual override,
      render-scale factor. Check that T2 and the web meet their targets with the automatic choice.
      *Mechanism done 2026-10-20; the check on T2 and the web is still open (P2).* `QualityProfile::for_adapter` (discrete card High,
      integrated, virtual or unknown Balanced, software Low; WebGL2 mode no better than Balanced), `Quality` (Automatic or one profile
      by hand), `--quality auto|high|balanced|low` and *View → Renderer → Quality* in the editor, and a `Governor` per view: it
      lowers the profile one step when the median of 30 frames is over 33 ms, raises it one step after 180 frames in a row at 10 ms or
      less, never above the adapter's (or the chosen) profile, and judges nothing for 90 frames after a change. A profile sets the
      **render scale** (High and Balanced 1.0, Low 0.75: the picture is drawn smaller and shown stretched, with filtering, over the
      whole view) and the time a frame may spend on shapes that are not needed at once (4 / 3 / 2 ms, halved in the browser). The
      governor never changes the memory caps (a cap below what is in use makes the next request fail): those come from the adapter and
      the choice only. A frame's time is the processor's part, or the card's when the view drew less than 100 ms before (an idle card
      takes longer to wake than to draw a small frame, which would lower the quality for no reason). Tests: 13 on the profile and the
      governor (steps, spikes, cooldown, ceiling, bad numbers), 6 on the picture (smaller and over the whole view, soft edge, no
      bleeding from the texture's slack, exact at High, shown again when nothing changed, caps follow the choice, a profile by hand
      is never lowered, an automatic view goes down to Low and back up), 19 mutation checks.
- [ ] 5.10 Vertex size: if a tier is limited by vertex bandwidth, test compact normals (16-bit) and a smaller vertex;
      keep only what measures faster.
- [x] 5.11 Picking (parallel track, CPU only): a spatial index for hit testing (a click takes about 7 ms on the stress
      scene today), updated from the change stamps of Phase 1.
      *Done 2026-10-20, as boxes and not a grid.* The cost of a click was flattening the path of every shape (twice for stroked ones),
      so `Scene::hit_test` now keeps the box of every shape's control points (`HitCache` in `scene.rs`, found by node id and good
      for the node's geometry stamp; a copy of a scene starts with none, as undo makes copies) and skips a shape whose box, grown by
      half the stroke width and twice the tolerance, does not hold the point, before any flattening. Skinned shapes (bent by bones),
      paths without points and paths with a point that is not a number take the exact route. 21,000 shapes: median 3.4 ms -> 0.13 ms
      (p95 7.6 -> 0.28); the first click after a load measures all the boxes, 1.2 ms. The walk over the nodes is left (about 6 ns a
      node); a grid would be the next step at 200,000 shapes. Tests: `hit_tests.rs` asks 1,500 random points of random scenes
      (curves, thin and 14 wide strokes, mirrored, skewed and turned groups, clip groups, instances, hidden and locked nodes) the
      same question with and without boxes, after 60 edits of every kind (move, outline edit, hide, replace, delete, copy), and a
      bent skinned shape; 5 of 7 mutation checks caught, the other two are equivalent (a path with no usable points gives no hit
      either way). Debug builds also compare every box they use with a new one.
- [ ] 5.12 True instancing of symbols (the part of 4.5 that was left out). Instances share the CPU-side tessellation, but each has its
      own copy of the vertices on the card, because the shape-table row is a field of every vertex. If a measurement asks for it
      (thousands of instances of one symbol: the many-sprites scene of 6.6, T2 memory), give the vertices a per-instance row: an
      instance-rate vertex buffer of rows, bound at a different offset for each draw (WebGL2 has no first-instance), one call for the
      instances of a symbol whose parts are single-shape or all opaque (D11), document order for the others. Changing the vertex
      format touches every pipeline and the upload code, so only with numbers.
      **Numbers, 2026-10-20 (T1, illustration scene, zoom by 2%):** 105 shapes made again, 148 thousand vertices uploaded, 13 MB sent;
      the list update takes 3 to 5 ms and `store.flush()`, the upload of those copies, 4 to 10 ms; the whole frame 20 ms (59 at worst),
      against 0.01 ms for the plain scenes now. That is one frame each time a zoom crosses a bucket of some instanced art, not each
      frame, so it is left until a sprite-heavy scene or T2 asks for it.
- [ ] 5.13 Re-use the pictures of layers that did not change: a layer is drawn again in every frame today (the editor skips whole
      frames that did not change, but a pan redraws all of them). Key a layer's texture by what is in it (the stamps of its nodes
      and the scale, not its place) and show the same texture again when only its position changed.
      *Started 2026-10-20:* the CPU side no longer walks anything for a move (see 5.3). The card still draws every layer again in every
      frame (276 passes in the illustration scene, 6.8 ms on T1 by the earlier probe). Showing a texture again is only exact for a
      move of whole pixels and for a layer that is not clipped by the view, so it wants T2 numbers before it is built.

**Done when:** the benchmark meets the section 1 table on T1, T2, T3 and T3w, or says exactly what is missing and why.
**Result:** *(fill in)*

### Phase 6: Dynamic content (M)
**Goal:** everything that changes every frame still draws correctly and quickly, on every tier.

- [x] 6.1 Skinned shapes: re-tessellate on the CPU each frame for those shapes only, into their own small chunks (D17).
      Measure the cost per skinned shape on T2 and on the web (single thread).
      *Done 2026-10-23 (the cost on T2 and the web is not measured: P2).* The tessellation cache makes a skinned shape from its outline
      as the bones bend it now (`Skin::deform`, the CPU renderer's `effective_path`) and remembers that bent path with the node's
      geometry stamp: while both are the same its triangles are good, and when a bone moves only the shapes bound to it are tessellated
      again, with a new mesh identity so that the draw list sends the new vertices (and rebuilds the one chunk they are in). No
      special chunks were needed: the draw list already rebuilds only the chunks whose items changed. A skinned shape is never culled
      by its bind pose (it is always made; the draw list's own boxes use the bent outline), its layer, when it needs one, is sized from
      the bent outline, and a skin that does not match its path is drawn in its bind pose, as on the CPU. **Two limits remain, and
      stay in the feature gate** (`Unsupported::Skinned`): a skinned clip mask, and a skinned shape inside the art of a symbol (every
      instance would bend it differently and the cache has one entry per node). The editor now draws the sample's tentacle on the GPU
      with no notice. Not yet done: a moving bone still makes the whole frame (the journal patch only knows shapes), which for a
      scene of 21,000 static shapes with one bone moving costs a full frame (6 ms) and not 0.1 ms; that is for 6.4.
      Tests: `skin_tests.rs` (every pose against the CPU renderer; an animation of 24 frames on one renderer, each equal to a new
      renderer's picture, with only the skinned shape made again; a pose the bind pose cannot reach; opacity and clips; the limits),
      three on the cache, and 9 mutation checks.
- [x] 6.2 Morphing shapes: the same way, only when the pose changes.
      *Done 2026-10-23, with no new code:* a morph is an outline written into the shape in every frame (`apply_pose` stamps the
      geometry only for shapes that have a shape track), which for the renderer is an edited outline: the cache makes that shape
      again, the journal patch sends its vertices and the part of its chunk's index list (see 5.3: 0.08 ms and 12 kB for one outline),
      and a shape whose outline did not change is not touched. Checked by the playback test that adds a morph track
      (`anim_tests`) for what the pose writes, and by the patch tests of 5.3 for what the renderer does with it.
- [x] 6.3 Clip playback in the editor: pose the dynamic shapes only, instead of copying the whole scene.
      *Done 2026-10-23.* Playing a clip used to copy the whole scene in every frame (12 to 14 ms on the big scenes), pose the copy
      and draw it; to the GPU renderer every copy was another scene (a new journal), so every frame was a full walk as well. The
      editor now keeps one copy (`AnimState::preview`) and poses it again in every frame (`update_preview`); the copy is made anew only
      when what it was made from changes (the scene's stamps, the clip and its content, the IK constraints), and the bones of IK chains
      are given back their transforms from the scene before the solver runs again, since it starts from where the bones are. The
      renderer then sees the same scene frame after frame, with a journal of what the pose moved. Tests (`anim_tests.rs`): the
      preview equals a new copy at scrambled times, the same copy is posed (same allocation, same journal) in every frame, edits of
      the scene, the clip (a morph, a fade and a color added and taken away) and the IK constraints make a new copy; 5 of 6 mutation
      checks (the sixth is equivalent: two clips with the same content give the same pose). The sample's own animation is small, so
      the gain shows on a big scene with a clip (the clone was 12 ms of the 22 ms for the stress scene).
- [x] 6.4 The running game view: physics and scripts move nodes every frame; only rows change, no re-tessellation.
      *Done 2026-10-23 for frames of plain shapes and symbol instances.* The journal patch (5.3) now also brings up to date a **group or
      an instance that moved**: the cache is updated for that subtree only (`TessCache::update_node`), the subtree is walked on its own
      (`draw_list::collect_node`, `Collected::as_edits`) and its items get new rows; every one of them must already be in the list
      (otherwise the frame is made whole, as for an instance that has come near, or a group that grew into another bucket while the
      frame's time for shapes was spent). A node that did more than move (opacity, visibility, clip, a bone, a change of overrides) is
      the full walk, and so is a group with a gradient in it. `NodeIndex` covers every node and works out a node's parent from the
      transforms along its path when asked, so a group that moved and a shape edited inside it in the same batch are right. A frame
      with instances is no longer "not flat" (only opacity, clips and layers make a frame not flat). Measured on the `sprites` scene
      (3,000 instances of a symbol of three parts, 1080p), every frame moving some of them: **1 moved 2.9 -> 0.01 ms of preparation, 100
      moved 0.09 ms, 300 moved 0.25 ms**, no triangles made or sent (3,000 moved: 2.9 ms, as before; a patch costs about what a walk
      does when everything changed). Still the full walk: frames with layers, clips or fading (an animated faded group), bones and the
      skinned shapes they move (6.1), gradients, and everything that appears or goes (6.5). Tests: five in `renderer_tests.rs`
      (rows only for a group, an instance, a group inside a group, ten instances and a group; a grown instance gets its own
      triangles; nine kinds of node that must make the frame whole; a randomized sequence of 100 frames x 3 seeds; a bone; a group
      that needs triangles the frame had no time for; the zoom window), and 14 mutation checks. The randomized test compares with an
      edge-aware rule, not bytes: where instances of one symbol are at several sizes, which zoom bucket's triangles each gets depends
      on what the cache did before, and a renderer that patched and one that walked every frame may draw curves with slightly
      different (equally good) triangles.
- [ ] 6.5 Objects created by scripts (`instantiate`): tessellated once on first draw and shared between copies of the
      same template (rows pointing to the same geometry; instancing rules of 4.5).
- [ ] 6.6 An animated regression scene (the sample's tentacle, pulsing sun, rolling ball) and an animated benchmark,
      plus a generated many-sprites scene (thousands of moving instances).
      *Started 2026-10-23:* the generated many-sprites scene exists (`evolve_editor --gen-scene sprites`, 3,000 instances of a symbol
      of three parts, in `bench_scene.rs`; it is also drawn against the CPU renderer by `gpu_parity_tests`), and the probe
      `profile_tests::what_an_animated_frame_costs` moves 3,000, 300, 100 or 1 of them in every frame and reports the animation's
      own time, the preparation and the card's time (see 6.4 for the numbers). Not done: the animated regression scene of the
      sample (tentacle, sun, ball) and an animated mode of `--benchmark`.
- [ ] 6.7 Optional GPU skinning, only if 6.1 is too slow somewhere: bone matrices in the data texture (not in uniforms,
      which are 16 KB on WebGL2), weights per vertex. It bends triangles instead of anchors, so the picture differs from
      the CPU at strong bends: it gets its own looser comparison rule and stays opt-in per shape.

**Done when:** the animated scenes match on both renderers and the animated benchmark meets its target on every tier.
**Result:** *(fill in)*

### Phase 7: Editor integration (M)
**Goal:** the GPU renderer becomes the editor's default without losing any editor behavior.

- [ ] 7.1 Scene and Game views use the GPU texture; egui still draws selection outlines, handles, guides and tool
      overlays on top.
- [ ] 7.2 Thumbnails and PNG export stay on the CPU renderer (the reference).
- [ ] 7.3 Display scale: moving the window between monitors with different scaling, and resizing panels, behave
      correctly (the grow-only targets of 2.6).
- [ ] 7.4 Device lost (driver reset, sleep): first find out what eframe does, since egui's own resources are lost too.
      Then recreate our resources (geometry, rows, ramps, layers) from the scene and carry on. Test by simulating it.
- [ ] 7.5 Fallback and settings: no adapter or no device means the CPU renderer plus a status message; the
      *View → Renderer* choice and the quality profile are remembered.
- [ ] 7.6 Whole MANUAL_TESTS.md checklist (including the big-scene section) passes with the GPU renderer on T1 and T2.
- [ ] 7.7 Flip the default to automatic (GPU when available), when the feature gate of 2.9 reports nothing unsupported
      for the inventory of 0.8 (Q4).

**Done when:** 7.6 passes and the default is flipped.
**Result:** *(fill in)*

### Phase 8: Player, web and desktop platforms (L)
**Goal:** exported games and the web player use the same renderer and meet their targets.

- [ ] 8.1 Desktop player moves from glow to wgpu (eframe's `wgpu` feature), using `evolve_gpu`.
- [ ] 8.2 Web player: `wasm32` build with both the WebGPU and the WebGL2 backends, chosen at run time. Set a size
      budget against today's 6.5 MB (1.7 MB gzipped), measure, trim features; update the `tools/build-web-player`
      scripts and the pinned `wasm-bindgen` version.
- [ ] 8.3 WebGL2 pass in real browsers (Chrome, Firefox, Safari) on the T2 laptop: full regression suite and benchmark.
- [ ] 8.4 WebGPU pass in the browsers that have it, same checks.
- [ ] 8.5 WebGL context loss: handle the browser's "context lost / restored" events like the device loss of 7.4.
- [ ] 8.6 macOS (Metal), Linux (Vulkan, and the GL fallback), Windows (DX12 and Vulkan): run the checks and fix
      platform issues.
- [ ] 8.7 Exported games expose the quality profile and the CPU fallback.

**Done when:** an exported desktop game and the web game render the sample projects and the three benchmark scenes on
the GPU and meet the T1, T2, T3 and T3w targets.
**Result:** *(fill in)*

### Phase 9: Advanced vector features (XL, can be split)
**Goal:** the SVG-style features that the engine wants within about a year, in both renderers (reference rule).
Features the inventory (0.8) found already in the CPU renderer are done earlier, in 3.11 or 4.9, with the techniques
below; this phase covers the rest. Each feature: CPU reference, GPU path, regression scenes in normal and WebGL2-limits
mode, a benchmark entry on T2, and a CHANGELOG line (plus a format bump with a frozen fixture if the data model changes).

- [ ] 9.1 Dashed strokes. Dashes are split into sub-paths on the CPU (with kurbo or lyon) and tessellated like any
      stroke, so every dash gets correct caps and joins and matches the CPU. A change of dash offset (animated "marching
      ants") re-splits only that shape. Optional fast path for butt caps: discard in the fragment shader using the
      distance along the path (D5d), which also gives pixel-unit dashes for pixel-width strokes.
- [ ] 9.2 Blend modes (the SVG / CSS set: multiply, screen, overlay, darken, lighten, color-dodge, color-burn,
      hard-light, soft-light, difference, exclusion, hue, saturation, color, luminosity, plus "plus"/add). Fast path with
      the card's fixed blending where it is exact: normal, plus, screen always; multiply when the backdrop is opaque
      (the main target always is). All other modes: the group goes into a layer, the backdrop region under it is copied,
      and a composite shader reads both. This splits the main pass (D13), so consecutive blended items are grouped.
      Isolation rules as in SVG.
- [ ] 9.3 Masks: luminance and alpha masks. The mask content is drawn into a layer, turned into coverage, and the
      masked group's layer is multiplied by it during composition. Masks inside masks and masks on instances.
- [ ] 9.4 Filters, set A: Gaussian blur, offset, flood, color matrix, composite, merge, blend, drop shadow. A filter is a
      small graph of steps run on effect layers. The filter region grows the layer by the blur margin (3σ). Blur is
      separable (two passes); large radii run on a downsampled copy, with the downsampling set by the quality profile.
- [ ] 9.5 Filter caching: the result of a filter on content that did not change (by its revisions) is kept and reused.
      This is the main speed tool for filters on T2, the web and phones.
- [ ] 9.6 Filters, set B (decide the list from the inventory and user demand): morphology, component transfer,
      turbulence, displacement map, convolve matrix, lighting, image, tile.
- [ ] 9.7 Patterns: the pattern tile is drawn into a cached layer and sampled with repeat in the fragment shader.
- [ ] 9.8 Raster images as a node kind: textured quads, images in an atlas or texture array within the WebGL2 limits,
      mipmaps for zoomed-out images, sampling that matches the CPU renderer.

**Done when:** every feature above (or its replacement list after the inventory) passes its regression scenes and meets
the illustration-scene target with the feature in use on T2 and the web.
**Result:** *(fill in)*

### Phase 10: Text (XL)
**Goal:** text as a first-class vector node, drawn the same way by both renderers.

- [ ] 10.1 Decide the shaping and layout library (D-decision recorded in section 9). Requirements: pure Rust, wasm-ready,
      complex scripts and bidirectional text, font fallback. Check the current state of the candidates at that time
      (for example cosmic-text, parley, or a HarfBuzz port used directly).
- [ ] 10.2 Fonts in the project: storage, licensing note, embedding in exports and in the web build (size budget of 8.2).
- [ ] 10.3 CPU reference: text is shaped, each glyph outline becomes a path, drawn by tiny-skia like any shape.
- [ ] 10.4 GPU, large text: each glyph outline is tessellated once per zoom bucket and cached; glyphs are single-shape
      items, so they can be instanced (rule of 4.5). Text fill, stroke, gradients, opacity and clipping come for free
      from the shape machinery.
- [ ] 10.5 GPU, small text: below a size threshold on screen (decide by measurement, around 24 px), glyphs come from a
      coverage atlas rasterized on the CPU, for legibility and speed; compare with the CPU renderer at those sizes.
- [ ] 10.6 Text and the effect layers: text inside groups, masks, filters, blend modes.
- [ ] 10.7 Regression scenes: Latin, Arabic, a CJK sample, mixed sizes, stroked and gradient text, at three zooms.
- [ ] 10.8 Editing text in the editor is editor work (egui overlays), listed here only as a dependency.

**Done when:** the text regression scenes match and a text-heavy illustration scene meets its target on T2 and the web.
**Result:** *(fill in)*

### Phase 11: Mobile (L)
**Goal:** exported games run on phones and meet the T4 targets.

- [ ] 11.1 Choose the T4 reference devices and record the CPU baseline on them.
- [ ] 11.2 Player shell: check whether eframe supports the target phones well at that time; if not, a thin shell with
      winit and a wgpu surface that uses `evolve_gpu` without the `egui` feature (D1).
- [ ] 11.3 Android: Vulkan, with GLES as the fallback. iOS: Metal.
- [ ] 11.4 Tile-based GPU rules: discarded or transient MSAA attachments (D4), as few passes as possible (D13), layer
      pool cap of the Low profile, filter caching (9.5).
- [ ] 11.5 Heat and battery: frame-rate cap (30 or 60) chosen by the game, idle frames (2.10).
- [ ] 11.6 Full regression suite and benchmark on the T4 devices.

**Done when:** an exported sample game and the illustration scene meet the T4 targets on the reference devices.
**Result:** *(fill in)*

### Phase 12: Beyond the baseline (open-ended)
Ideas, not commitments, all general engine features: a high-quality anti-aliasing mode (analytic coverage, or Vello
revisited, D18), GPU culling and indirect drawing where supported, text on a path, variable fonts, more filter
primitives. Anything that only a particular game needs goes into that game's plan (D9).

## 5. Order and checkpoints

```
Phase 0 -> Phase 1 -> Phase 2  <- checkpoint 2.14: targets per tier with real numbers (T1, T2, web canary)
                         -> Phase 3 -> Phase 4 -> Phase 5  <- checkpoint: section 1 table on T1, T2, T3, T3w
                                                    -> Phase 6 -> Phase 7 (editor default) -> Phase 8 (player, web)
                                                                     -> Phase 9 -> Phase 10 -> Phase 11 -> Phase 12
```
- Phases 3 and 4 can overlap once 2.5 exists.
- 5.11 (picking) is CPU-only and can run in parallel with anything after Phase 1.
- Phase 8 starts after Phase 6, because games need skinning, morphing and spawned objects.
- Phases 9 and 10 can start after Phase 4 (they need the layer system), in parallel with 5 to 8, as long as each
  feature lands in both renderers. Phase 11 needs Phase 8.
- If the inventory (0.8) moves features into 3.11 or 4.9, Phases 3 and 4 grow accordingly; that is expected.

## 6. How each phase is verified

- **Picture tests.** The same scene rendered by the CPU and the GPU renderers and compared. Scenes: the existing
  renderer tests, the sample project, stroke style grids, hairlines, a gradient grid, self-overlapping transparent
  strokes, nested groups, symbol colors and overlapping instances, every feature of Phases 9 and 10, and the benchmark
  scenes at three zooms.
- **The comparison rule (edge-aware).** MSAA and tiny-skia never agree exactly on edges, so the rule separates them.
  An edge mask is built from the CPU image (pixels within 1 pixel of a color change). Interior pixels may differ
  by at most 3/255 per channel; edge pixels by at most 96/255; the average difference over the whole image must stay under
  1/255; no more than 0.1% of interior pixels and **0.5% of edge pixels** may exceed their limit. (Tuned on the first real
  pictures, 2026-10-15: honest 4x MSAA puts a handful of edge pixels at thin curves and sharp corners just past 96, 97 and
  113 were seen, below 0.2% of the edge pixels; a mistake in pixel placement puts nearly all of them past it, a shift of
  half a pixel is 127 on every straight edge, and a test on the card checks that this is caught.) Each rule part has a
  mutation check.
- **WebGL2-limits mode.** Every GPU test also runs with the GL backend and WebGL2 limits where available (0.12).
  Real-browser runs happen at the canary (2.12), at each checkpoint, and in Phase 8.
- **Benchmarks.** `--benchmark` prints CPU and GPU columns for the three views of each scene plus editor costs, per
  tier. Every phase records its results in BENCHMARKS.md, one table per tier, so a regression is visible.
- **Mutation checks.** For tests that guard a risky rule (clipping, order, depth, instancing, culling, the comparison
  itself), temporarily break the rule and confirm the test fails. This caught a weak test when culling was added.
- **Manual checks.** New lines in MANUAL_TESTS.md for what only a person can judge: smoothness, shimmering of thin lines
  while zooming, behavior after sleep and after a WebGL context loss, text legibility.
- **No GPU.** All GPU tests skip cleanly. The CPU path keeps its own tests.
- **Safety rules.** Each rule of GPU_SAFETY.md that can be tested has a test with a mutation check (F4), and a phase
  that adds GPU work checks its work against the file before it is closed.
- **Every phase ends with** `cargo clippy --workspace --all-targets` clean, `cargo test --workspace` green (normal and
  WebGL2-limits mode), the benchmark re-run on the tiers named in the phase, and a dated entry in the progress log.

## 7. Risks and unknowns

| risk | what we do about it |
|---|---|
| Thin lines look dim, broken or shimmering | Hairline rule (D5c) from Phase 2, regression scene 3.1, manual check |
| MSAA edges differ from tiny-skia | Edge-aware comparison rule (0.11); high-quality mode in Phase 12 |
| WebGL2 is slow or differs on some drivers | Baseline limited to WebGL2 features (D8), WebGL2-limits tests, web canary from Phase 2 |
| Integrated GPUs and phones are fill-rate or bandwidth limited | Discarded MSAA attachments, few passes, front-to-back pass, quality profiles, filter caching |
| Layer count and memory blow up in nested scenes | Shortcuts that skip layers (4.4), pool cap, reduced resolution |
| Change tracking touches many files | Audit first (1.1); one system for rendering, undo and picking |
| Large coordinates lose precision at deep zoom | Floating origin and a deep-zoom test (5.5) |
| Colors look washed out or too dark in egui | Color handshake test (2.7), own final quad if needed |
| Blend modes split the main pass and cost speed | Fixed-function fast paths; group consecutive blended items (9.2) |
| Filters are expensive on weak hardware | Downsampled blur, filter caching, profile-dependent quality (9.4, 9.5) |
| Text is a large subsystem | Separate phase, library decision first, CPU reference first |
| The web build gets much larger (wgpu with two backends, text, fonts) | Size budget and measurement in 8.2 and 10.2 |
| Software GPUs in CI are slow or differ in details | Compare with a tolerance; keep timing tests local, not in CI |
| wgpu changes its API between versions | Pin with eframe (30.0.1 now); upgrade together, as for egui |
| eframe's support on phones is limited | Core of `evolve_gpu` has no egui dependency (D1); thin shell if needed (11.2) |
| Faster on large scenes but slower on tiny ones | Benchmark both; idle frames cost nothing; CPU renderer stays selectable |

## 8. Open questions

*Questions that need the owner or a machine are also tracked, with what is needed and what they block, in
[OPEN_POINTS.md](OPEN_POINTS.md) (P1 to P8). Add new ones there.*

| # | question | status |
|---|---|---|
| Q1 | Are the per-tier targets of section 1 right, and which machines are T2 and T4? | proposed; T2 chosen in 0.10, T4 in 11.1; numbers confirmed at 2.14 and after Phase 5 |
| Q2 | Is a new project format version acceptable for pixel-width strokes (D5b)? | open; the renderer supports it from Phase 2 at no cost, only the data-model flag waits (3.10) |
| Q3 | How much does the web version matter? | **answered 2026-10-15:** it must run well on WebGL2 too; D8 is a hard constraint |
| Q4 | Flip the default when the feature gate is empty (end of Phase 7), or only after Phase 8? | proposed: end of Phase 7; decide at 7.7 |
| Q5 | Which shaping and layout library for text? | decide in 10.1 |
| Q6 | Which filter primitives beyond set A are needed in the first year? | decide at 9.6 from the inventory and user demand |
| Q7 | Do radial gradients have a focal point, and which spread modes does the data model have? | **answered 2026-10-15 (0.8):** no focal point, pad only |
| Q8 | The CPU renderer's opacity and clip layers are full-screen (about 8 ms each). Make them as big as the node's bounds? | open; optional. It would make the reference renderer, the thumbnails, PNG export and the fallback much faster on scenes like the illustration scene, and tests quicker. A task of its own in `evolve_engine`, not required by the GPU work |

## 9. Decision log

- **2026-10-15** Backend: wgpu (OpenGL rejected: deprecated on macOS and iOS, fewer features; the editor already runs
  on wgpu). Plan: lyon tessellation, own offscreen target, CPU renderer kept as reference and fallback.
- **2026-10-15** Scope rule D9 added: the plan is about the engine and editor; IronKrieg is a test scene only.
- **2026-10-15** The edge rule of the comparison helper was tuned on the first real GPU pictures (0.5% of edge pixels may
  exceed 96/255); GPU tests run serially on a shared device, see the progress log.
- **2026-10-15** Pixel-width strokes demoted from "the map needs it" to an optional general feature.
- **2026-10-15** Revision after review. Changes and reasons:
  - Owner answers: the web must run well on WebGL2 (Q3 closed); targets apply to every hardware tier (tiers T1 to T4
    added); text, dashes, filters, blend modes and masks are wanted within about a year (Phases 9 and 10 added).
  - Change tracking moved to Phase 1, before the renderer: the GPU cache cannot work without it.
  - Shape table (D10) and chunked index lists (D17) replace per-shape draw calls and the static/dynamic split: moving or
    recoloring a shape rewrites one row; uniform buffers (16 KB on WebGL2) could not hold per-shape data.
  - One stroke method with the width applied in the shader (D5): merges world and pixel widths and gives the hairline
    rule, needed to match tiny-skia on the stress scene.
  - Depth values per draw item (D11): fixes darker spots where transparent strokes overlap themselves, and makes
    instancing of opaque multi-part symbols safe.
  - Stencil clipping is the main clip method (D12): with MSAA the stencil is per sample, so it is anti-aliased; the
    previous plan wrongly called it "hard edge".
  - Effect layers as one mechanism, with prepasses (D13): prepares filters, masks and blend modes; few passes for
    WebGL2 and tile-based GPUs. Layers are never silently skipped when the pool is full.
  - Color space decided in Phase 2 instead of Phase 3 (D14): the target format depends on it.
  - Edge-aware comparison rule: the previous rule (0.5% of pixels beyond 24/255) cannot pass with MSAA on stroke-dense
    scenes.
  - Memory budget split into content and targets: the previous "under 100 MB including layers" was impossible (MSAA
    targets alone are about 75 MB at 1080p, about 300 MB at 4K).
  - Zoom buckets apply to round joins and caps too, using the effective scale (the probe showed it).
  - Instancing restricted to single-shape or fully opaque symbols (painter's order).
  - Desktop player moved after dynamic content (games need skinning and spawned objects).
  - Illustration benchmark scene added: the map scenes do not represent SVG-style content.
  - Vello recorded as the considered alternative (D18).
  - Phases renumbered; see appendix A.
- **2026-10-15** After the inventory (0.8): tasks 3.6 and 4.4 corrected. The earlier text let opacity on a leaf shape go
  into its row; but the CPU fades a shape's fill and stroke as one whole, so that is only exact when the shape has one
  of them. Tasks 3.11 and 4.9 closed as empty. Q8 added (the CPU renderer's full-screen layers).
- **2026-10-16** Culling pulled forward from 5.7: the first hands-on try showed that making every shape at every zoom goes over
  any budget and costs the frame time of a CPU render. Done as a view-based test with the extent of each shape cached by its
  geometry stamp; the rest of 5.7 stays in Phase 5.
- **2026-10-16** Geometry budget of the High profile 192 MB to 512 MB (alloc.rs, GPU_SAFETY.md): the real map needs about
  150 MB steady and a change of zoom needs room beside it. A budget exists to stop runaway use, not to be tight; the
  per-frame and per-view limits remain. Raised by the assistant on measurement; the owner can overrule it (P11).
- **2026-10-17** The shape table row is 8 texels, not 4 (D10 said 4): the gradient's geometry is per draw item, and a second small
  table of gradient data would have needed another binding, another generation to track and an indirection in the shader.
  The table costs 128 bytes per item instead of 64 (2.7 MB for the map) and holds 128 rows per line: 262,000 items on WebGL2.
- **2026-10-17** The ramp atlas is a plain 2D texture, not a texture array (D16 said array): WebGL2 allows only 256 array layers,
  and 2,048 rows of one texture is more room. Beyond it the view uses the CPU renderer.
- **2026-10-17** Miter joins are no longer left to lyon where the limit is exceeded (see the progress log): lyon's limit is twice
  SVG's and it clips instead of bevelling. Only that rule is replaced; ordinary corners and every other join are still lyon's.
- **2026-10-17** Strokes are flattened at 0.35 of the fill tolerance after a picture test showed edges up to 0.75 px off.
- **2026-10-16** Phase 4 design (before the code):
  - **A frame is a list of passes, a pass is a list of ops, and ops are grouped into chunks.** The main pass is last; every layer has
    a pass of its own, inner layers before outer ones. An op is an item (a fill or stroke), a mask push, a mask pop, or a layer
    composite. Chunks (one draw call each) hold up to 512 consecutive items at one clip level; a mask or a composite is a chunk
    of its own. Everything stays keyed by (node, part), rows and vertices are kept as before, so editing one shape still
    rebuilds one chunk.
  - **Clips (4.1)** draw the mask's fill into the stencil with the colour write mask off and depth untouched (compare "always",
    no write), incrementing where the current level passes; content is tested "equal" to the level; the mask is drawn again
    decrementing at the end. With 4x MSAA the stencil is per sample, which equals the CPU's "layer through an anti-aliased
    mask" (the sum over samples of the covered ones). The mask counts only as a fill silhouette (its own colours, stroke, opacity
    and visibility do not), so the tessellation cache makes a fill mesh for it whatever its paints are, and the mask is
    culled by its own box: a mask that cannot reach the view skips the whole group, as on the CPU.
  - **Layers (4.2 and 4.3)** are drawn in prepasses into resolve textures of a pool keyed by size class; the MSAA colour and
    depth-stencil of a pass are scratch targets shared by every layer of the same class (they are discarded after the pass),
    so a live layer costs 4 bytes per pixel, not 36. A layer is the node's box on the screen, clipped to the view. The view
    uniform has one 256-byte slot per pass (dynamic offset), because the layer's picture is the view shifted by its corner.
    A composite is an ordinary item (a quad of 4 vertices with a row) drawn by a second pipeline whose fragment shader loads
    the layer's texel at the pixel's position (no sampling, no blur) and multiplies by the opacity.
  - **Opacity (4.4)** goes down into the leaves (no layer) when the group has one child, or when its children's boxes (strokes
    and one pixel of anti-aliasing included) do not touch each other; otherwise the group gets a layer.
- **2026-10-16** Phase 4, what changed from the design above while building it, and why:
  - **Instancing is shared tessellation, not one draw call** (4.5; the rest is 5.12). The shape-table row is a field of every vertex
    (D10, D17), so two instances cannot share vertices on the card; sharing needs a per-instance row in an instance-rate buffer, and
    WebGL2 has no first-instance, so every call would bind it at its own offset. That changes the vertex format of every pipeline and
    wants numbers first. What is shared is the expensive part, the CPU-side tessellation (one entry per node and bucket, a second
    one when instances of different sizes need another bucket in the same frame).
  - **A key has a scope**: (node, part, scope), where the scope numbers the chain of instances an item is in (0 for the scene).
    The numbers come from a table that keeps them from frame to frame and forgets instances that were not seen.
  - **Items carry their own triangles** (a reference to the cache's mesh and what it was made from), instead of the draw list asking the
    cache by key four times a frame; chunk signatures include what the triangles were made from, so a re-made shape whose vertices
    land in the same place can never keep an old index list.
  - **Reduced resolution** (4.2) is a filtered stretch: the layer is drawn at 1/2 or 1/4 with the view scaled down, shown by a
    fragment path that samples with a linear filter. Layers are acquired before the draw list is updated, because the composite rows
    depend on how big each layer turned out.
  - **Scratch targets are counted as `Targets`, not `Layers`** (the layer budget of 64 MB is for the pictures that stay: 4 bytes a
    pixel; one full-HD layer with its scratch would be 75 MB). And **at most 2,048 layers a frame** (each is a pass): beyond that
    the view uses the CPU renderer.
  - `Unsupported` has one variant left, skinned shapes. A mask is made as a fill-only mesh whatever its paints are, so the cache
    entry says which it is (`mask`), and a node changing role never reuses the wrong one.
- **2026-10-15** WebGL2-limits mode is a device mode of `evolve_gpu` (GL backend + WebGL2 default limits), not a cargo
  feature, so every test runs in it without extra commands; it is skipped where no GL adapter exists.

## 10. Progress log

- **2026-10-16** Phase 4 (groups and effect layers), in this order:
  1. **Clips (4.1).** Stencil clipping on the existing pass: two more pipelines (push and pop of a mask, no color, no depth), the
     cache makes fill-only meshes for masks and culls a whole group by its mask's box, the draw list became passes of ops.
     Anti-aliased edges match tiny-skia because the 4x stencil is per sample.
  2. **Layers (4.2 to 4.4).** A layer pool by size class, a pass per layer with its own view uniform, composites drawn by a second
     fragment entry, boxes from control points and stroke reach, the two shortcuts (one child; children that cannot touch).
     Found by the tests on the way: a faded group of overlapping children needs a layer, and a shape with a see-through fill *and*
     stroke needs its own (the earlier gate had said so already); an expectation of mine that the circle in a "no layer" group
     needed none was wrong the other way and the test said so.
  3. **Instances (4.5).** Scopes, shared tessellation with a second cache slot, colour overrides, per-instance culling, a depth limit.
     Removed the gate's last features: `Unsupported` is just skinned shapes now.
  4. **Reduced resolution and limits (4.2, 4.6).** Layers drawn at 1/2 or 1/4 when the memory is full, at most 2,048 layers a frame.
     The first tests of the reduced path were too loose to see a misplaced composite (a mutation check showed it); they now
     use rules at about twice the measured difference.
  Also: the whole-project parity tests draw everything but the tentacle; `FrameStats` got `passes`, `layers`, `layers_reduced`,
  `layer_pixels` and the stage times. 37 new tests and 38 mutation checks (all caught). The complete illustration scene takes about
  17 ms on T1 (9.9 CPU, 6.8 card) against the CPU renderer's 2.8 s; the whole-map pan got about 1.5 ms slower (BENCHMARKS.md).
  Open: 4.8 on T2 and in the browser; MANUAL_TESTS.md "After Phase 4" for the owner.
- **2026-10-16** Two bugs from the owner's hands-on try after Phase 3, fixed before going further:
  1. **Province fills vanished while zooming** (the border stayed, and the province could still be clicked). The geometry
     store joined staged writes that were up to 4 KB apart into one write and filled the gap with zeros, taking the gap for
     the padding of a size class. That was true on a first frame, where everything is packed in order, but once culling had
     shapes leave and come back they took holes on both sides of shapes already on the card, and those were zeroed: every
     vertex of such a shape fell on one point. Writes are now joined only when one block starts where the previous one ends.
     Found with a new tool that zooms into the map with one renderer and compares every step with a fresh one
     (`profile_tests`, ignored, needs the map): steps that differed by 77,000 to 102,000 pixels now differ by at most 231,
     all from the 10% bucket hysteresis (with it switched off: at most 5). Two new GPU tests (shapes that come back between
     shapes that stayed; every frame of a zoom and pan against a fresh renderer) and a store test; mutation check caught.
  2. **Selecting provinces drew long straight lines across the whole view.** Not the renderer: egui joins the corners of a
     *closed* line with miters of any length (epaint's `add_line_loop` does not cut sharp corners), and real outlines turn
     back on themselves or have points a fraction of a pixel apart. Outlines (selection, colliders) now drop points closer
     than half a pixel and are drawn as open lines back to the start, whose sharp corners egui cuts. Test with egui's own
     tessellator; three mutation checks caught.
- **2026-10-16** Phase 3 (shape parity), in this order:
  1. **3.12, and the miter limit.** Fixing the exact 180 degree reversal showed that the problem was the miter limit in general:
     lyon counts the limit as twice what the SVG rule and tiny-skia say (it compares half the miter's length) and *clips* a
     miter that is too long where tiny-skia cuts it flat (a bevel). So a sharp corner just past the limit (23 degrees at a limit
     of 4) was a spike on the GPU and a bevel on the CPU, in any scene with sharp corners. The tessellator now decides corners
     itself with Skia's own test (`sin(half the angle) < 1 / limit`), cuts the stroke into pieces at the corners that are cut
     flat (flat ends, whatever the cap) and adds the bevel's triangle itself; lyon only strokes what is left, with
     ordinary miters. 4 tests and 9 mutation checks (one equivalent: bevel joins cut too, which looks the same).
  2. **3.2.** Bucket hysteresis (10%) and strokes flattened finer than fills (factor 0.35); see 3.2 above. 6 mutation checks.
  3. **3.3 to 3.5, gradients.** Shape table rows of 8 texels, `ramps.rs`, the gradient part of the shader, paints decided by
     tiny-skia, per-gradient stops clamped to 256. 9 picture tests (linear, radial, transforms on the gradient and the shape,
     strokes, thin lines, a moved view, degenerate gradients, 90 different gradients so that the atlas grows, a steep ramp
     that shows half-texel mistakes). 20 mutation checks; one remains uncaught on purpose-less grounds (baking over 255 texels
     instead of 256 moves colors by less than the natural difference to tiny-skia, 2 levels).
  4. **3.6, opacity on a single shape** with the same-depth trick from the review (stroke first, fill at the same depth, only
     when both paints are opaque); the feature gate now says node by node what it leaves out. 9 mutation checks, among them
     the two the plan asked for (different depth values; same depth with the fill first).
  5. **3.9, the regression suite** (see the task) and a stroke style grid.
  6. A faster hash (`fast_map.rs`) for the per-frame lookups took about 3 ms off the whole-map frame, which had grown by
     the extra passes of the draw list.
  Benchmarks of the map after Phase 3 are in BENCHMARKS.md.
- **2026-10-17** Task 2.13 on T1: the real map, the stress scene and the illustration scene (timing-only mode), three views each,
  CPU and GPU side by side (BENCHMARKS.md). The GPU renderer is 9 to 14 times faster than the CPU renderer on the map at every
  zoom (7.7, 3.4 and 1.3 ms against 67, 38 and 10 ms); on the illustration scene the comparison is not fair yet (the GPU leaves out gradients, opacity, clip groups and symbols); the whole-map frame is
  still 6 to 7 ms of CPU bookkeeping (Phase 5). The run took about 15 seconds of GPU time in all; no process left running.
- **2026-10-17** Task 2.12 (web canary). `evolve_gpu` now compiles for `wasm32-unknown-unknown` (clock from `web-time`;
  the blocking helpers for tests and tools are native only), and `tools/web-canary` is a page that opens a WebGL2 or WebGPU
  device on a canvas, loads a project file, draws three views for a fixed number of frames each, shows the picture, and
  reports adapter, limits, frame times (CPU time of `render` and the time between animation frames), memory and any error the
  graphics layer reported. `tools/build-web-canary.ps1` builds it and the two generated scenes into `dist/web-canary`. Result
  on T1 (headless Edge 155): no errors in any of the three configurations; the picture is correct; steady frames on the real
  card are 7 ms of CPU for the whole stress scene (WebGL2 or WebGPU), but the **first frame costs 1.5 s** (tessellating 21,000
  shapes in wasm, 8 times slower than native). CI checks that both crates keep compiling for the browser. Nothing in the
  renderer had to change except the clock and the native-only helpers: the WebGL2-limits test mode did its job.
- **2026-10-16** Owner's first hands-on try of Phase 2 (map open, GPU renderer): scene mode and play mode lagged, zooming and
  dragging lagged a lot, resizing showed white parts. The numbers of the benchmark (7 ms a frame) described only the renderer,
  so the real editor frame was timed panel by panel (a temporary profile of the real panes, now `profile_tests.rs`, ignored).
  Findings and fixes:
  1. **The editor's own overlays cost 420 ms a frame with the map**, whatever the renderer: `bone_ids` looked every node up
     by its id (quadratic). Fixed with one walk of the tree (`Scene::for_each_node`); the same pattern was removed from
     the physics step, `stats`, `debug_bodies`, `main_camera`, `deletable` and script registration. Play mode with the map:
     2,160 ms a frame before, 0.5 ms after; starting a game 1,360 ms before, 25 ms after. Tests: bone order, a
     speed guard (21,000 nodes), walk order equals `all_ids` on random trees.
  2. **Zooming made the view fall back to the CPU renderer** (480 ms a frame): at a new zoom bucket the geometry of every
     shape was made again while the old one was still held, and the buffers doubled next to each other, over the 192 MB
     geometry budget. The notice at the bottom left of the view said so, but it is small. Fixed by (a) **view culling**
     (see 5.7), (b) the draw list **giving back what is gone or out of date before asking for anything new** and packing a
     buffer that is mostly holes before growing it, (c) `GrowableBuffer::reserve`: room for everything that is coming in one
     step, just large enough, instead of doubling (the first frame of the map made buffers up to twice too large), (d) the
     geometry budget of the High profile raised from 192 MB to 512 MB (the real map needs about 150 MB; see P11), (e) an
     item whose new vertices could not be had is marked to be made again, so a failed frame no longer leaves ranges
     that were given back in use (found while writing the tests). About 25 new tests, 21 mutation checks all caught (the
     first rounds missed several, which led to better tests).
  3. Measured after the fixes (map, RTX 3060, headless panes, release): pan 6 ms, zoom 21 ms on average with a 130 ms hitch
     where the zoom crosses a bucket and every visible shape is tessellated again (this is B3 and 5.4's budget per frame,
     not done yet), dragging a province 29 ms, play mode 0.5 ms. These are the editor's work only; the card and the window
     system come on top.
  4. New in the editor: the **frame times in the status bar** (fps for frames that follow each other, average and worst
     work of the last 120 frames, hover for where the time went), the **renderer box in the status bar**, and in play mode
     the **right mouse button moves the view** (it rotates while editing).
  Not looked at yet: the white parts while resizing the window (to be judged again now that frames are short).
- **2026-10-13** Benchmark harness and importer built; baseline recorded (BENCHMARKS.md).
- **2026-10-13** CPU renderer culling; compact project file.
- **2026-10-15** Editor costs fixed (Objects panel, selection); lyon tessellation probe measured; plan written.
- **2026-10-15** Plan reorganized into a checklist.
- **2026-10-15** Phase 0 built (0.1 to 0.6): `evolve_gpu` crate, headless device, readback, picture comparison,
  `--gen-scene`, benchmark prepared.
- **2026-10-15** Plan revised after review (this version): tiers, new Phases 1, 9, 10, 11; tasks 0.8 to 0.12 added.
- **2026-10-15** Phase 0 continued: 0.8 inventory (section 2), 0.9 illustration scene and its CPU baseline, 0.11 edge-aware
  comparison, 0.12 WebGL2-limits mode; CI probe job prepared (0.7); clippy clean, `cargo test --workspace` green.
- **2026-10-15** Before Phase 2: `EVOLVE_REQUIRE_GPU` and the `gpu-required` CI job (a place where GPU tests cannot skip),
  `local/` for big generated files (P7 closed), hands-on tests of the editing panels with simulated input
  (`edit_tests.rs`), SVG import behaviour pinned (P10).
- **2026-10-15** Phase 2, step 1 (foundation): `evolve_gpu::alloc` (tracked allocator with budgets and error scopes, rule C1
  enforced by a test over the crate, editor and player sources), `evolve_gpu::errors` (uncaptured errors and device loss
  recorded instead of panicking; every GPU test fails if the card reported an error), `--benchmark` time limit with
  `--allow-long` and a total running time in the report, CI timeouts (1 minute for GPU test runs, job limits).
- **2026-10-15** Phase 2, step 4 (into the editor): the picture shown by our own paint callback and shader (color handshake
  tested on sRGB and non-sRGB window formats, and against egui's image path), `GpuView` with idle frames from the Phase 1
  stamps, the feature gate with its notice, `--renderer auto|cpu|gpu` and *View → Renderer*, explicit vsync, fallback to the CPU
  renderer on device loss, card error or budget, the debug watchdog, the Scene and Game panes in the idle-frame test, 24
  more tests, mutation checks (glue 8 of 8, editor 7 of 7 runnable). The real editor was started once with `--renderer gpu`
  and ran for 8 seconds without a card error. **Stopped here for the owner's hands-on try** (MANUAL_TESTS.md, "The GPU
  renderer in the editor").
- **2026-10-15** Phase 2, step 3 (first pictures, headless): shape table, draw list with chunks, shader and pipeline, targets
  with slack, the renderer (`prepare`, `record`, `render`). Picture tests against tiny-skia on both device modes (fills,
  strokes with every join and cap, painter's order, alpha, self-overlapping strokes, transforms and non-uniform scale,
  zooms and a rotated view, the camera frame, fill rules, hairlines), renderer behaviour tests (an idle frame uploads
  nothing, one edit rebuilds one chunk, a move costs one table line, unsupported scenes are refused, no memory leaks over
  many frames and resizes), 12 more mutation checks all caught. GPU tests now run one at a time on one shared device per
  mode (parallel devices hung the OpenGL backend once), and every test is checked for leaked graphics memory.
  `--benchmark` fills the GPU columns. First numbers: BENCHMARKS.md.
  Found by the tests on the way: thousands of tiny buffer writes (one per draw item) made the OpenGL backend take 2.3 s
  in `submit` and time out now and then, and they were the cause of an intermittent hang of the test suite; vertex and
  index data is now staged and sent in a few merged writes (the OpenGL frame went from 2.6 s to 30 ms). Matters for the web.
- **2026-10-15** Phase 2, step 2 (geometry on the CPU side): 24-byte vertex, range sub-allocator, geometry store with
  growing and compaction, lyon tessellation of fills and strokes with scale buckets and clamps, tessellation cache driven
  by the Phase 1 stamps (test: editing one shape tessellates only that shape). 16 mutation checks, all caught.
- **2026-10-15** Phase 1 built (1.1 to 1.5): `evolve_engine::revision` (stamps, five kinds, `NodeMut` guard), every
  mutation path converted and audited, undo check 3.6 ms to 0.0 ms, contributor rules in CONTRIBUTING.md.

## Appendix A. Old to new numbering

| old | new |
|---|---|
| Phase 0 | Phase 0 (tasks 0.1 to 0.7 unchanged; 0.8 to 0.12 new) |
| — | Phase 1: Change tracking (was 4.2) |
| Phase 1: Minimal GPU renderer | Phase 2 |
| Phase 2: Shape parity | Phase 3 (old 2.5 color rules moved to D14 and 2.7) |
| Phase 3: Groups | Phase 4 |
| Phase 4: Speed at scale | Phase 5 (old 4.2 to Phase 1; old 4.5 basic zoom buckets to 3.2; old 4.6 pixel-width strokes to 3.10; old 4.1 replaced by D10 and D17) |
| Phase 5: Dynamic content | Phase 6 |
| Phase 6: Editor integration | Phase 7 (old 6.3 resize moved to 2.6) |
| Phase 7: Player and platforms | Phase 8 (old 7.5 mobile note became Phase 11) |
| Phase 8: Beyond the baseline | Phases 9, 10 and 12 |
