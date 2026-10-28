# Benchmarks

Baseline numbers for the **CPU renderer** (tiny-skia) on a large imported map (21 440 objects), measured on 2026-10-13. Later renderer
work (see TODO.md, "Phase 1: engine and vector renderer") is compared against these.

## How to reproduce

```text
cargo build --release -p evolve_editor
mkdir -p local/bench                                              # local/ is in .gitignore: big generated files go there
target/release/evolve_editor --gen-scene large local/bench/large.evolve    # also: illustration, sprites
target/release/evolve_editor --benchmark local/bench/large.evolve local/bench/report   # writes report.json and .txt
```

The first tables were measured on a large imported map (21 440 objects, 679 000 points) that is not part of this repository; the
generated scenes below (`large`, `illustration`, `sprites`) are the ones anyone can reproduce.
Do not commit generated projects (`local/` is in `.gitignore`).

`--benchmark` renders three views headless at 1920 x 1080 (1 warm-up frame, then up to 10 frames): the whole map, a
region with 10% of the objects visible, and a close-up with 0.5% visible. The zoom is searched until that share is
visible. The report also has empty fields (`gpu_ms`, `vram_mb`, `draw_calls`) for a GPU renderer to fill in.

## Baseline (CPU renderer, 1920 x 1080)

| | one object per arc | one object per province pair |
|---|---|---|
| objects / anchor points | 21 440 / 678 937 | 4 877 / 678 937 |
| file size, load (read + parse) | 382 MB, 459 ms | 361 MB, 405 ms |
| RAM after load | 44 MB | 36 MB |
| render, whole map (100% visible) | 73 ms | 65 ms |
| render, region (10% visible) | 75 ms | 65 ms |
| render, close-up (0.5% visible) | 63 ms | 46 ms |
| bitmap per frame / at 60 fps | 8.3 MB / 498 MB/s | same |
| clone the scene (clip preview, only while a clip plays) | 14.2 ms | 10.1 ms |
| undo check with nothing changed (every frame) | 3.6 ms | 2.7 ms |
| object under a click | 7.0 ms | 6.2 ms |
| find an object by id | 6 us | 2 us |

## After step 1: culling (2026-10-13)

Shapes (and opacity layers and clip masks) that cannot reach the target are skipped. A test checks that the picture
is pixel-identical with culling on and off, with strokes, miter spikes, rotation, opacity and clip groups just
outside the edges, and fails if the stroke margin is removed.

| render, 1920 x 1080 | arc: before | arc: after | pair: before | pair: after |
|---|---|---|---|---|
| whole map (100% visible) | 73 ms | 72 ms | 65 ms | 58 ms |
| region (10% visible) | 75 ms | 36 ms | 65 ms | 19 ms |
| close-up (0.5% visible) | 63 ms | 10 ms | 46 ms | 5 ms |

What is left in the close-up is mostly the bounds test, which looks at every anchor (about 3 ms for the map).

Where the 72 ms of the whole-map view goes (21 440 objects, 679 000 points, measured separately):

| | time |
|---|---|
| converting paths to tiny-skia paths | 8.6 ms |
| rasterizing the 1 234 filled provinces | 27.7 ms |
| rasterizing the 20 206 border strokes | 33.3 ms |
| bounds tests (culling) | 2.8 ms |

So **caching the converted paths could save at most 12%** and is not worth the invalidation risk. Dropping points
closer than 0.5 px (1 px) before drawing keeps 45% (29%) of the points and saves about 28% (40%) at this zoom, at the
price of a slightly different picture. The whole-map view is therefore near the floor of what this CPU renderer
does (about 14 fps at 1080p) without threads or a GPU.

## After step 2: compact project file (2026-10-13)

Projects over 1 MB are written on one line (same JSON, no format version change). One object per arc:

| | before | after |
|---|---|---|
| file size | 382 MB | 57.7 MB |
| load (read + parse) | 459 ms | 119 ms |
| write to text (save, autosave) | 531 ms | 85 ms |

Not done, on purpose: every anchor in the map has zero handles, so a new format version that omits them would save
about another half (about 25 MB). Compression (gzip) would bring it to 4.4 MB but makes saving slower, which is the
wrong direction for autosave; it would suit exporting or sharing a project. The autosave still writes on the UI
thread (85 ms every 30 s).

## The generated scene (2026-10-15)

`evolve_editor --gen-scene large <out.evolve>` writes a deterministic scene that needs no game files: 1 200 filled
shapes of 350 points and 20 000 polylines of 12 points (21 200 shapes, 660 000 points, 57 MB). CPU renderer at 1080p:
whole view 90 ms (p95 102), region 34 ms, close-up 13 ms. Editor costs: load 116 ms, scene clone 10.6 ms, undo check
3.6 ms, Objects panel 0.4 ms, scene walk with world matrices 0.07 ms. The graphics card of the development machine is
an NVIDIA GeForce RTX 3060 (Vulkan). This is the engine's own large benchmark scene; the IronKrieg map above is the
real-world second one.

## The illustration scene (2026-10-15)

`evolve_editor --gen-scene illustration <out.evolve>` writes the second generated scene, made of what an SVG-first
engine draws every day (a landscape, fixed seed): 4 976 shapes and 3 300 symbol instances (8 276 draw items, 5.3 MB),
cubic curves, 244 linear and 402 radial gradient paints, 400 compound even-odd shapes, miter, round and bevel joins
with widths from 0.4 to 30, 35% of the items semi-transparent, 76 groups with opacity (60 of them nested inside
another opacity group), 30 clip groups, and 1 055 instances with color overrides. Tests check each of these.

CPU renderer at 1920 x 1080 (the 4 000 x 2 250 world scaled to the screen; the three views are chosen by the
benchmark as for the other scenes):

| | time |
|---|---|
| whole scene (100% of the shapes visible) | **2 788 ms** (about 0.4 fps) |
| region (10% visible) | 1 930 ms |
| close-up (0.5% visible) | 1 873 ms |
| load, scene clone, undo check, click | 12 ms, 1.6 ms, 0.2 ms, 0.2 ms |

The close-up costs almost as much as the whole scene, so this is not geometry: it is the **full-screen layers**.
A node with opacity below 1 is drawn into a bitmap of the whole target and then composited (`render.rs`,
`draw_node`; the same for a clip group), and instances are never skipped by culling. Measured by switching features
off in a throwaway run (whole view, release build):

| | time |
|---|---|
| as is | 2 816 ms |
| every opacity set to 1 | 528 ms |
| every clip group turned into a plain group | 2 637 ms |
| neither | 347 ms |

So about 280 opacity groups (clouds, half of the panels, a third of the gems, the two groups the clouds sit in) cost
about 2.3 s, roughly 8 ms each, and clip groups about 0.2 s. Even without them the scene takes 347 ms (3 fps), from
the 8 000 overlapping gradient and stroke items. This replaces the earlier "code reading, not measured" note below:
**opacity groups are the CPU renderer's worst case, and the GPU renderer must not copy that cost** (plan D13, tasks
4.2 to 4.4).

## Change tracking: the undo check (2026-10-15, plan Phase 1)

The editor used to compare the whole scene with the last undo state on every repaint (`History::update`). It now asks
the scene's change stamps first, and only compares for real when they differ.

| undo check, nothing changed | before | after |
|---|---|---|
| large generated scene (21 200 shapes, 660 000 points) | 3.6 ms per frame | **0.0 ms** |
| illustration scene (4 976 shapes, 3 300 instances) | 0.2 ms | 0.0 ms |

Re-measured on the real map afterwards (`local/ironkrieg/map.evolve`): undo check 0.0 ms. Not caused by this change,
noticed while measuring: deleting or grouping a big selection in the *generated* large scene takes 0.4 s at 2 000
selected shapes and 1.3 s at 4 000, the same with the stamping switched off; on the real map it is 0.1 to 0.3 s for
2 000 to 4 000 shapes (see OPEN_POINTS.md, P9).

## The GPU renderer, first numbers (2026-10-15, plan Phase 2 step 3, tier T1)

Machine: NVIDIA GeForce RTX 3060 (Vulkan). `evolve_editor --benchmark` now draws each view with the GPU renderer as well:
one cold frame (tessellation, uploads, drawing), then 10 steady frames, each timed from submit until the card is done.
Headless, 1920 x 1080, 4x MSAA. The GPU time is wall time with the CPU waiting (the web has no timestamp queries, so none
are used here). The **stress scene** is `--gen-scene large` (21 200 shapes, 660 000 points, polylines with round joins):

| view | CPU renderer | GPU cold frame | GPU steady frame (median) | of which CPU update | draw calls | triangles |
|---|---|---|---|---|---|---|
| whole scene | 109 to 113 ms | 180 to 204 ms | **7.3 to 8.3 ms** | 5.6 to 5.7 ms | 42 | 1.06 M |
| region (10% visible) | 44 to 45 ms | 182 to 197 ms | 8.9 to 14.9 ms | 6.0 to 10.4 ms | 42 | 1.12 M |
| close-up (0.5% visible) | 24 ms | 302 to 351 ms | 11.1 to 17.0 ms | 10.1 to 10.8 ms | 42 | 1.22 M |

(Two runs, the second after batching the buffer uploads; the ranges are the spread between them. This machine is not quiet:
the GPU frame times follow the CPU update time, so treat the spread as noise of about 50%. Timing tests stay local.)

- The GPU draws the whole scene about 14 times faster than the CPU renderer (7.3 to 8.3 against 109 to 113 ms). In the
  close-up the CPU renderer skips almost everything outside the picture, so the gap is small: 11 to 17 ms on the GPU
  against 24 ms. The GPU renderer still walks and submits every shape; culling (plan task 5.7) is what would help there.
- **Most of a steady frame is not the card.** 5.6 of the 7.3 to 8.3 ms is the CPU walking the scene, checking the caches and
  comparing chunk signatures on every frame; in the close-up 10 ms of 11 to 17 ms. Task 2.10 (a frame where nothing changed does
  nothing) and Phase 5 remove this; the target is under 0.5 ms.
- Upload per steady frame: 0 bytes. A cold frame sends 41 to 47 MB (1.1 to 1.26 million vertices at 24 bytes plus the
  chunk index lists). Graphics memory held: 148 MB, of which about 36 MB is the view's targets (4x MSAA color and
  depth-stencil and the picture at 1080p) and the rest geometry and the shape table.
- Zooming in makes more triangles (round joins and curves are cut finer: 1.06 M, 1.12 M, 1.22 M), which is the zoom
  bucket of plan D3 at work, and costs a new cold frame when the bucket changes.
- Not measured yet: the T2 laptop and the web (open point P2, task 2.12), 4K, the illustration scene (it has gradients,
  opacity groups, clip groups and symbols that the GPU renderer cannot draw yet; `--gpu-draw-supported` times only what
  it can).

## Plan task 2.13, tier T1: CPU and GPU side by side (2026-10-17, after culling and the zoom fixes)

Machine: RTX 3060 on this desktop (Vulkan), release build, headless, 1920 x 1080, 4x MSAA, 10 steady frames per view, one cold
frame first. "GPU steady" is submit until the card is done. `evolve_editor --benchmark <scene> <report>`; each run took 3 to 6
seconds of GPU time (the illustration scene, 66 s, is the CPU renderer's 2 to 3 seconds per frame). Reports are in `local/bench/`.

| scene, view | objects visible | CPU renderer | GPU cold frame | **GPU steady** (CPU update part) | draw calls | graphics memory |
|---|---|---|---|---|---|---|
| real map, whole | 100% | 67 ms | 197 ms | **7.7 ms** (6.9) | 42 | 149 MB |
| real map, region | 10% | 38 ms | 83 ms | **3.4 ms** (2.9) | 16 | 104 MB |
| real map, close-up | 0.5% | 10 ms | 14 ms | **1.3 ms** (1.0) | 1 | 77 MB |
| stress scene, whole | 100% | 91 ms | 183 ms | **7.3 ms** (6.4) | 42 | 139 MB |
| stress scene, region | 10% | 34 ms | 57 ms | **2.8 ms** (2.3) | 13 | 95 MB |
| stress scene, close-up | 0.5% | 12 ms | 11 ms | **1.4 ms** (1.2) | 1 | 76 MB |
| illustration, whole (timing only) | 100% | 2,791 ms | 47 ms | **9.1 ms** (0.9) | 9 | 88 MB |
| illustration, region (timing only) | 10% | 1,944 ms | 26 ms | **4.7 ms** (0.5) | 3 | 82 MB |
| illustration, close-up (timing only) | 0.5% | 1,907 ms | 8 ms | **4.0 ms** (0.3) | 1 | 76 MB |

- **Culling worked as planned:** the region and close-up views went from 8.9 to 14.9 ms and 11 to 17 ms (before, see
  "first numbers" above) to 2.8 and 1.4 ms, and the GPU is now 9 times faster than the CPU renderer even in the close-up.
- **Whole scene: still 7 ms, of which 6 to 7 ms is the CPU walking 21,000 shapes** each frame (plan Phase 5 target: under
  0.5 ms). The card itself is not the limit there.
- **The illustration rows are not pictures the renderer could draw yet** (gradients, opacity, clip groups and symbols were left
  out, 3,700 of 4,976 objects), so they said what the supported part costs, not what the finished renderer will. There the CPU
  work is tiny (0.3 to 0.9 ms) and the 4 to 9 ms is the card: thick strokes overlapping at 4x MSAA. *(Complete since Phase 4: see
  "After Phase 4" below.)*
- A cold frame is 0.2 s natively for the whole map (1.5 s in the browser, see the canary).
- The benchmark again showed P9: deleting or grouping 2,000 selected shapes takes 0.4 s in the generated scene.

## After Phase 3: gradients, opacity, bevels (2026-10-17, tier T1, plan 3.x)

Same machine and method as the 2026-10-17 table above (1920 x 1080, 4x MSAA, 10 steady frames, release). What changed since: shape
table rows of 8 texels, gradients drawn, strokes flattened 13% finer, hysteresis, a faster hash for the per-frame maps, the
miter-limit pieces. The machine was quieter this time, so small differences are not meaningful.

| scene, view | CPU renderer | GPU cold frame | GPU steady (CPU update part) | graphics memory |
|---|---|---|---|---|
| real map, whole | 69 ms | 220 ms | **8.2 ms** (7.5) | 155 MB |
| real map, region (10%) | 35 ms | 81 ms | **2.3 ms** (1.7) | 107 MB |
| real map, close-up (0.5%) | 10 ms | 14 ms | **1.0 ms** (0.7) | 77 MB |
| stress scene, whole | 88 ms | 179 ms | **6.0 ms** (5.0) | 143 MB |
| stress scene, region | 33 ms | 58 ms | **2.6 ms** (2.1) | 96 MB |
| stress scene, close-up | 13 ms | 11 ms | **1.1 ms** (0.8) | 76 MB |
| illustration, whole (timing only) | 2,826 ms | 62 ms | **12.5 ms** (0.9) | 95 MB |
| illustration, region (timing only) | 1,953 ms | 39 ms | **8.4 ms** (0.4) | 87 MB |
| illustration, close-up (timing only) | 1,882 ms | 11 ms | **5.3 ms** (0.3) | 77 MB |

- The map got no slower from the extra work (155 MB against 149 MB in memory: the finer strokes), once the per-frame lookups
  use the faster hash. Without it the whole-map frame had grown to 9 to 13 ms.
- The illustration scene now draws its 406 gradients (its picture is still incomplete: 216 opacity groups, 15 clip groups and
  3,100 symbol instances are left out). Its time is the card: thick overlapping strokes and large gradients at 4x MSAA.
  That is the scene to watch when layers arrive (Phase 4) and for fill rate on the T2 laptop (Phase 5).

## After Phase 4: layers, clips and instances (2026-10-16, tier T1, plan 4.x)

The illustration scene is now drawn **completely** (nothing is left out: `skipped` is empty). Same machine and method (1920 x 1080,
4x MSAA, RTX 3060 on Vulkan, release build, headless, one cold frame and then 12 steady frames per view), measured with
`cargo test --release -p evolve_editor where_the_time_of_a_gpu_frame_goes_with_the_generated -- --ignored --nocapture`, which takes
a few seconds of GPU time. "Card" is the time from the submit until the card reported the work done.

| scene, view | CPU renderer (2026-10-17) | GPU cold frame | GPU steady: CPU / card | items | draw calls | passes (layers) |
|---|---|---|---|---|---|---|
| stress scene, whole | 88 ms | 172 ms | 6.2 / 1.1 ms | 21,200 | 42 | 1 |
| stress scene, region | 33 ms | 101 ms | 3.6 / 0.7 ms | 11,523 | 23 | 1 |
| stress scene, close-up | 13 ms | 12 ms | 1.1 / 0.3 ms | 652 | 2 | 1 |
| **illustration, whole** | **2,826 ms** | 124 ms | **9.9 / 6.8 ms** | 29,353 | 879 | 277 (276) |
| **illustration, region** | **1,953 ms** | 90 ms | **3.9 / 1.7 ms** | 15,220 | 146 | 29 (28) |
| **illustration, close-up** | **1,882 ms** | 16 ms | **1.1 / 0.5 ms** | 260 | 9 | 3 (2) |

- **The complete illustration scene takes about 17 ms for the whole view** (CPU 9.9 + card 6.8, which overlap in practice), against 2.8 s
  for the CPU renderer: over 150 times faster, and over 1,000 times in the close-up. Its 276 layers are what the CPU renderer made as
  full-screen pictures; here they are 2.2 million pixels in all (the whole view is 2.1 million), because each is the size of its group.
- The 876 or so draw calls of the whole view are mostly the layers: every layer is a pass with one or a few calls, and every composite
  is one call. That is where much of the 9.9 ms of CPU time goes (the stress scene, with no layers, takes 6.2 ms for 21,000 shapes,
  so the 276 layers cost a few milliseconds: roughly 15 to 30 microseconds each). Fewer layers are cheaper still: a group with opacity whose children do not touch each other, or a
  single child, needs none (plan 4.4).
- **The whole-map pan got a little slower than at the end of Phase 3** in the real editor frame: 7.1 to 8.6 ms (6 ms before; zooming
  25 to 27 ms on average against 21; dragging 28 to 30 ms, 29 before; one game frame 0.5 ms, unchanged). The restructuring (items
  carry their triangles and a scope, chunks are signed with what they were made from, passes and ops) cost about a millisecond and a
  half. Stage by stage a pan frame prepares in 6.2 ms: 2.2 ms to walk the scene and bring the triangles up to date, 2.2 ms to find the
  draw items and 1.8 ms for rows, vertices and chunks (the plan's Phase 5 removes these walks for views that did not change).
  `FrameStats` now has `tessellate_ms`, `collect_ms` and `list_ms`, and `where_the_time_of_a_gpu_frame_goes_with_the_map` prints them.
- Not measured: the T2 laptop and the browser (open point P2; task 4.8). The canary needs no change, it draws what it is given.

## Animated frames: the sprites scene (2026-10-23, tier T1, plan 6.4 and 6.6)

`cargo test --release -p evolve_editor what_an_animated_frame_costs -- --ignored --nocapture`: 3,000 instances of a symbol of three
parts (`--gen-scene sprites`), 1080p on the RTX 3060; in every frame some of them are moved and turned a little, as physics or a script
would, and the frame is prepared and drawn (40 frames each, the card's time included; the animation's own time is the finding of each
node by its id and the write of its transform).

| instances moved in a frame | animation (ms) | preparation, before | preparation, now | whole frame with the card, now |
|---|---|---|---|---|
| 3,000 | 2.2 | 2.97 ms | 2.86 ms | 5.3 ms |
| 300 | 0.02 | 2.92 | **0.25** | 0.55 |
| 100 | 0.00 | 2.88 | **0.09** | 0.51 |
| 1 | 0.00 | 2.90 | **0.01** | 0.50 |

A frame in which everything moves costs what a walk costs (about a microsecond for each instance); in all the others only what moved
is touched. No triangles are made or sent in any of them. The preparation of a frame used to be the same 2.9 ms whatever moved, and
for a scene with a large static part (the 21,000 shapes of the stress scene, say) it would have been 6 ms or more.

## The web first frame (2026-10-20, headless Edge, WebGL2 on SwiftShader, stress scene, plan 5.4)

| Memory reserved at the start | cold first frame | of which making triangles |
|---|---|---|
| none / 192 MB | 1,280 ms | 1,010 ms |
| 320 MB | 624 ms | 391 ms |
| 448 MB | 168 ms | 85 ms |
| 768 MB | 155 ms | 84 ms |
| sized by `memory_for_shapes` (485 MB, 93 ms to make) | 158 ms | 87 ms |

The cost was the program's memory growing in many small steps, not the tessellator. The region view's first frame is still 400 ms
(250 ms building items). Andrea's own Chrome and the T2 laptop are not measured yet.

## Official `--benchmark` after the first part of Phase 5 (2026-10-20, tier T1, real map only)

`evolve_editor --benchmark local/ironkrieg/map.evolve local/bench/map-p5` (Andrea allowed the run; 3 s in all, report in
`local/bench/map-p5.txt`). Same machine and settings as the 2.13 table, steady frames in ms (CPU update part in brackets):

| view | before Phase 5 | now | GPU cold frame before -> now |
|---|---|---|---|
| whole | 7.7 (6.9) | **0.86 (0.01)** | 197 -> 237 ms |
| region | 3.4 (2.9) | **0.47 (0.00)** | 83 -> 88 ms |
| close-up | 1.3 (1.0) | **0.25 (0.00)** | 14 -> 16 ms |

Read this with care: the benchmark draws the same view ten times, so every steady frame is a reused frame (the fast path of 5.3).
A real pan or zoom costs what the editor probes in the next section show. The cold frame of the whole view is 20% slower than
before (one run each, so it may be noise; to check when the next full run is done).

The stress scene and the illustration scene, later the same day (after the zoom and layer reuse of 5.3; reports `local/bench/large-p5.txt`
and `illustration-p5.txt`; 6 s and 66 s). Steady frames as the benchmark measures them, each from submit to completion with the card
idle in between (so they include the wake-up of the card, a floor of about 5 ms that the old tiny-scene reports show too):

| scene, view | items / draw calls | cold frame | steady frame (CPU update) | graphics memory |
|---|---|---|---|---|
| stress, whole | 21,200 / 42 | 182 ms (before 183) | **0.88 ms** (0.00) (before 7.3) | 143 MB |
| stress, region | 6,407 / 13 | 61 ms | **0.44 ms** | 96 MB |
| stress, close-up | 249 / 1 | 9 ms | **0.24 ms** | 76 MB |
| illustration, whole | 29,353 / 879 | 234 ms | 7.98 ms (p95 39.8) (0.06) | 201 MB |
| illustration, region | 8,330 / 81 | 82 ms | 14.4 ms (0.02) | 205 MB |
| illustration, close-up | 232 / 1 | 38 ms | 25.4 ms (0.01) | 80 MB |

Memory by kind, new in the report (`GPU memory: content / layers / targets`, at 1080p with the High profile): stress scene 68 / 0 / 75 MB
(whole view), 21 / 0 / 75 (region), 1 / 0 / 75 (close-up); the real map 81 / 0 / 75, 32 / 0 / 75, 3 / 0 / 75. The targets are the
formula (75 MB at 1080p); the content of the whole stress view is a little over the 64 MB the plan proposed.

The illustration scene's steady numbers are not comparable with the old reports (then the GPU skipped what it could not draw: 4,807
items; now it draws all 29,353 items with 276 layers and the instances). They are also far above what the card needs: the probe
`what_the_layers_of_the_illustration_scene_cost` (`profile_tests.rs`: the benchmark's three views, 20 frames one after the other, then
wait) gives **10.3 ms** (277 passes, 276 layers), **3.1 ms** (14 passes) and **1.9 ms** (1 pass) a frame. So the close-up is not
slow; the benchmark's way of timing one frame at a time with an idle card adds up to 20 ms to a small frame on this machine. The
whole view of the illustration scene is the one the card spends its time on (276 layers drawn again every frame: plan 5.13).
Also in these runs: a click on the stress scene takes 0.3 ms (was 6 ms; plan 5.11). Deleting 4,000 selected objects took 1.3 s on
the stress scene, as in every earlier report, and grouping them 1.4 s (a search and a shift of the list for each object, in the
editor's delete and group, not in the renderer; TODO.md). After `Scene::remove_all` and the new `Scene::group` (one walk each): 3 ms and
2 ms. The 21,200-shape click is 0.3 ms.

## After the first part of Phase 5: frames that do not walk the scene (2026-10-20, tier T1, plan 5.1, 5.3, 5.4)

`cargo test --release -p evolve_editor the_cost_of_each_kind_of_frame -- --ignored --nocapture` (a few seconds on the card) times the
CPU side of preparing each kind of frame, averaged over 30 frames, at 1920 x 1080 on the RTX 3060. Before and after, in milliseconds:

| frame | stress scene | real map | illustration |
|---|---|---|---|
| nothing changed | 6.8 -> **0.01** | 7.2 -> **0.00** | 6.7 -> 6.3 |
| pan 3 px | 6.9 -> **0.00** | 7.0 -> **0.01** | 6.9 -> 6.5 (the layers' places) |
| zoom 2% (no budget) | 7.2 -> 7.0 | 7.3 -> 7.0 | 22 -> 22 (13 MB sent: one copy per instance) |
| one shape moved | 7.4 -> **0.06** | 7.3 -> **0.2** | 6.5 -> 6.6 |
| one outline edited | 8.9 (2.1 MB sent) -> **0.08** (12 kB) | 7.9 (1.5 MB) -> **0.07** (6 kB) | 6.6 -> 6.9 |

The real editor (`PROFILE_GPU=1 cargo test --release -p evolve_editor where_the_time_of_a_frame_goes_with_the_map -- --ignored
--nocapture`, the map, two views): a pan 7.5 -> 0.3 ms; one dragged frame (both views, the undo snapshot that the headless run adds
left out) about 22 -> 5 ms; zooming 20 steps: average 25 -> 18 ms, **worst frame 169 -> 45 ms** (zoom out 134 -> 36 ms), thanks to
the 4 ms budget for making shapes. The illustration scene (layers, instances, fading) always takes the whole path: 6 to 7 ms for
any frame, and 22 ms for a zoom, which is the cost of instances (5.12) and of redrawing every layer (5.13).

## What a real editor frame costs with the map (2026-10-16, after the owner's first hands-on try)

The benchmark above times the renderer alone. The owner found the editor slow with the GPU renderer, so the real panes were
timed one by one on the real map (`local/ironkrieg/map.evolve`, 21,000 shapes), release build, RTX 3060, no window (the card's
presenting and the window system are not included). Reproduce: `PROFILE_GPU=1 cargo test --release -p evolve_editor
profile_tests -- --ignored --nocapture` (without `PROFILE_GPU` the CPU renderer is used). It takes a few seconds of GPU time.

| what | before | after |
|---|---|---|
| Scene view, nothing changed (GPU renderer) | 420 ms | 0.3 ms |
| Scene view after a pan (GPU renderer) | 430 ms | 6 ms |
| Scene view, CPU renderer | 480 ms | 6 ms (the overlays were the cost) |
| Zooming in and out, 20 steps (GPU renderer) | 31 to 46 ms on average, **then the CPU renderer took over for good** (budget) | 21 to 23 ms on average, worst 115 to 135 ms at a bucket change, stays on the GPU |
| Dragging a province, whole editor frame | 80 ms (CPU renderer in the Scene view) | 29 ms (9.7 Scene view, 8.3 Game view, 9.5 undo bookkeeping that runs once the mouse button is up) |
| Starting a game | 1,360 ms | 25 ms |
| One game frame (update, Scene view, Game view) | 2,160 ms | 0.5 ms |

What it was: the editor's bone overlay (`bone_ids`, looking each node up by its id) cost 420 ms a frame; the physics step,
the game statistics and the collision overlay did the same in play mode; the GPU renderer made every shape again at every
zoom bucket, went over its geometry budget and left the view to the CPU renderer. See the plan's progress log of 2026-10-16.
Still open: a zoom that crosses a bucket tessellates every visible shape in one frame (about 130 ms at the whole-map view);
the plan's per-frame budget (B3, task 5.4) spreads it over several frames.

## The web canary (2026-10-17, plan task 2.12, tier T1 machine, headless Edge 155)

`tools/web-canary` runs the renderer in a browser: three views of a generated scene, 60 frames each on a 960 x 540 canvas.
Build and run: MANUAL_TESTS.md, "The web canary". The stress scene (21,203 objects):

| | WebGL2, real card (ANGLE Direct3D 11) | WebGPU, real card | WebGL2, software (SwiftShader) |
|---|---|---|---|
| errors from the graphics layer | 0 | 0 | 0 |
| first frame, whole scene (everything made) | 1,516 ms | 1,536 ms | 1,496 ms |
| whole scene, CPU time of a frame (median) | 7.5 ms | 7.2 ms | 12.3 ms |
| whole scene, time between frames | 8.3 ms | 8.3 ms | 192 ms (the software rasterizer) |
| region (10% visible), CPU time / between frames | 5.6 / 4.2 ms | 5.8 / 4.2 ms | 6.4 / 108 ms |
| close-up (0.5% visible), CPU time / between frames | 1.9 / 4.2 ms | 2.2 / 4.2 ms | 3.1 / 25 ms |
| draw calls, whole / region / close-up | 42 / 25 / 2 | same | same |
| geometry / table / targets memory | 16 / 2 / 28 MB | same | same |

The headless browser does not wait for the screen (frames come faster than 60 Hz), so the "between frames" column on the real
card is the cost of drawing, not what a screen shows. The illustration scene (gradients, opacity, clip groups and symbols left
out, so timing only): 128 ms first frame, 2.3 ms whole scene on the software driver. Findings: nothing in the shaders or the
buffers needed to change for WebGL2 or WebGPU; the one clear web cost is the cold first frame: tessellating all shapes takes
**1.5 s in wasm** against 0.19 s natively. The T2 laptop is still to be measured (P2).

The owner's own run (2026-10-17, Chrome 155 with a visible window, 1280 x 720, WebGL2 on the RTX 3060 through ANGLE Direct3D 11,
stress scene, 120 frames per view): 0 graphics errors; first frame 1,584 ms; whole scene 7.4 ms CPU per frame and 8.3 ms
between frames; region 5.5 / 4.2 ms; close-up 1.9 / 4.2 ms; geometry 16 MB, table 2 MB, targets 49 MB. The same as the headless
runs. Frames 4.2 ms apart mean that screen refreshes faster than 60 Hz, so the close-up is limited by the screen, not by us.

## After step 3: editor costs (2026-10-15)

Measured on the same map (21 440 shapes). The Objects panel is run headless (300 x 700 px, layout, painting and
tessellation of its rows); the selection numbers are for every shape selected (Ctrl+A).

| | before | after |
|---|---|---|
| Objects panel, one frame | 103 ms | 0.3 ms |
| find the top-level objects of a selection | 18 ms (250), 129 ms (500), 1 s (1 000), 7.7 s (2 000); cubic, so about 2.6 hours for all 21 440 | 0.4 ms |
| drop deleted objects from the selection (every frame) | 2 ms at 2 000, grows with the square: about 230 ms at 21 440 (extrapolated) | 0.6 ms |
| combined bounds of the selection (every frame) | 12.5 ms at 2 000, about 1 s at 21 440 (extrapolated) | 9.7 ms |
| clip preview clone, while a clip plays | twice per frame (Scene and Game view) | once per frame |
| undo check, nothing changed | 3.6 to 4.1 ms | unchanged |

- The Objects panel built and laid out a row for every object on every repaint (21 440 rows, six widgets each),
  which is the lag seen when the map was first opened. It now draws only the rows in view. A headless test scrolls a
  3 000-row tree and clicks it, and fails if hidden rows are painted.
- `Scene::top_level_selection` compared every selected object with every other one and looked each up by walking the
  tree. It is now a single walk. Tests compare it, the new `selection_bounds` and `retain_existing` with the old
  one-by-one definitions on random trees.
- Delete and group of a big selection are fine (about 160 and 220 ms for 4 000 shapes).
- **Not done:** the undo check (every repaint compares the whole scene with the last undo state) needs real change
  tracking. Skipping it on quiet frames would help little, since mouse movement counts as activity, and would change
  what Undo does after a late change. The selection bounds could be cached while the selection and scene are unchanged.

## What the numbers say (baseline, before culling)

- **Frame time does not depend on what is visible.** The close-up draws 0.5% of the objects and costs about as much
  as the whole map. The renderer has no culling: it converts and draws every object each frame. Culling alone
  should remove most of the close-up and region cost.
- **Frame time depends on the amount of geometry, not on the object count.** 4.4 times fewer objects with the same
  anchor points is only 10% faster.
- **About 13 fps for the whole map at 1080p**, before the editor draws anything else. The 4K case is not measured.
- **Editor costs:** the undo check takes 3.6 ms on every repaint that is not part of a drag. The scene clone for the
  clip preview (14 ms) only happens while a clip plays (this paragraph first said "every frame"; that was wrong).
- **The project file is the surprise: 360 to 380 MB** for 679 000 points, because the JSON writes every number on
  its own line. Loading is fast (about 0.4 s); the size is the problem (saving, autosave every 30 s, sharing).
- Object lookup by id and RAM are not a problem at this size.

## Limits of these numbers

- One machine, one run each, 1080p only, 10 frames per view. Treat differences under about 10% as noise.
- Borders are 3 km (nation) and 1 km (internal) wide in world units, so at the full-map zoom they are thinner than a
  pixel. The game wants constant pixel widths; real strokes will cost more.
- The map has no coat of arms, labels or UI yet.
- Not measured: the editor's own window drawing, the texture upload to the graphics card (it cannot be timed
  headless; the bitmap is 8.3 MB per frame), 4K, and memory use while editing.
- A node with opacity below 1 is drawn into a full-screen layer of its own (`render.rs`, `draw_node`). This was
  first only read from the code; the illustration scene (above) measured it: about 8 ms per such node at 1080p.
