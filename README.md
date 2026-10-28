# EvolveEngine

A vector-first 2D game editor written in Rust. Everything on screen is drawn straight from Bézier data at the
display's resolution, so artwork stays razor sharp at any zoom level.

EvolveEngine is free and open source ([MIT](LICENSE)). The idea behind it: an editor should not be fixed. It is meant
to be a solid base that you change, together with your game, to fit what your game needs (see
[Where this is going](#where-this-is-going)).

The original six roadmap phases are implemented, and a GPU renderer (wgpu) is being added next to the CPU one, to keep big
scenes fast: it is already in the editor as an option (see [The GPU renderer](#the-gpu-renderer)). See [Status](#status)
for the details and the limits, and [TODO.md](TODO.md) for the plan and for everything that was left out or simplified.

## Download

Ready-made builds for Windows, macOS (Apple Silicon) and Linux (x86-64) are attached to each release on the
repository's Releases page. Unpack the archive anywhere and start `evolve_editor`. It contains the editor, the player
(needed by *Export Game*) and the web player (needed by *Export for Web*), all in one folder: keep them together.

The programs are not code-signed yet, so the system warns on first start:
- **Windows:** SmartScreen says "Windows protected your PC": choose *More info* → *Run anyway*.
- **macOS:** "cannot be opened because the developer cannot be verified". Either right-click the program → *Open*, or
  remove the download mark once: `xattr -dr com.apple.quarantine <the unpacked folder>`.
- **Linux:** nothing to do; if your file manager dropped the executable bit, `chmod +x evolve_editor evolve_player`.

## Build from source

Runs on Windows, macOS and Linux. Requires the [Rust toolchain](https://rustup.rs), plus:
- Windows: the MSVC C++ Build Tools.
- macOS: the Xcode command line tools (`xcode-select --install`).
- Linux: a C compiler and the usual window-system development packages
  (on Debian / Ubuntu: `sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev`).

```
cargo build --release --workspace     # the editor and the standalone player
cargo run -p evolve_editor --release
cargo test --workspace
cargo clippy --workspace --all-targets   # kept free of warnings
```

The GPU tests open a graphics device without a window. On a machine with no graphics adapter they print "skipped" and pass
(set `EVOLVE_REQUIRE_GPU=native` to make a missing adapter a failure, as the CI job on Linux does). Anything that uses the card
follows [GPU_SAFETY.md](GPU_SAFETY.md).

For *Export for Web*, build the browser player once (needs `rustup target add wasm32-unknown-unknown` and
`cargo install wasm-bindgen-cli --version 0.2.129 --locked`):

```
powershell -ExecutionPolicy Bypass -File tools/build-web-player.ps1   # Windows
tools/build-web-player.sh                                              # macOS, Linux
```

## Using the editor

**The window** is made of dockable panels, laid out like most game editors, left to right: **Objects** (every object
of the scene, as a tree), **Scene** (where you edit), **Game** (what the player sees), **Inspector** (properties of the
selection), with **Assets**, **Scenes**, **Timeline** and **Console** as tabs along the bottom. Drag a panel's tab to
move it (drop it on the edge of another panel to split, or on its tab bar to stack), drag the dividers to resize, and
close a panel with its x. *Window* lists all panels (click to open or close one) and *Window → Reset Layout* restores
the default. Your layout is remembered between sessions.

**Camera:** every scene has a **Camera** object (listed in Objects, drawn in the Scene view as a thin frame labelled
with its name). The frame is what the player will see: the game window shows exactly that area, with bars if the
window has a different shape. Click the frame's border to select the camera, then move or turn it like any object, or
use the Inspector for its position, rotation, size (*W* and *H*, in world units: only the ratio matters on screen) and
background color. Put a camera inside a group, or move it from a script (`set_pos`), to make it follow something.
The first camera in the Objects list is the one used; a scene without any camera shows a default 1280 x 720 frame.

**Renderer and frame times:** the Scene and Game views are drawn by the CPU renderer by default. *View → Renderer* (or the
*Renderer* box in the status bar, or `--renderer auto|cpu|gpu` on the command line) switches to the GPU renderer, see
[The GPU renderer](#the-gpu-renderer). The status bar shows how fast the editor is running: frames per second while frames
follow each other, the average and the worst time of the last 120 frames, and where the time went (hover for the details;
*View → Show frame times* turns it off). *View → Frame rate* caps the rate while a game runs or a clip plays, to see how a game
feels at a lower rate or to spare the card; an idle editor draws nothing.

**Creating objects:** the *+ Add* menu at the top of the Objects panel (also on right-click in its empty space) adds
an empty group, a rectangle, an ellipse or a camera in the middle of your view, and imports SVGs. Everything you
create, draw or import appears in the Objects list with an icon for its kind (camera, rectangle, ellipse, path,
group, symbol instance, bone). Right-click a row for Rename, Duplicate and Delete. The pin at the end of a row selects that object and takes the Scene view to
it, which is the way to find one object among thousands.

**Scenes:** a project holds any number of scenes, edited one at a time. The drop-down at the top of the Objects panel
and the **Scenes** tab switch between them; in the Scenes tab, *+ New scene* adds one, click opens, double-click
renames, and right-click offers Duplicate, *Set as start scene*, Move up/down and Delete. The list order is the play
order. Each scene has its own objects, symbols, animation clips and camera, and its own undo history (switching
scenes starts a fresh history). Scripts move between scenes: `next_scene()` goes to the one below the current scene,
`load_scene("Level 2")` goes to a scene by name, `restart_scene()` starts the current one again, and
`set_global("score", 10)` / `get_global("score")` keep values across scenes (see Scripts).

| Tool (key) | What it does |
|---|---|
| Select (`V`) | Click to select, drag to move, drag empty space for a box selection. `Shift` adds. Double-click picks a shape inside a group. Handles scale (`Shift` keeps proportions, `Alt` scales from the center) and rotate (`Shift` = 15° steps). |
| Node (`A`) | Drag anchors and Bézier handles (`Alt` breaks handle symmetry). Double-click a segment to add an anchor; double-click an anchor to switch it between corner and smooth. `Del` removes the selected anchor. |
| Pen (`P`) | Click for corner points, drag for curves. Click the first point to close the path. `Enter` finishes, `Esc` cancels. `Shift` snaps to 15°. |
| Rect / Ellipse (`R` / `E`) | Drag to draw. `Shift` = square/circle, `Alt` = from center. |
| Pencil (`N`) | Draw freehand; the stroke is simplified into smooth curves. |

**View:** mouse wheel zooms at the cursor · middle-drag or `Space`+drag pans · right-drag rotates the view · on a
trackpad (a Mac, or a precision touchpad) two-finger scroll pans, pinch or `Cmd`/`Ctrl`+scroll zooms ·
`Home` fits the camera frame · `F` zooms to the selection.

**Edit** (on macOS read `Ctrl` as `Cmd`)**:** `Ctrl+Z` / `Ctrl+Shift+Z` undo/redo · `Ctrl+X` / `Ctrl+C` / `Ctrl+V` cut, copy, paste (`Ctrl+Shift+V`
pastes in place; copies also go to the system clipboard, so you can paste into another editor window, and pasting
SVG text imports the drawing) · `Ctrl+D` duplicate · `Ctrl+G` / `Ctrl+Shift+G` group/ungroup ·
`Ctrl+J` combine into a compound path · `[` / `]` send backward / bring forward · `Del` delete.

**Object menu:** *Shape Operations* (union, subtract, intersect, exclude) combine the selected shapes; the lowest
shape is the base and keeps its style. *Make Clipping Mask* (`Ctrl+7`) groups the selection and uses the top-most
shape as a mask; *Release* is `Ctrl+Alt+7`. *Release Compound Path* (`Ctrl+Shift+J`) splits a compound path into
separate shapes. With the Node tool, `X` cuts a path open at the selected anchor. While moving, pink smart guides
show alignment with other objects (toggle in the View menu).

**Symbols (prefabs):** select artwork and choose *Object → Create Symbol from Selection* (or *Import SVG as Symbol*).
The artwork becomes a master shown in the **Assets** panel at the bottom, and the selection turns into an instance of
it. Drag a card onto the canvas (or double-click it) to place more instances. All instances share the master's
geometry, but each has its own position, scale, rotation and optional fill/stroke color override (Properties panel).
To change the master: *Edit master* puts an editable copy on the canvas; edit it, keep it selected, then press
*Update from selection*. *Detach* turns an instance into an ordinary group.

**Animation (Timeline tab, bottom):** a clip is a set of tracks, one per object property (position, rotation,
scale, opacity, solid fill color, solid stroke color), each with keyframes. The scene always shows the pose at the
current time, so the normal tools edit what you see.
- *Add keyframe* keys the selected objects at the playhead. Turn on **Auto-key** and any move, rotate, scale or
  opacity edit is recorded as a key at the current time (the first key on a track also stores the starting value
  at 0 s). One `Ctrl+Z` undoes the edit together with its keys.
- Click the ruler or drag to scrub. Click a diamond to select a key, drag it to retime it, `Del` removes it.
- A selected key shows its time, its value and an **easing curve editor**: pick a preset or drag the two
  Bézier handles. The easing shapes the move from that key to the next.
- *Play* previews the clip (loop optional) without changing the scene. Several clips per project are supported.
- Rotation and scale pivot around an object's origin. Newly drawn shapes have it at their center; use
  *Object → Set Origin to Center* for others.

**Morphing:** key the **Shape (morph)** property (*Add keyframe → Shape*, or edit anchors with Node tool while
Auto-key is on) and the outline blends from one key's shape to the next. Shapes with different anchor counts are
matched automatically (extra anchors are added without changing the look, start points are aligned).

**Rigging (Bone and Weights tools, `B` / `W`):**
- *Bone tool:* drag in empty space to add a bone; drag from a bone's tip to add a child; drag a bone's body to aim it
  (`Shift` moves, `Alt` also stretches). Bones are ordinary nodes, so they appear in the scene tree, can be keyed
  (rotation is the usual one) and are invisible in playback.
- *Skinning:* select a shape and some bones (shift-click in the scene tree), then *Object → Bind Shape to Selected
  Bones*. Anchors get distance-based weights, so the outline bends smoothly as bones turn. The *Weights tool*
  paints the active bone's influence (`Alt` erases); the Skin section of the properties panel picks the bone and
  brush. A skinned shape follows its bones, not its own transform. *Unbind skin* restores the plain shape.
- *IK:* select a tip bone and a target object, then *Object → Create IK*. The chain (default 2 bones, adjustable)
  reaches for the target, which you can drag or animate with keyframes.

**Running the game (Game menu, or the buttons of the Game panel):** Play runs a *copy* of your project as a game in
the **Game** panel, so the project you are editing is never changed; the Scene panel keeps showing the running scene
(editing is paused until you stop). `F5` runs/stops the game from the scene you are editing, `Shift+F5` runs it from
the project's start scene (what an exported game does), and `F6` pauses. While the game runs, the mouse position your
scripts read is the pointer over the Game panel, seen through the camera. Debug overlays (Game menu): collision
shapes (green = static, yellow = trigger, cyan = dynamic), path anchors, and statistics (scene name, fps, shapes drawn,
bodies, scripts). The **Console** tab shows `print()` output and script errors. `evolve_editor --play` starts the game
on launch. The sample project has two scenes: a title screen (press `Space`, or click the button) that leads to a
small game: arrow keys roll the ball, `Space` jumps, `R` resets, `Esc` goes back to the title, and the sun dims
while the ball passes through it.

**Physics:** select an object and tick *Collision body* in the properties panel.
- *Static*: solid and fixed. Any outline works, concave ones included (hills, bowls, walls).
- *Dynamic*: a moving circle (sized from the shape) with gravity, bounce and friction; it collides with static
  bodies and other dynamic bodies.
- *Trigger*: doesn't block anything, but reports when a dynamic body enters or leaves.

**Scripts:** select an object and press *Add script*. Scripts are written in [Rhai](https://rhai.rs) (a small,
JavaScript-like language). Every handler gets the object's id as `me`:

```
fn on_init(me) { }                      // once, when the game starts
fn on_update(me, dt) { }                // every frame
fn on_input(me, key) { }                // a key went down: "Space", "Left", "A", ...
fn on_collision(me, other) { }          // a dynamic body hit something solid
fn on_trigger(me, other, entered) { }   // entered a trigger (true) or left it (false)
```

Functions you can call: `find("name")`, `exists(id)`, `destroy(id)`, `instantiate("name", x, y)` (see below) ·
`pos_x/pos_y(id)`, `set_pos(id, x, y)`,
`move_by(id, dx, dy)`, `rotation/set_rotation(id, degrees)`, `scale_x/scale_y/set_scale` ·
`opacity/set_opacity`, `set_visible`, `set_fill(id, r, g, b)`, `set_stroke(id, r, g, b)`, `set_stroke_width(id, w)` ·
`velocity_x/velocity_y(id)`,
`set_velocity`, `add_impulse`, `set_gravity(gx, gy)`, `overlaps(a, b)`,
`raycast(ox, oy, dx, dy, max)` (returns `#{hit, id, x, y, nx, ny, distance}`) ·
`key_down("Left")`, `key_pressed("Space")`, `mouse_x()`, `mouse_y()`, `mouse_down()`, `mouse_right_down()`,
`mouse_right_pressed()`, `mouse_right_released()`, `mouse_wheel()` (how far the wheel moved this frame, positive up) ·
`main_camera()`, `camera_width()`, `camera_height()`, `set_camera_height(h)`, `set_camera_size(w, h)` (a game can zoom its own
camera; the editor's Game view gives the game the wheel and the right button) ·
`play("clip")`, `play("clip", true)` (loop), `stop_animation()` · `get_var(id, "name")`, `set_var(id, "name", v)`
(per-object storage that survives between handlers) · `next_scene()`, `load_scene("name")`, `restart_scene()`,
`scene_name()`, `get_global("name")`, `set_global("name", v)` · `time()`, `random()`, `print(...)`.

**Switching scenes from a script:** the change happens at the end of the frame, and the new scene starts from
scratch (its objects, per-object variables and animation are new). What carries over is the *globals*
(`set_global` / `get_global`, a missing one reads as `0`), the Console and the clock. `next_scene()` on the last scene,
or `load_scene` with an unknown name, only writes an error to the Console.

A script that fails is reported in the Console and stops (the others keep running); runaway loops are cut off.

**Creating objects while the game runs:** `instantiate("Bullet", x, y)` copies the object named `Bullet` (with its
shape, physics body and script) to that position and returns the new id, or `-1` if there is no such object. If there
is no object with that name, a symbol with that name is placed instead. A common pattern: draw a `Bullet`, give it a
body and a script, and hide it (eye icon). **Hidden objects are inactive templates**: their scripts and physics do not
run until the object is shown. The copies are visible and active at once, and their `on_init` runs on their first
frame. (The function is called `instantiate` because `spawn` is a reserved word in Rhai.)

**Exporting a game (File menu):**
- *Export Game (desktop)...* writes one standalone program (`.exe` on Windows): the player program with your project
  built into it. Send it to anyone; it needs no editor and no project file. `F11` toggles fullscreen. The game is
  scaled to fit the window. On macOS the result is an application bundle (`game.app`, a folder that Finder shows as one
  app). It is signed ad hoc, so it opens normally on your Mac; people who download it get the same "developer cannot
  be verified" warning as for the editor (right-click → *Open*, or `xattr -dr com.apple.quarantine game.app`).
- *Export for Web (HTML)...* writes a folder with `index.html`, `evolve_player.js` and `evolve_player_bg.wasm`.
  Serve it over http to play (browsers don't allow WebAssembly modules from `file://`), for example with
  `python -m http.server` in that folder, or upload it to any static web host.
- Both need the player to be built once (see below). The editor finds it next to its own executable
  (`evolve_player.exe` on Windows, `evolve_player` elsewhere) and in `dist/web-player`.
- The browser player is about 6.5 MB (about 1.7 MB when your web host compresses it, which most do). It is built
  with a size-optimized profile and without egui's bundled fonts, since a game only draws vectors.
- From a script: `evolve_editor --export-desktop game.evolve out`, `evolve_editor --export-web game.evolve out-folder`,
  `evolve_editor --export-png game.evolve out.png [scale]`, `evolve_editor --export-svg game.evolve out.svg` (these two
  use the project's start scene) and `evolve_editor --sample demo.evolve` (writes the demo project). Also:
  `--renderer auto|cpu|gpu` and `--quality auto|high|balanced|low` (see below), `--gen-scene large|illustration|sprites out.evolve`
  (writes a big benchmark scene that needs no game files) and `--benchmark project.evolve report` (measures rendering and
  editing cost, see [BENCHMARKS.md](BENCHMARKS.md)).
  `evolve_player game.evolve` plays a project file directly. An exported game contains all scenes and starts at the
  start scene.

**Exporting pictures (File → Export Picture):** these use the scene you are editing.
- *PNG Image* renders what the camera sees at 0.1x to 8x, optionally with a transparent background.
- *SVG Drawing* writes all visible objects as an SVG: groups, clipping masks, gradients, symbol instances with their
  color overrides, and skinned shapes as they currently look. Bones are left out.
- *PNG Sequence (current clip)* renders every frame of the selected animation clip (`name_0001.png`, ...) at the
  clip's frame rate, with IK solved. The frame count is `duration x fps`, so a looping clip repeats seamlessly.

**Files:** `Ctrl+N/O/S`, `Ctrl+Shift+S`. Projects are saved as readable JSON (`.evolve`), one file with all scenes. Files
from older versions (one scene, or a screen size and color instead of a camera) open as usual: they get a camera
that frames the old screen. *File → Import SVG* brings
in paths, groups, strokes and gradients. A title ending in `*` means unsaved changes: New, Open and closing the
window ask whether to save first. While you have unsaved changes the editor also writes a recovery copy every 30
seconds (in `%LOCALAPPDATA%\EvolveEngine` on Windows, `~/Library/Application Support/EvolveEngine` on macOS,
`~/.local/share/EvolveEngine` on Linux); if the editor ever closes unexpectedly, the next start offers to recover it.

**Project file versions:** your projects will keep opening. The format has a version number, and every new editor opens
files of all older versions and upgrades them when it loads them (an editor never silently loses an old project).
The reverse is not true: a file saved by a newer editor is refused by an older one, with a message.
Version 1: one scene, screen size and color on the scene · 2: a camera object replaces the screen · 3 (current):
several scenes per project.

The Objects panel shows the scene tree (drag rows to re-order or re-parent, eye/lock toggles, double-click to
rename) and the Inspector shows the properties of the selection: position and size, opacity, fill and stroke (solid,
linear or radial gradient, width, join, cap, fill rule). With nothing selected it shows the scene and its camera.

## Layout

```
crates/
  evolve_engine/   engine library, no UI dependencies
    scene.rs        node tree: groups, shapes, symbol instances, cameras; grouping, hit-testing
    project.rs      a project: ordered scenes and the start scene
    game_camera.rs  the camera object: frame, view matrix, upgrading old projects
    path.rs         Bézier paths (anchors + handles), flattening, splitting
    style.rs        colors, gradients, stroke settings
    camera.rs       the editor's pan / zoom / rotate view, world <-> screen
    animation.rs    clips, tracks, keyframes, easing, pose sampling
    morph.rs        blending between paths
    physics.rs      collision queries, bodies, triggers, dynamics
    clipboard.rs    copy / paste of objects, spawning copies at runtime
    export_image.rs PNG rendering, animation frames, SVG export
    script.rs       the Rhai scripting API
    runtime.rs      the game loop: scripts + physics + animation on a scene copy
    rig.rs          bones, IK solver, skinning and weight painting
    symbol.rs       symbols (prefabs) and instances
    boolean.rs      union / subtract / intersect / exclude
    render.rs       tiny-skia rasterizer (the CPU renderer, the reference)
    revision.rs     change tracking: what changed in a scene since a renderer, the undo history or an index last looked
    io.rs           .evolve project files, SVG import (usvg)
  evolve_gpu/      the GPU renderer (wgpu), also built for the browser
    renderer.rs     draws a scene on the card; reuses the last frame when only the view moved
    tessellate.rs   tess_cache.rs  draw_list.rs  curves to triangles (made once per zoom level), what is drawn and in which order
    layers.rs       targets.rs     groups with opacity and clip groups, the offscreen pictures they use
    profile.rs      quality profiles (High, Balanced, Low) and the governor that picks one
    egui_glue.rs    shows the pictures inside egui (the editor and the desktop player)
    device.rs  errors.rs  a device without a window, graphics errors that never crash the editor
  evolve_player/   standalone player (desktop exe and WebAssembly) and the export/bundle code
    bundle.rs       appends a project to the player exe; writes the web folder
    app.rs          the player window; input.rs keyboard/pointer -> engine input
  evolve_editor/   egui application
    app.rs          editor state, commands, shortcuts
    layout.rs       the dockable panels (egui_dock), saved layout
    canvas.rs       Scene view: navigation, tools, render target
    game_view.rs    Game view: what the camera sees, play controls, the game's mouse
    scenes_ui.rs    the Scenes panel and switching between scenes
    objects.rs      Objects panel: icons, "+ Add" menu
    overlay.rs      grid, camera frame, selection box, handles, tool previews
    select_tool.rs  node_tool.rs  draw_tools.rs  freehand.rs
    inspector.rs    scene tree and property editor (Inspector, camera properties)
    assets.rs       asset browser (symbols)
    timeline.rs     anim.rs   timeline UI; playback, auto-key, pose sync
    rig_ui.rs       bone and weights tools, rig overlays and properties
    game.rs         Play mode, debug overlays, console, physics/script properties
    dialogs.rs      unsaved-changes / recovery / export dialogs; autosave.rs  crash-recovery copy
    clipboard_ui.rs export_ui.rs   cut/copy/paste and picture export commands
    panels.rs       menu, toolbar, status bar
    history.rs      undo/redo
    renderers.rs    which renderer draws the views, and the fallback to the CPU one
    frame_stats.rs  frame_limit.rs   frame times in the status bar, the frame-rate cap
    cli.rs          bench.rs  bench_scene.rs   command-line actions, --benchmark, --gen-scene
tools/
  build-web-player.ps1 / .sh   builds the browser player used by Export for Web
  web-canary/ build-web-canary.ps1   a browser page that runs the GPU renderer and reports what the browser offers
  package-release.sh           packs a release download
```

Rendering is done by [tiny-skia](https://github.com/linebender/tiny-skia) on the CPU with analytic anti-aliasing,
at physical-pixel resolution (so HiDPI screens get the full detail), and optionally on the graphics card (below). The UI is
[egui](https://egui.rs). The engine crate has no dependency on the editor, so a game runtime can reuse it as is.

## The GPU renderer

The CPU renderer redraws every visible shape on every repaint, which is too slow for a very large drawing (a map of 21,000
shapes and 680,000 points takes about 70 ms a frame at 1080p). The GPU renderer ([wgpu](https://wgpu.rs): Vulkan, Metal,
DirectX 12 and, for the browser, WebGL2 and WebGPU) makes triangles from the curves once for each zoom level, draws only what
can reach the view, and draws again only what changed. On the large benchmark scene (21,200 shapes) a frame takes about 8 ms
instead of 110 ms.

- **How to use it:** *View → Renderer → GPU* (or Auto), or `evolve_editor --renderer gpu`. Today the default is still the CPU
  renderer. *Quality of the GPU renderer* (Automatic, High, Balanced, Low; `--quality`) sets how much memory the renderer may take and
  how much time it spends on a view: Automatic picks from the graphics card and lowers itself while frames are slow, and
  Low also draws at a reduced resolution, a little softer.
- **What it draws:** fills, strokes (joins, caps, thin lines), linear and radial gradients, opacity, clip groups, symbol
  instances and skinned shapes, with the same picture as the CPU renderer. Dashes, blend modes, masks, filters, patterns,
  images and text are not in the engine yet (see the plan).
- **The CPU renderer stays the reference and the fallback.** A view that the GPU renderer cannot draw completely, a machine
  with no usable graphics device, a lost device or a card error all fall back to the CPU renderer with a small notice in the
  view; the editor does not crash. The rules the code follows are in [GPU_SAFETY.md](GPU_SAFETY.md).
- **Where it is going:** [GPU_RENDERER_PLAN.md](GPU_RENDERER_PLAN.md) has the plan phase by phase, with the measurements and
  the open decisions; [BENCHMARKS.md](BENCHMARKS.md) has the numbers; [MANUAL_TESTS.md](MANUAL_TESTS.md) lists what needs a person
  at the keyboard; [OPEN_POINTS.md](OPEN_POINTS.md) lists what waits for a person or a machine.

## Status

| Roadmap item | State |
|---|---|
| 1.1 Application shell, scene graph | Done (the loop is egui/eframe's: input, update, render each frame) |
| 1.2 Paths, curves, shapes, fills, strokes (joins, caps), linear/radial gradients, anti-aliasing | Done |
| 1.3 Pan / zoom / rotate camera, world ↔ screen conversion | Done |
| 2.1 Selection, multi-select, transform handles, grid snapping, smart guides | Done. Guides align a moving selection to other objects and the camera frame |
| 2.2 Pen, shapes, freehand, anchors/handles, path cutting, boolean operations | Done. Boolean results are polylines (curves are flattened to 0.01 units), not Béziers |
| 2.3 Scene tree, order, group, hide, lock, re-parent, compound paths, clipping masks | Done |
| 3.1 SVG import, native format | Done. Dashes, text, filters, masks and clip paths are ignored on import |
| 3.2 Prefabs / symbols with overrides, asset browser | Symbols and the asset browser are done (overrides are fill and stroke colors, plus each instance's own transform). Sound files and scripts are not listed yet because there is no runtime to use them |
| 4.1 Timeline, keyframes (transform, opacity, fill/stroke color), easing curve editor | Done. Gradients and symbol-instance overrides can't be keyed yet, and skew is dropped from keyed transforms |
| 4.2 Path morphing, bone rigging, IK, weight binding and painting | Done. IK is a simple iterative solver without joint limits, and morph pairing is by index and nearest anchors (no manual point matching) |
| 5.1 Vector-path collision (ray casts, segment intersection, point-in-polygon, overlap tests), simple rigid bodies and triggers | Done. Dynamic bodies are circles (no rotation); static and trigger bodies use exact outlines |
| 5.2 Scripting runtime, node/animation/input/transform API, `on_init` / `on_update` / `on_collision` / `on_input` | Done (Rhai) |
| 6.1 Play / Pause / Stop in the viewport, debug overlays | Done (colliders, anchors, fps and draw statistics) |
| 6.2 Export: standalone desktop and web (WebAssembly/HTML5) builds | Done. Desktop = player + project in one exe; web = folder with index.html, JS and WASM (about 6.5 MB, 1.7 MB gzipped) |
| GPU renderer (extra, [plan](GPU_RENDERER_PLAN.md)) | In the editor as an option, the CPU renderer is still the default. Draws everything the CPU renderer draws except two rare cases (a skinned clip mask, a skinned shape inside a symbol instance), which fall back with a notice. Runs in a browser (the web canary), but the player and the web export still use the CPU renderer. Dashes, blend modes, masks, filters and text are later phases |
| Change tracking, benchmarks, CI (extra) | The scene knows what changed (undo, renderer and picking use it); `--benchmark` and `--gen-scene`; CI builds and tests on Windows, macOS and Linux, with a GPU job on Linux that is prepared but has not run yet |
| Editor structure | Dockable Objects / Scene / Game / Inspector panels, a camera object per scene, and several scenes per project with script-driven transitions (added after the roadmap, see [TODO.md](TODO.md) for the limits) |

## Where this is going

The goal is a complete, free editor that people can start from and then *evolve*: add panels, tools, inspector fields
and commands that their own game needs, and keep those changes together with the game's project. Today the editor is
a fixed set of features; making it extensible is the next big step. The plan is in [TODO.md](TODO.md), what changed is in
[CHANGELOG.md](CHANGELOG.md), and the way to help is in [CONTRIBUTING.md](CONTRIBUTING.md) (please read the
[Code of Conduct](CODE_OF_CONDUCT.md) too).

## License

[MIT](LICENSE) © 2026 Andrea Nisco
