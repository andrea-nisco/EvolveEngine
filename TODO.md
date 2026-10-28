# EvolveEngine: plan and open items

**The goal:** a complete, free, open-source (MIT) 2D game editor that people can start from and then *evolve*: change
the editor itself, together with their game, to fit what that game needs. So the plan has two halves: a base that is
complete enough to really use, and an editor that is built to be changed.

`- [ ]` open · `- [x]` done · **(!)** worth doing first · *italics* = a known limit that is a deliberate trade-off, not a bug

## The plan, in order

### Current track: make the editor handle the IronKrieg map (started 2026-10-13)

Why: IronKrieg (a grand-strategy game, 1 234 provinces and 20 206 border lines) is the real, large project used to
stress-test and then evolve the editor. The goal is a game that is fast on every platform, built as vectors (map,
coats of arms, soldiers). The old Unity plan lives in `Desktop\IronkriegEvolveEngine\RustPort`; we follow it only where
it fits and change anything else. Measure first, fix what the numbers show, then build the GPU renderer, then the game.
Numbers and how to reproduce them: [BENCHMARKS.md](BENCHMARKS.md).

**Done**
- [x] `ironkrieg_import` (map export to a project) and `evolve_editor --benchmark`.
- [x] Culling in the renderer (close-up of the map 63 to 10 ms).
- [x] Compact project file for big projects (382 MB to 58 MB; load 0.46 to 0.12 s).

**Next, in this order**
1. [x] Editor costs on the map (2026-10-15): the Objects panel only lays out visible rows (103 to 0.3 ms), selecting many
       objects is linear instead of cubic (Select All: hours to under 1 ms), the clip preview is made once per frame.
       The clone only ever happened while a clip plays, not every frame as first assumed.
       **Still open:** the undo check compares the whole scene on every repaint (3.6 to 4.1 ms); fixing it needs real
       change tracking (revision counters or shared paths). Also the selection bounds (about 10 ms with everything
       selected) could be cached.
2. [x] Re-open the map in the editor window and note what still lags (2026-10-16, twice). First try with the GPU renderer:
       very slow; causes found by timing the real editor frame and fixed (quadratic overlays and play mode, geometry budget
       at a change of zoom, no culling; see the plan's progress log of 2026-10-16). Second try: "100 times smoother", a
       little lag remains now and then. **Next:** find out when (zoom crossing a bucket = P12, mouse release after a drag =
       undo snapshot ~10 ms, autosave = step 3) using the worst-frame figure in the status bar.
3. [ ] Autosave off the UI thread (85 ms every 30 s today).
4. [ ] GPU renderer on wgpu (backend agreed 2026-10-15). The detailed plan, with phases 0 to 8, design decisions,
       risks and how each phase is verified, is in [GPU_RENDERER_PLAN.md](GPU_RENDERER_PLAN.md), which is also the checklist:
       tick it and update its logs as work is done. Scope: engine and editor only; IronKrieg is a test scene. Status
       2026-10-16: Phases 1 to 4 are done (change tracking, the renderer, gradients and thin lines, clips, layers and symbol
       instances): the GPU renderer draws everything the CPU renderer does except skinned shapes, and the complete illustration
       scene takes about 17 ms instead of 2.8 s. Still open from them: 3.1 and the "After Phase 4" checks for the owner (MANUAL_TESTS.md),
       4.8 and 2.12/2.13 on the T2 laptop and in the browser (OPEN_POINTS.md P2), 3.10 (P4). Next action: Phase 5 (speed at scale:
       the walks over unchanged views, zoom-bucket hitches, true instancing, layer reuse), or Phase 6 (skinned shapes) first.
5. [ ] Then the game: borders and constant-width strokes (done), coats of arms as SVG (first look done 2026-10-27: random
       shields over the capitals, fixed 100 km size; still to do: follow the zoom and fade as in the Unity game, shadow, editing a
       shield, the player's own shield), text and UI (a plan of its own, to write), picking provinces (needs a spatial
       index; today a click takes about 7 ms), soldiers.

**Open questions for the owner**
- The open questions in section 8 of [GPU_RENDERER_PLAN.md](GPU_RENDERER_PLAN.md) (performance bar and machine,
  format change for pixel-width strokes, importance of the web version, when the GPU renderer becomes the default).
- ~~Where should the imported map and its generated files live?~~ Answered 2026-10-15: `local/` in the repository root,
  which is in `.gitignore` (OPEN_POINTS.md, P7).

### Phase 1: open-source foundation (before announcing anything)

**Done**

- [x] MIT licence ([LICENSE](LICENSE)), `license` in every Cargo.toml.
- [x] Continuous integration ([ci.yml](.github/workflows/ci.yml)): build + test on Windows, macOS and Linux, clippy
      with warnings denied, and the wasm web player.
- [x] Prebuilt downloads: [release.yml](.github/workflows/release.yml) builds the editor, the player and the web player
      for Windows, macOS (Apple Silicon) and Linux on a `v*` tag and attaches them to a **draft** release for you to
      review; [tools/package-release.sh](tools/package-release.sh) does the packing (checked on the Mac: the package
      unpacks, keeps its executable bits, and its editor exports a game).
- [x] macOS: builds and passes the tests; autosave / layout folder follows each platform's convention
      (`~/Library/Application Support`, `~/.local/share`, `%LOCALAPPDATA%`); `tools/build-web-player.sh`; trackpad
      navigation in the Scene view (two-finger scroll pans, pinch and `Cmd`+scroll zoom, a mouse wheel still zooms;
      checked by hand on a trackpad).
- [x] *Export Game (desktop)* on macOS writes a `.app` bundle (the untouched player plus the project as a resource,
      ad-hoc signed) instead of appending to the program, which had broken the signature. Checked on the Mac: the
      bundle passes `codesign --verify --strict`, `open` launches it and it plays its game. Unit-tested everywhere.
- [x] Project-format stability: a frozen fixture per format version
      ([tests/fixtures](crates/evolve_engine/tests/fixtures)) that must always open, a test that the writer's output for
      the current version does not change without a version bump, and the rules in CONTRIBUTING.md and the README.
      Checked by deliberately breaking the format twice (both caught). *Fixtures v1 and v2 are reconstructed from v3, not
      genuine old files; if you have real ones from early editors, add them next to these.*
- [x] Community files: [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), issue forms (bug
      report, feature request), a pull-request checklist, [CHANGELOG.md](CHANGELOG.md), a README for newcomers.
      *Private conduct reports go to the repository owner's profile contact; replace that with a dedicated address or
      GitHub's private reporting if you prefer.*
- [x] Hand tests (2026-10-13, reported by the author): [MANUAL_TESTS.md](MANUAL_TESTS.md) passes, and the Pen tool,
      Pencil tool, Bézier handle dragging and IK target dragging work.
- *Decisions:* no Intel Mac build (Apple is retiring Intel Macs); no code signing or notarization for now, so the
  SmartScreen / Gatekeeper warnings described in the README stay.

**Open: needs a person or another machine**

- [ ] **(!)** Push to a public repository, then fix whatever the first CI run reports (Linux packages and versions in
      particular), try the release workflow with "Run workflow" before the first tag, and see that the issue forms show up.
- [ ] Run the editor on Linux by hand (never done) and fix what shows up.
- [x] On Windows, once: the mouse wheel still zooms in the Scene view (the trackpad change touched that code). *(Checked by the owner, 2026-10-16: zooming with the wheel in the Scene view works.)*
- [ ] On the Mac, once: *File → Export Game* from the editor's own save dialog (the dialog is not covered by tests;
      the "Exporting a game" section of [MANUAL_TESTS.md](MANUAL_TESTS.md)).
- [ ] Optional: map the trackpad's two-finger twist (egui reports it as a rotation) to rotating the view, so a Mac never
      needs a right button; the rotation direction has to be checked on a real trackpad.

### Phase 2: extensibility (what makes this product different)

Today panels and tools are closed lists in the code ([layout.rs](crates/evolve_editor/src/layout.rs) `Pane`,
[tools.rs](crates/evolve_editor/src/tools.rs) `Tool`), scripts only run inside the game, a project is one JSON file and
an object has only the built-in physics body and one script. None of that lets a user change the editor.
The decision: **a clean Rust core (fork and recompile works) plus editor extensions written in Rhai that travel with the
project.** Keep the core small and build features such as tilemaps and particles as first-party extensions, which proves
the system and documents it.

1. [ ] **(!)** Registries: panels, tools, menu commands, inspector sections and asset kinds register themselves instead
       of being enum variants; the built-in ones become the first users. Do this first: every feature added before it
       makes it more expensive.
2. [ ] **(!)** Project folder format: the project JSON plus `assets/` and `extensions/` next to it (a single `.evolve`
       file keeps working). Also the natural place for images, sounds and shared scripts.
3. [ ] Editor scripting API: Rhai extensions that add menu commands, panels with simple UI, tools and batch operations
       on the scene; loaded from the project's `extensions/` folder, hot-reloaded, errors shown in the Console.
4. [ ] Custom components / properties on objects with an automatic Inspector UI (a game's own data: health, inventory,
       dialogue). Today an object cannot carry anything but the built-ins.
5. [ ] Extension manager panel, documented example extensions, a versioned and documented extension API.
6. [ ] Later, if people ask: native or WASM plugins for other languages.

### Phase 3: the baseline every game needs

- [ ] **(!)** Physics: box and polygon bodies, rotation, continuous collision detection, kinematic bodies (moving
      platforms), collision layers, one-way platforms, joints. (Details in "Phase 5" below.)
- [ ] **(!)** Audio: sound assets, a playback API for scripts, export bundling. A tool without sound cannot ship a game.
- [ ] **(!)** Input: all mouse buttons, wheel, touch (also needed for the web export), gamepad, text input.
- [ ] **(!)** Save data: globals are lost when the game closes.
- [ ] **(!)** Text and UI: there is no text object at all, and nearly every game needs labels and menus.
- [ ] **(!)** Script API gaps: timers (`after(seconds, fn)`), remove / disable nodes, parent and children access,
      `on_collision_end`, seeded random.
- [ ] Raster images (PNG / JPG) as a kind of object, for people who mix art styles.
- [ ] Benchmark the CPU renderer on 4K screens and large scenes; add a render-scale option or the GPU renderer if it
      disappoints. Profile the per-frame costs listed under "Cross-cutting" before fixing them.

### Phase 4: authoring comfort

- [ ] Align and distribute; more shape tools (rounded rectangle, polygon, star, line, arc); Node tool marquee and
      multi-anchor move; snapping while scaling, rotating and drawing.
- [ ] Timeline: zoom and scroll, multi-key selection, copy / paste keys, snap to keys.
- [ ] A project-wide asset library (symbols and clips belong to one scene today).
- [ ] Undo for scene-list changes, and an undo history that survives switching scenes.
- [ ] Script editor basics: line numbers, jump to the error, syntax highlighting.
- [ ] Play mode: hot-reload of scripts, frame step, time scale, restart without leaving Play mode.

### Phase 5: growth

- [ ] Export presets (resolution, fullscreen, window title), application icon, web loading screen and fullscreen button.
- [ ] Example projects and templates, a tutorial, a documentation site.
- [ ] Settings dialog (grid size, theme, shortcuts), shortcut cheat sheet, localization, accessibility.

### Deferred

GPU renderer (now item 4 of the current track above, after the editor-cost fixes), PDF export, Android / iOS wrappers, blend modes and group effects,
dashed strokes, gradient spread modes. Everything below is the detailed list of known limits per area; the plan above
picks from it.

---

## Done: editor structure (dockable panels, camera, scenes)

1. [x] Dockable layout: Objects | Scene | Game | Inspector across the window, Assets / Scenes / Timeline / Console
       as tabs below; drag to rearrange, saved between sessions, *Window → Reset Layout*.
2. [x] A Camera object replaces the "game screen" rectangle: it is created with every new scene, drawn as a frame in
       the Scene view, with position, rotation, size and background color in the Inspector. Old projects get a
       camera that frames their old screen.
3. [x] Objects panel with an icon per kind, a "+ Add" menu (also on right-click) and row menus (rename, duplicate,
       delete).
4. [x] A separate Game view (what the camera sees, letterboxed) with Play / Pause / From start; the game's mouse is
       the pointer over it, seen through the camera.
5. [x] Several scenes per project (file format 3; versions 1 and 2 still open): Scenes panel, scene picker, start
       scene, script functions `next_scene`, `load_scene`, `restart_scene`, `scene_name`, `get_global`, `set_global`;
       exported desktop and web games carry all scenes.

### Limits of this work

- [x] Hand-tested with [MANUAL_TESTS.md](MANUAL_TESTS.md) (section "Scenes and cameras"): the Game view while the
      Scene tab is closed, Duplicate and *Set as start scene*, deleting open and closed scenes, Play (`F5`) from a
      scene other than the start scene, the unsaved-changes prompt after scene-list changes, and saving / reopening a
      multi-scene file through the dialogs. The test found and fixed: renaming (Enter did not close the box),
      object duplicates not named "... copy", Esc not closing the delete box, and the camera-protection message
      being easy to miss. Re-run the checklist after larger changes to scenes or the file format.
- [ ] Undo history is per open scene and starts over when you switch scenes; scene-list changes (add, rename,
      reorder, delete) are not undoable. (Deleting a scene now asks for confirmation first.)
- [ ] Symbols (the Assets panel), animation clips and IK constraints belong to one scene; there is no project-wide
      asset library. Pasting between scenes copies the symbols it needs.
- [ ] Only the first camera of a scene is used; no switching between several cameras from scripts, no camera
      follow / smoothing / shake helpers, no zoom animation helper (size can be keyed only by editing it in scripts).
- [ ] The camera's scale is ignored (its size field sets the visible area); a camera cannot be resized by dragging
      its frame, only in the Inspector. The area outside the frame is not dimmed in the Scene view.
- [ ] No scene-transition effects (fade, slide), no loading of scenes in the background, no additive scenes.
- [ ] Globals hold plain script values and are lost when the game closes: there is no save data yet.
- [ ] The visual scene "flow graph" (arrows between scenes with conditions) does not exist: order plus
      `load_scene` in scripts is the whole model.
- [ ] Raster images (PNG / JPG) are not a kind of object yet: "images" still means SVG.
- [ ] The Game view has no resolution or aspect-ratio presets, no "maximize on play", and no stats overlay in the
      Scene view while playing.
- [ ] Panels cannot be torn out into separate OS windows.
- [x] The scene's only camera can no longer be deleted or cut (alone or inside a group); a scene without a camera
      (old projects) still falls back to a 1280 x 720 frame. A second camera, if one gets in through paste or
      duplicate, can be deleted.

---

## Done from the first list of picks

All six were completed in the second round of work:

1. [x] Unsaved-changes prompt (New / Open / closing the window) and autosave with crash recovery (a recovery copy
       every 30 s while there are unsaved changes; offered at the next start).
2. [x] Cut / copy / paste, paste in place, via the system clipboard so it works between editor windows and documents.
       Pasting SVG text imports the drawing.
3. [x] Creating objects from scripts: `instantiate("name", x, y)` (named `instantiate` because `spawn` is reserved in Rhai).
4. [x] PNG export (any scale, optional transparency), SVG export, PNG sequence from the current clip.
5. [x] Hands-on pass over the Node, Bone and Weights tools (see "What the hands-on test found" below).
6. [x] Web player shrunk from 8.9 MB to 6.5 MB (about 1.7 MB gzipped): size profile with LTO, and no bundled fonts.

### What the hands-on test found

- Fixed: splitting a straight edge with the Node tool gave the new anchor (and its neighbours) handles; it now inserts
  a plain corner.
- Fixed: autosave never fired while the window was idle (egui only redraws on events); it now schedules a wake-up.
- Fixed: `spawn` is reserved in Rhai, so the API function is `instantiate`.
- Verified by hand: rectangle drawing, moving / adding / deleting anchors, root and child bones, aiming a bone (children
  follow), binding a shape to bones, weight painting with the brush and heat map, copy / paste, the unsaved-changes
  prompt (Cancel and Don't save), autosave, crash recovery (Recover).
- Not reproduced: the view zoom jumped from 54% to 89% once during the session; it did not happen again in
  repeated runs of the same steps (possibly stray mouse-wheel input). Keep an eye out for it.
- Tried by hand later (2026-10-13, reported by the author): the Pen and Pencil tools, dragging Bézier handles and
  dragging an IK target all work.
- Still not tried by hand: Alt-erase in the Weights tool, Shift-move and Alt-stretch of bones, the timeline with
  Auto-key on skinned or morphing shapes.

---

## New limits introduced by that work

- [ ] Paste does not carry skins or IK constraints (they refer to bones of the original); the status line says so.
- [ ] Pasting into a different document duplicates a symbol whose contents differ from an existing one of the same id.
- [ ] Only one recovery slot exists (the most recent unsaved document); the interval (30 s) is fixed.
- [x] PNG sequence export now runs on a worker thread with a progress window and Cancel (frames already written
      stay on disk). Only one sequence can run at a time; the single-PNG export is still synchronous.
- [ ] SVG export leaves out bones, uses the current pose, and has no options (precision, embedded names, flattening
      of symbols).
- [ ] `instantiate` copies into the template's parent and has no pooling or despawn timer; copies can only be
      parameterised through variables set after creation.
- [ ] Hidden objects are inactive (scripts and physics skipped); there is no separate "enabled" flag.
- [ ] The web player has no fonts, so error messages are shown through the HTML page instead of inside the canvas.

---

## Phase 1: engine and vector renderer

Baseline numbers for the IronKrieg map (21 440 objects, 679 000 points) are in [BENCHMARKS.md](BENCHMARKS.md):
about 13 fps for the whole map at 1080p, and the same cost when only 0.5% of it is visible.

- [x] Cull objects outside the view (close-up 63 to 10 ms, region 75 to 36 ms; the whole map is unchanged, since all of it is visible).
- [ ] Cache the converted paths: measured at only 12% of the whole-map frame, so low value for the CPU renderer. Triangles for a GPU renderer are a different matter.
- [ ] Level of detail: drop points closer than about half a pixel before drawing (saves about 28% at the whole-map view; slightly changes the picture). More useful as a GPU optimization.
- [ ] Cache the bounds of each shape so culling does not look at every anchor each frame (about 3 ms for the IronKrieg map).
- [ ] Strokes with a constant width in screen pixels (the game's borders need it).
- [ ] A node with opacity below 1 is drawn into a full-screen layer of its own; many such nodes would be very slow.
- [ ] GPU renderer option for very large scenes (today: CPU tiny-skia, redrawn on every repaint; no dirty-region tracking).
- [ ] Cache rendered layers / static parts of the scene.
- [ ] Optional clipping or dimming of content outside the camera frame in the Scene view (the Game view and exports already show the frame only).
- [ ] A real engine-owned main loop abstraction (today the loop is egui/eframe's).
- [ ] Render-scale option for the player on 4K screens (CPU rendering at full resolution can be slow).
- [ ] Dashed strokes (the renderer supports them; the data model and UI do not).
- [ ] Blend modes, group blur / drop shadow.
- [ ] Gradient spread modes (repeat / reflect) and focal points for radial gradients.

## Phase 2: authoring tools

- [x] Copy / paste / cut and paste in place (done, see above).
- [ ] More shape tools: rounded rectangle, polygon, star, line, arc; a text tool.
- [ ] Node tool: drag a segment to reshape it, marquee-select several anchors, move several anchors together,
      smooth/corner/symmetric handle modes.
- [ ] Pen tool: right-click or double-click to finish consistently, continue an existing open path, join two open paths.
- [ ] Snapping: node-to-node snapping, angle snapping in the Node tool, smart guides while scaling, rotating and
      drawing (today only while moving), distribution / equal-spacing guides.
- [ ] Align and distribute commands (left, center, right, top, bottom, even spacing).
- [ ] Path operations: simplify path, offset path, outline stroke, reverse direction.
- [ ] Boolean operations keep Bézier curves (today results are flattened polylines at 0.01 units).
- [ ] Editing inside a clipping group without releasing it; nested mask UI.
- [ ] Numeric rotation field; movable pivot for rotate/scale.
- [ ] Layer panel: search, color labels, multi-select drag, solo.
- [ ] *Non-uniform scaling of a rotated shape shears it and scales its stroke (matrix transform, no stroke lock).*
- [ ] *Undo stores whole-scene snapshots (100 steps). Fine for normal scenes, memory-heavy for very large ones.*

## Phase 3: assets and file formats

- [ ] SVG import: dashes, text, filters, masks, clip paths, patterns, images are ignored.
- [x] SVG export (done). [ ] PDF export.
- [ ] Symbols: edit the master in place (today: *Edit master* makes a copy, then *Update from selection*).
- [ ] Symbols: overrides per inner object (today only one fill and one stroke color for the whole instance).
- [ ] Symbols that contain skinned shapes or bones.
- [ ] Asset browser: sounds, scripts, images and vector files (today only symbols); folders, search, rename on disk.
- [ ] Shared script files / modules (today a script lives inside one node).
- [x] Project format: big projects are written on one line (IronKrieg map: 382 MB to 58 MB, load 0.46 s to 0.12 s).
- [ ] Project format: omit zero handles (about half of the remaining size; needs a new format version), an optional
      gzip for exporting / sharing (58 MB to 4.4 MB), autosave off the UI thread (85 ms every 30 s), migrations between versions.
- [ ] Recent files list, restore view settings (grid, zoom). (Autosave, crash recovery and the panel layout are done.)
- [ ] Project folder with assets next to the `.evolve` file instead of a single JSON.

## Phase 4: animation and rigging

- [ ] Timeline: zoom and scroll, multi-key selection, copy/paste keys, box-select keys, snap to other keys.
- [ ] Animate gradient fills and symbol-instance color overrides (today only solid fills and strokes).
- [ ] Hold / step interpolation; per-key easing presets library; value curve editor (graph of a property over time).
- [ ] Motion paths (animate position along a path) and a path overlay for keyed positions.
- [ ] Clip events / markers (call a script at a time), clip blending, simple state machine.
- [ ] Onion skinning; scrubbing audio once sound exists.
- [ ] Keyed transforms drop skew (translation, rotation and scale only).
- [ ] Auto-key can add stray position keys when grouping or ungrouping (child local transforms change). Consider ignoring
      structural operations.
- [ ] Morphing: manual point-matching hints; open/closed mismatch switches at 50% instead of blending; morphing skinned shapes.
- [ ] Rigging: joint limits, bend direction (pole) for IK, IK/FK blend, several solvers (two-bone analytic, FABRIK).
- [ ] Skinning: weight smoothing, normalize, mirror, copy between shapes, bone envelopes, handle-aware weighting.
- [ ] Bone UI: colors, rename in the viewport, length handles, "auto-rig a chain" helper, bone visibility toggle.
- [ ] *A skinned shape follows its bones, not its own transform; adding or deleting its anchors invalidates the skin
      (the editor warns and asks to unbind and rebind).*

## Phase 5: physics and scripting

- [ ] Dynamic bodies other than circles (boxes, polygons) with rotation and angular velocity. *Today: circles only.*
- [ ] Kinematic bodies (moving platforms that carry other bodies) and static bodies that move.
- [ ] Continuous collision detection. *Fast bodies can currently tunnel through thin walls.*
- [ ] Collision layers / masks, one-way platforms, sensors on dynamic bodies, joints and springs.
- [ ] Different masses, sleeping bodies, better friction model.
- [ ] Solid-contact *end* event (`on_collision_end`); today only contact start and trigger enter / exit.
- [ ] Script API: remove / disable nodes by name, parent / children access, world vs local rotation and scale
      helpers, point queries (`bodies_at(x, y)`), ray cast restricted to bodies, timers (`after(seconds, fn)`),
      scene / level switching, save data, random with seed.
- [ ] Input: mouse buttons other than the primary, wheel, touch, gamepad, text input.
- [ ] Script editor: syntax highlighting, autocompletion, line numbers, jump-to-error, a debugger / breakpoints.
- [ ] Script error messages show Rhai positions inside the script text only; no link back to the node in the viewport.
- [ ] Raycasts skip symbol instances; invisible nodes are ignored.
- [ ] *Scripts get node ids as plain integers (`me`); there is no object wrapper API.*

## Phase 6: play mode and export

- [ ] Play mode: hot-reload of scripts while playing, frame step, time scale, restart without leaving Play mode,
      inspect runtime values of a selected node.
- [ ] Real profiler (the "shapes drawn" number counts shapes, not GPU draw calls, because rendering is on the CPU).
- [ ] Desktop export: application icon, installer, code signing (unsigned executables trigger Windows SmartScreen),
      version info, optional compression of the embedded project. *The project JSON and scripts are readable inside
      the exe; there is no obfuscation.*
- [ ] Web export: loading screen, further size reductions (rhai features, trimming egui; the wasm is now 6.5 MB,
      about 1.7 MB gzipped), touch input, fullscreen button, a ready-made
      local test server script, hosting guide. *Needs an http server; it cannot run from `file://`.*
- [ ] Other platforms: macOS builds and runs (checked 2026-10-13); `.app` and AppImage packaging, and the Linux
      editor run, are still to do (see Phase 1). Android / iOS wrappers.
- [ ] Audio: sound assets, playback API for scripts, export bundling.
- [ ] Export presets (resolution, fullscreen, window title) stored in the project.

## Cross-cutting

- [ ] **GUI testing:** the main paths of the Node, Bone and Weights tools were tried by hand (see above), but the
      remaining ones listed there were not. Add a manual checklist or UI-level tests. Keep GUI automation safe: confirm
      the target window is in front before every input event (the helper used for the second round does this).
- [x] Continuous integration (workflow written, see Phase 1) and git repository.
- [x] Licence file and contributing notes. [ ] Changelog.
- [ ] Example projects and a short tutorial (the built-in sample is the only example).
- [ ] **(!)** Performance, measured on the IronKrieg map: the undo check costs 3.6 ms per repaint; the clip-preview clone
      (14 ms) only happens while a clip plays (see BENCHMARKS.md). Also: thumbnails are all redrawn on any change, physics re-flattens static outlines every step.
- [x] Performance of deleting and grouping many objects (2026-10-20): `evolve_editor --benchmark` on the 21,200-shape stress scene
      deleted 4,000 selected objects in 1.3 s and grouped them in 1.4 s (the cost grew with the square of the number: `Scene::remove` and
      `move_node` once per object, each a search of the tree and a shift of the list of siblings). `Scene::remove_all` and `Scene::group`
      now take the nodes out in one walk: 3 ms and 2 ms. Tests in `remove_tests.rs` compare both with the one-at-a-time versions.
- [ ] Dependency pins: egui / eframe 0.36, `wasm-bindgen` CLI must stay equal to the crate version (0.2.129).
      Plan an upgrade routine.
- [ ] Settings dialog (grid size, theme, shortcuts), a keyboard-shortcut cheat sheet inside the app, localization.
- [ ] Accessibility: scalable UI font size, high-contrast theme.

---

*Written for the project owner and any future contributor. Keep this file up to date: tick items off when done and
add new limits as they are discovered.*
