# Changelog

All notable changes to EvolveEngine. The format follows [Keep a Changelog](https://keepachangelog.com);
the project is not released yet, so everything is under "Unreleased".

## Unreleased

### Fixed
- Selecting a lot (all 4,640 objects of a large map) made the Scene view take 69 ms a frame, even when nothing moved. The outline of
  the selection looked each selected object up from the top of the tree twice (quadratic); it is now one walk of the scene. Outlines
  that are wholly off the screen are no longer made, shapes off the screen are not flattened, and when more than 30,000 points are left
  the step between points grows (up to 4 pixels) so that the cost does not grow with the selection. Converting a point from the world to
  the screen rebuilt the camera matrix (with a sine and a cosine) for every point: 230,000 times a frame. `Scene::selection_bounds`
  flattened every path to find its box; a path of straight lines now takes the box of its moved anchors (`Path::bounds_under`, 4 ms ->
  1 ms for the map). The map with everything selected: 69 ms -> 9 ms at the whole-map view, 69 ms -> 2.3 ms close up.
- A game that changed the look of thousands of nodes in one frame (every province and border of the map changing owner) spent 11.5 ms of
  its 14 ms renderer time looking each changed node up with `Scene::find`, a search from the top of the tree. The renderer's patch path
  now uses its own index. Conquering the whole map in one frame: 29 ms -> 15 ms (7.7 ms from the second time on). See BENCHMARKS.md.
- A running game with a camera that moves or zooms (one driven by a script) redid all the drawing work in every frame, even when
  nothing moved: a script that set the camera's size wrote it every time, and any change to the camera node made the GPU renderer
  walk all 21,000 shapes again. Setting the size the camera has changes nothing now; a camera that moves or changes size no longer
  makes the renderer redo the picture (the camera's frame is painted by the editor under the card's picture, not drawn by the card).
  On the map in play mode (headless, 1280 x 1041): idle 6 ms -> 0.01 ms of preparation, panning 7.5 ms -> 0.5 ms, zooming 14.7 ms ->
  5.7 ms on average (with a hitch of up to 50 ms when a zoom steps into a new level of detail, still to do).
- GPU renderer: giving back the memory ranges of a whole zoom step took 17 ms of a 45 ms frame on the map (every give-back looked
  through all the free ranges, and every range taken back looked through all the data waiting to be sent); both are now
  logarithmic. Packing a buffer copies neighbouring ranges in one piece. The worst frame of a zoom on the map went from 45-60 ms to
  about 30 ms.
- GPU renderer: shapes (provinces of a map) lost their fill while zooming or panning, leaving only the border drawn by a
  neighbouring object; new geometry sent to the card wrote zeros over shapes that were already there.
- Selecting shapes with outlines that turn back on themselves drew long straight lines across the whole view (the selection
  outline, and the collider outlines in play mode).
- Sharp corners with a miter join: the GPU renderer drew a long spike where the CPU renderer (tiny-skia, and SVG) cuts the corner
  flat, for corners a little sharper than the miter limit; and a path that doubled back on itself exactly also showed a spike.
  Corners now follow the same rule in both renderers.
- Strokes were flattened too coarsely at a deep zoom (an edge half a pixel too far in on wide strokes around curves).
- The editor was very slow on scenes with thousands of shapes: the bone overlay, the physics step, the game statistics and the
  collision overlay looked every node up by its id once per node (quadratic: 420 ms a frame for 21,000 shapes). Starting a game
  on a map of 21,000 shapes took 1.4 s and each frame 2 s; now 25 ms and 0.5 ms.
- The GPU renderer went over its geometry budget when the zoom changed, and then drew the view with the slow CPU renderer. It now
  gives back what is out of date before asking for room, makes room once instead of doubling, packs buffers that are mostly
  holes, and a frame that fails half way no longer leaves ranges in use that were given back. The geometry budget of the High
  profile is 512 MB (was 192 MB).
- Deleting or grouping thousands of selected objects froze the editor (1.3 s for 4,000 of 21,200); each now takes a few
  milliseconds. Clicking an object on a scene of 21,000 shapes took about 6 ms and now takes 0.3 ms.
- The GPU renderer redid all its work for every frame, even when only the camera moved a little. A pan, a small zoom or a frame where
  nothing changed now costs almost nothing (0.01 ms against 6 to 7 ms on 21,000 shapes), also for scenes with layers and symbol
  instances when the camera moves; moving or editing one shape costs 0.1 ms and sends 12 kB instead of 2 MB.
- A browser's first frame of a large scene took 1.3 s because the program's memory grew in many small steps; it now grows once and
  the frame takes 0.16 s.

### Added
- Project file format version 4: physics settings for the whole project (`PhysicsSettings`: whether bodies away from the camera
  are stepped less often, the margin around the camera's frame, how many frames apart they are stepped). Left out of the file while
  they are the defaults (off, half a frame, every 4 frames). Older files open with the defaults.
- Physics: with `slow_offscreen` on, dynamic bodies in the camera's frame grown by the margin are stepped every frame, the others
  once every `offscreen_interval` frames by all the time they waited, collisions included (turns spread by id, so the work is even
  from frame to frame). A waiting body that a stepped one may reach is stepped too; a waiting body keeps its contacts. The schedule
  (`physics_schedule.rs`) is apart from the solver. Longer steps are less precise: a bouncy ball out of view bounces fewer times.
- Game menu: "Slow physics out of view", with the margin (a share of the camera's frame, 0 to 500 %) and the interval (1 to 60
  frames). A change holds at once in a running game (`Runtime::set_physics_settings`). The undo history records the project's
  physics settings along with the scene, so Ctrl+Z takes back a change to them in the order it was made.
- Play mode: with "Show collision shapes", a body that waits for its turn out of view is drawn dashed (`DebugBody::waiting`). The
  Game menu's "Show physics area" draws the camera frame and the margin, where bodies are stepped every frame
  (`Runtime::debug_physics_area`); it is greyed out while the rule is off.
- `NodeMut::set_stroke_width(width)` and the script command `set_stroke_width(me, width)` (0 to 10,000): the width of a stroke is a
  paint change (it was a geometry change), and the GPU renderer keeps the triangles of a stroke while its new width is at most 10% wider
  or at least a quarter of the width they were made for (`evolve_gpu::tessellate::stroke_width_holds`); beyond that the shape is made
  again at the new width. A width that makes the stroke appear (from 0) or vanish still counts as a geometry change. So a game that makes
  a border bold or thin pays for a row of the shape table, not for new triangles. A game restyling the 6 borders of a province: 0.3 ms;
  all 3,403 borders of the map in one frame: 15 ms the first time (the thin borders that turn bold are made once), 8 ms after that.
  Each tessellation now has a stamp of its own (`CachedShape::made`), which the draw list remembers instead of the shape's geometry stamp,
  so a shape made again for the same geometry is never taken for the one it replaces.
- `Path::bounds_under(&Affine2)`: the bounds of a path moved by a transform, without copying or flattening a path of straight lines.
- Mouse and camera in scripts, for a game that moves its own camera: scripts can read `mouse_right_down()`, `mouse_right_pressed()`,
  `mouse_right_released()` and `mouse_wheel()`, and find and resize the camera: `main_camera()`, `camera_width()`, `camera_height()`,
  `set_camera_height(h)`, `set_camera_size(w, h)` (sizes are kept between 0.001 and 10,000,000, and a size that is not a number is
  ignored). In the editor the Game view gives the game the wheel and the right button, and the Scene view keeps them for its own
  camera; a drag that began over the picture goes on when the pointer leaves it. No change to the project file.
- A pin at the end of every row of the Objects panel: a click selects the object and takes the Scene view to it, to find an object of a
  list of thousands on the map. Objects that are tiny are shown among their
  surroundings: the view never shows less than half a thousandth of the scene's size. The status bar says what the
  pin did.
- View -> Frame rate: a cap on the editor's frame rate while a game runs or a clip plays (Display rate, 10, 30, 60 or 140 fps), to see
  how a game feels at a lower rate and to spare the card. Idle, the editor still draws nothing. The cap sits on top of vsync, so on a
  240 Hz display 140 lands on the display's steps.
- Playing an animation clip in the editor no longer copies the whole scene in every frame (12 to 14 ms on the big scenes): one copy is
  kept and posed again, and only when the scene, the clip or the IK constraints change is it copied anew.
- GPU renderer: in a running game (or any scene where groups and symbol instances move), a frame costs in proportion to what moved,
  not to the size of the scene: moving 100 of 3,000 sprites takes 0.09 ms of preparation, against 2.9 ms before.
- GPU renderer: skinned shapes (shapes bent by bones, like the sample's tentacle) are drawn on the GPU. Only the shapes whose bones
  moved are made again each frame. A skinned clip mask and a skinned shape inside a symbol instance still use the CPU renderer.
- GPU renderer quality profiles: High, Balanced and Low (memory caps, time spent on shapes that are not needed at once, and a render
  scale: Low draws at three quarters of the size and stretches the picture). The card's kind picks one at start (a discrete card High,
  an integrated one Balanced), *Automatic* lowers it while frames are slow and raises it again when they are fast, and
  *View → Renderer → Quality* or `--quality auto|high|balanced|low` chooses by hand. The benchmark reports the card's profile and its
  memory as content, layers and targets.
- The GPU renderer draws **clip groups, groups with opacity and symbol instances**, so the only thing it still leaves to the CPU
  renderer is skinned shapes (the sample's tentacle). Clips use the stencil (anti-aliased like the CPU renderer's); a group with
  opacity is drawn on a layer the size of the group (not the whole view, as the CPU renderer does), or with no layer at all when its
  children cannot touch each other; instances share the tessellation of their symbol, with colour overrides and their own opacity.
  The illustration benchmark scene, which takes the CPU renderer 2.8 s, is drawn completely in about 17 ms. When the memory for
  layers is full a layer is drawn smaller instead of the view going back to the CPU renderer.
- The GPU renderer draws **gradients** (linear and radial, fills and strokes) and **opacity on a single shape** without a layer
  (a shape with only a fill or only a stroke, or with both when both are opaque). Shapes stay in their zoom bucket a little past its
  edge, so a zoom that sits on a boundary no longer redoes every curve on every frame.
- The `evolve_gpu` crate, the start of the GPU renderer (plan: [GPU_RENDERER_PLAN.md](GPU_RENDERER_PLAN.md)): so far a
  headless graphics device (also limited to what WebGL2 offers, for tests), offscreen drawing with pixel readback, and
  an edge-aware picture comparison for tests. No rendering yet.
- Change tracking in the scene (`evolve_engine::revision`): every node and the scene carry stamps for what changed
  (shape geometry, transform, paint, structure, other data). Stamps are not saved; the project format is unchanged.
  It is the base the GPU renderer needs, and it is already used by the undo history. Contributor rules are in
  [CONTRIBUTING.md](CONTRIBUTING.md).
- The **GPU renderer** (Phase 2 of [GPU_RENDERER_PLAN.md](GPU_RENDERER_PLAN.md)), in the editor behind
  `evolve_editor --renderer auto|cpu|gpu` and *View → Renderer* (the default stays the CPU renderer). It draws solid fills
  and strokes (curves, joins, caps, transforms, groups, transparent colors, thin lines) with 4x anti-aliasing, batched into a few
  dozen draw calls, and shows a view again without redrawing when nothing changed. On the large benchmark scene a
  frame takes about 8 ms instead of 110 ms. Views with gradients, opacity, clip groups, symbols or skinned shapes still use
  the CPU renderer and say so. The CPU renderer is unchanged and remains the reference and the fallback; a lost graphics
  device or a card error switches the GPU renderer off without a crash. `--benchmark` now times the GPU renderer too
  (`--gpu-draw-supported` times scenes it cannot draw completely).
- **Web canary** (`tools/web-canary`, build with `tools/build-web-canary.ps1`): a browser page that runs the GPU renderer on
  WebGL2 or WebGPU and reports adapter, limits, frame times, memory and graphics errors. `evolve_gpu` now compiles for the
  browser.
- Frame times in the editor's status bar (frames per second while frames follow each other, average and worst work of the
  last 120 frames, hover for where the time went), and a **Renderer** box next to them. In Play mode the right mouse button
  moves the view around (while editing it still rotates it).
- The GPU renderer makes and draws only the shapes that can reach the view (view culling): zoomed in on a big scene, the work
  and the memory shrink by the share that is off screen, and the picture is identical.
- `Scene::for_each_node`, one walk over every node in document order (use it instead of `all_ids` followed by `find` for each id).
- `GPU_SAFETY.md`: rules for GPU code (frame pacing, bounded work, a memory budget, survivable errors) that the code
  follows and tests check.
- `EVOLVE_REQUIRE_GPU` and a CI job (`gpu-required`, Linux with a software Vulkan driver) in which GPU tests cannot skip
  quietly, plus a diagnostic job that lists what the Windows, macOS and Linux runners have.
- Tests that type into the real editing panels with simulated input (`egui_kittest`), a test that the panels leave the
  scene alone on idle frames, and a test that records what the SVG importer does with features the engine lacks.
- `local/` (git-ignored) for big generated files such as imported maps and benchmark reports.
- `evolve_editor --gen-scene illustration <out.evolve>`: a second benchmark scene with curves, gradients,
  transparency, opacity and clip groups and symbol instances (8 276 items).
- `evolve_editor --gen-scene large <out.evolve>`: a deterministic benchmark scene (21 200 shapes, 660 000 points) that
  needs no game files; `--benchmark` now also names the graphics card and times the scene walk.
- `evolve_editor --benchmark <project> <report>`, a headless benchmark of rendering and editing cost
  ([BENCHMARKS.md](BENCHMARKS.md)).
- MIT licence ([LICENSE](LICENSE)) and `license` in every crate.
- Continuous integration ([.github/workflows/ci.yml](.github/workflows/ci.yml)): build and tests on Windows, macOS and
  Linux, clippy with warnings denied, and the WebAssembly player.
- macOS and Linux support: `tools/build-web-player.sh` (twin of the PowerShell script), per-platform data folder for
  autosave and the panel layout (`~/Library/Application Support`, `~/.local/share`, `%LOCALAPPDATA%`).
- Trackpad navigation in the Scene view: two-finger scroll pans, pinch and `Cmd`/`Ctrl`+scroll zoom; a mouse wheel
  still zooms.
- [CONTRIBUTING.md](CONTRIBUTING.md), a README for newcomers (platform requirements, licence, project direction), and
  the plan in [TODO.md](TODO.md) (phases 1 to 5).
- Release workflow ([release.yml](.github/workflows/release.yml)): on a `v*` tag it builds downloads for Windows, macOS
  (Apple Silicon) and Linux, each with the editor, the player and the web player, and drafts a GitHub release.
  `tools/package-release.sh` does the packing and can be run locally.
- Project-format safety net: frozen fixtures for format versions 1 to 3, tests that they keep opening and that the
  writer's output only changes together with a version bump, and the rules in CONTRIBUTING.md.
- Issue forms (bug report, feature request), a pull-request checklist and a [code of conduct](CODE_OF_CONDUCT.md).
- Manual test section "Navigating the Scene view (trackpad and mouse)".

### Changed
- Undo no longer compares the whole scene on every repaint: it asks the scene's stamps, which takes no measurable
  time (3.6 ms per frame on the 21 000-shape benchmark scene before). Undo behaves the same.
- `Scene::find_mut` returns a guard (`NodeMut`) that records what is changed; code that wrote through it keeps
  compiling, but patterns that destructured the result need `&mut node.kind`.
- The Objects panel only lays out the rows that are on screen: one frame with 21 000 objects went from 103 ms to
  0.3 ms. Selecting many objects is no longer extremely slow (the selection logic was cubic; Select All on 21 000
  shapes would have taken hours and now takes under a millisecond).
- While a clip plays, the Scene and Game views share one preview copy of the scene instead of making one each.
- Projects larger than 1 MB are written on a single line instead of indented, which is the same JSON in a tenth of
  the space: a map of 21,000 shapes went from 382 MB to 58 MB, loading from 459 to 119 ms and saving from 531 to 85 ms.
  Small projects stay indented. No format version change, and files of both kinds open in every version.
- The renderer skips shapes, opacity layers and clip groups that are outside the picture; zoomed-in views of big
  scenes draw several times faster (close-up of a map of 21,000 shapes: 63 to 10 ms). The picture is unchanged.
- *Export Game (desktop)* on macOS now writes a `.app` bundle (the player untouched, the project as a resource, ad-hoc
  signed). The previous approach, appending the project to the program, broke its code signature.
- The CLI help and the error messages about the web player no longer assume Windows (`.exe`, `.ps1`).

### Fixed
- Crash-recovery files no longer land in the temp folder on macOS and Linux (where the system may delete them).

### Checked by hand (2026-10-13)
- By the author: the [MANUAL_TESTS.md](MANUAL_TESTS.md) lists for scenes and cameras, renaming and duplicating, and PNG
  sequence export, plus the Pen, Pencil, Bézier handle and IK target dragging tests. The trackpad navigation section on a Mac trackpad. Still to check: the mouse wheel on Windows.
- macOS: the editor builds and starts; an exported `.app` passes strict signature validation, launches with `open` and plays
  its game; all workspace tests pass.
