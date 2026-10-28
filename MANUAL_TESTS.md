# Manual test checklist

Things that unit tests do not cover. Tick them off while running the editor (`cargo run -p evolve_editor`)
and note the date and any bug next to a failed line. Untick everything to reuse the list as a regression
check after larger changes.

Status: "Scenes and cameras", "Renaming and duplicating" and "PNG sequence export" were checked by hand on 2026-10-13 and
passed. "Navigating the Scene view (trackpad and mouse)" was checked on a Mac trackpad on 2026-10-13 and passed;
the mouse wheel on Windows is still to be re-checked once. "Exporting a game" is new and open.

## Scenes and cameras

Setup: a new project with three scenes named A, B, C (Scenes tab, **+ New scene**), each with a visible
shape so they are easy to tell apart. A is the start scene.

- [X] **Duplicate**: right-click B, *Duplicate*. A copy opens, with the same contents and a different name;
  editing the copy leaves B unchanged.
- [X] **Set as start scene**: right-click C, *Set as start scene*. The "start" tag moves to C and the project
  counts as changed. *Play from start* begins in C.
- [X] **Play from another scene**: open B, press Play (`F5`). The game starts in B (not in the start
  scene); `next_scene()` from a script goes to the scene below B. Stop, and the editor is back on B.
- [X] **Delete a scene that is not open**: right-click C while B is open, *Delete*. A confirmation asks first;
  **Cancel** keeps it, **Delete** removes it and B stays open. Start scene moves to a valid scene.
- [X] **Delete the open scene**: confirm. The editor continues in a neighbouring scene with an empty undo history.
- [X] **Last scene**: with one scene left, *Delete* is disabled.
- [X] **Esc key** on the delete confirmation: with the "Delete scene" box open, press the `Esc` key on the
  keyboard. The box closes and the scene is still there. (Clicking outside the box does nothing: it stays
  until you choose Delete or Cancel.)
- [X] **Unsaved-changes prompt**, repeat these steps once for each scene-list change (add a scene, rename one,
  move one up or down, delete one, set another as start):
  1. *File → Save* so the title bar has no `*` after the file name.
  2. Make the one change. A `*` appears in the title bar.
  3. *File → New* (or *Open*, or close the window). A box "Unsaved changes" must appear with
  Save / Don't save / Cancel. Press Cancel.
  4. If no box appears for a change, that change is not marking the project as modified: note which one.
- [X] **Save and reopen with the dialogs**: *File → Save As*, close the editor, reopen with *File → Open*. All
  scenes, their order, names and the start scene are back, and the open scene is the start scene.
- [X] **Game view with the Scene tab closed**: close the Scene tab, leave the Game view open. It still draws,
  Play works and the mouse works through the camera.
- [X] **Camera protection**: select the camera and press Delete, use *Edit → Delete*, the row menu *Delete* in
  the Objects panel, and *Cut*. Each shows a message "A scene needs a camera, so it was not deleted" in a
  box near the bottom of the window for a few seconds, and the camera stays.
- [X] Group the camera with a shape and delete the group: refused too. Selecting the shape and the camera
  together deletes only the shape.

## Renaming and duplicating

- [X] Double-click a scene in the Scenes panel, type a name, press Enter: the name is saved and the box closes.
- [X] Same, but click somewhere else instead of Enter: the name is saved.
- [X] Same, but press `Esc`: the old name stays.
- [X] Rename an object (including the camera) in the Objects panel: Enter closes the box and the name stays.
- [X] Duplicate an object (`Ctrl+D` or the row menu): the copy is called "<name></name> copy". Duplicating a scene
  does the same.

## PNG sequence export

Setup: the built-in sample is enough. It opens on the "Sunset" scene whose "Idle" clip is 3 s at 30 fps
(90 frames). In the export box set the scale to 3 or 4 so each frame takes longer and there is time to press
Cancel.

- [X] *File → Export Picture → PNG Sequence (current clip)...*, then choose a folder. A progress window appears, the editor stays
  responsive (move the pointer, switch tabs) and the count goes up.
- [X] **Cancel** mid-way: the window closes, the status line says how many frames were written, and the
  files that were written are on disk.
- [X] A finished export reports "Exported N frames" and the folder holds N numbered PNGs.
- [X] Starting a second sequence while one runs is refused with a status message.

## Navigating the Scene view (trackpad and mouse)

Setup: the Scene view open on any scene. On a Mac with a trackpad (or a Windows precision touchpad, where only the
pinch and `Space`+drag lines apply, because Windows reports touchpad scrolling like a mouse wheel).

- [X] **Two-finger scroll** over the Scene view moves the view in any direction, following the fingers; letting go
  glides to a stop and does not zoom.
- [X] **Pinch** zooms at the cursor.
- [X] **`Cmd` + two-finger scroll** (up / down) zooms at the cursor.
- [X] A **mouse wheel** (a real mouse, not the trackpad) still zooms at the cursor. Using the wheel right after the
  trackpad, and the other way round, switches behaviour without sticking.
- [X] `Space` + drag with one finger (press-and-hold the trackpad button, or tap-and-drag) pans too.
- [X] Right-drag rotates the view; on a Mac that is `Ctrl` + click-drag, or a two-finger click if enabled in System Settings.

## Exporting a game

Setup: the built-in sample project, and a release build next to its player (`cargo build --release --workspace`, then
`cargo run -p evolve_editor --release`).

- [ ] *File → Export Game (desktop)...* proposes `game.app` on macOS (`game.exe` on Windows, `game` on Linux). Choose a
  folder and Save: the status line reports the export.
- [ ] Double-click the exported game in Finder / Explorer: it opens without the editor and plays the sample. Exporting
  again to the same name replaces it.
- [ ] macOS only: copy the `.app` to another folder and open it from there; it still plays.

## Big scenes

Setup: make a big scene with `evolve_editor --gen-scene large big.evolve` (21,200 shapes, see BENCHMARKS.md) and open it.

- [ ] The Objects panel scrolls smoothly through the thousands of rows. Clicking a row selects that object, and the
  collapse arrow, eye and lock buttons work on the row you click.
- [ ] Renaming an object (double-click) works in the middle of the list, and Enter / Esc close the box.
- [ ] Drag a row onto another to reorder; the drop line appears on the right row.
- [ ] `Ctrl+A` selects everything without freezing, and the Scene view stays usable (before 2026-10-24 it took 69 ms a frame with 4,600 objects; it now takes about 9 ms
  at the whole view and 2 ms close up).
- [ ] Select a few hundred shapes with Shift-click or a marquee and move them; it stays responsive.
- [ ] Zoomed in on one shape, panning and zooming feel smooth (the view of the whole scene is the slowest).

## After Phase 1 of the GPU plan (editing panels and change tracking)

Setup: the built-in sample project. Tests with simulated input already cover typing in the name, script and symbol-name
fields, the physics and clip checkboxes and a slider; these lines are the real-window check that nobody has done yet.

- [ ] Select a shape and type a long new name in the Inspector's Name field: no character is lost, the cursor does not jump
  to the start or end while typing, and the name in the Objects panel follows. `Ctrl+Z` brings the old name back in
  one step.
- [ ] Select the ball, click into its script and type in the middle of a line: the text stays where you type it, and the
  error message (if the script breaks) appears and disappears as you type.
- [ ] Tick and untick *Collision body*; with a dynamic body drag the *Bounce*, *Friction* and *Gravity* sliders: the values
  follow the mouse and stay after you let go.
- [ ] Assets panel: select a symbol and type in its Name field; the card's name follows. Camera: change its name and
  width and height.
- [ ] Drag a shape and undo (`Ctrl+Z`): it jumps back in one step. Drag three shapes with Shift-select and undo: one step.
  Rotate and scale with the handles and undo.
- [ ] Press Play (F5) on the sample: the ball rolls and the colors change as before; stop Play, nothing is left changed in
  the scene, and `Ctrl+Z` does not offer a step the game made.
- [ ] Open a big scene (`evolve_editor --gen-scene large big.evolve`): note what still lags (open point P8).

## After Phase 3: gradients, opacity and thin lines on the GPU renderer

Start the editor with *GPU* chosen (status bar). These need a window; the tests compare pictures but cannot judge shimmering.

- [ ] **(plan 3.1)** Make a path or open the map and zoom in and out slowly with the wheel across thin borders (lines thinner than a
      pixel): they should fade and thicken smoothly, with no flicker, no lines that vanish and come back, and no jumps when a
      zoom crosses a power of two. Compare with *CPU* if in doubt.
- [ ] Draw a rectangle (R) in an empty scene, and in the Inspector give its fill a linear gradient and its stroke a radial one
      (or the other way). With *GPU* chosen there is **no notice** at the bottom left of the view, and switching to *CPU* and back
      shows the same picture (edges may differ a little).
- [ ] Give that rectangle an opacity of 50%: still no notice, and the picture still matches *CPU*. Then give its stroke a
      transparent color (alpha below 255): still no notice (since Phase 4 that shape is faded on a layer of its own), and the
      picture still matches *CPU*.
- [ ] Change a gradient's stops or handles in the Inspector: the picture follows on every frame, no flicker.
- [ ] **(fixed 2026-10-16)** Open the map with *GPU*, zoom in and out a lot and pan around: no province loses its color.
- [ ] **(fixed 2026-10-16)** Select many provinces (a marquee over the map, or Ctrl+A): only their outlines are drawn, no long
      straight lines across the view, at any zoom.

## After Phase 4: clip groups, groups with opacity and symbol instances on the GPU renderer

Start the editor with *GPU* chosen. Each of these is also a test that compares pictures; what the tests cannot judge is how it
feels and whether it flickers.

- [ ] Open the built-in sample (*File → New* or restart) with *GPU* chosen. The notice at the bottom left now names only what is
      left: **skinned shapes** (the tentacle). Everything else (gradients, the clip groups, the groups with opacity, the symbol
      instances) is drawn by the GPU renderer: switch *View → Renderer* between *CPU* and *GPU* and the picture looks the same
      (edges may differ a little). Delete the tentacle's skinned shape, or open another scene, and the notice is gone.
- [ ] Draw two overlapping rectangles of different colors, select both and group them (`Ctrl+G`), and give the group an opacity of
      50%: where they overlap you see only the one on top, as one flat shape at 50%, **not** a darker patch where both show
      through. Same picture on *CPU*.
- [ ] Select shapes and make a clip group (the top object is the mask). The content is cut by the mask's outline, smooth at the
      edge. Give the mask no fill, or hide it: it still clips. Move the mask with the Select tool: the clip follows on every frame.
- [ ] Put a symbol instance in a group that has opacity, and give the instance a color override in the Inspector: the picture
      follows, and it is the same on *CPU*.
- [ ] Zoom in and out and pan across all of these: no flicker, and no frame where a faded group is drawn twice or not at all.
- [ ] Open the illustration scene (see BENCHMARKS.md for `--gen-scene illustration`) with *GPU* chosen: no notice, and it looks like
      the *CPU* picture (it is the scene with thousands of transparent strokes, nested faded groups and instances).

## The web canary (plan task 2.12), for the T2 laptop and any browser

One-time setup is the same as for the web player (`rustup target add wasm32-unknown-unknown`,
`cargo install wasm-bindgen-cli --version 0.2.129 --locked`). Then:

```text
powershell -ExecutionPolicy Bypass -File tools/build-web-canary.ps1      (Windows; on other systems follow the steps inside that script)
cd dist\web-canary
python -m http.server 8080
```

Open `http://localhost:8080/index.html?backend=webgl&scene=scene-large.evolve` in the browser, wait a few seconds (it runs three
views of 120 frames and stops), and send me the text under the picture (select it and copy). Then the same with `backend=webgpu`
(only if the browser has WebGPU) and with `scene=scene-illustration.evolve`. Add `&size=1920x1080` for a full-HD canvas. On the
T2 laptop this is task 2.12's last step. The picture on the canvas should look like a map of colored shapes (stress scene) or
thick colored strokes, gradients, faded groups and symbols (illustration scene; since Phase 4 nothing of it is left out). It uses the card
for a few seconds only; close the tab when done.

## The GPU renderer in the editor (Phase 2 of the GPU plan)

*Second try (after the fixes of 2026-10-16). Close the running editor first, or the new build cannot replace it, and build with
`--release`: a debug build is several times slower and was not what the numbers describe.*

Setup: `cargo run -p evolve_editor --release -- --renderer gpu`, or start normally and choose the renderer in the **status
bar** (bottom right, "Renderer: …") or in *View → Renderer*. The status bar also shows the frame times (hover for where
the time went); *View → Show frame times* turns them off. Please write down what it shows while you pan, zoom and drag: it
is the real number, measured around the real work of each frame. "idle" means the editor is drawing nothing, which is right.
The status line says which graphics card was found. Open a big scene (`evolve_editor --gen-scene large big.evolve`, see BENCHMARKS.md) with *File → Open*. The GPU renderer draws fills, strokes, gradients, opacity, clip groups and symbols; views with skinned
shapes (the built-in sample has one, the tentacle) use the CPU renderer and say so in a small notice at the bottom left of
the view. (This text was written for Phase 2, when much more was missing; the notices of the checks below that name gradients,
opacity or symbols no longer appear.) **Do not leave the editor running when you are not using it.**

- [ ] With the map open and the GPU renderer chosen, the whole-map view is smooth (the CPU renderer gave about 14 frames a
  second). Pan with the middle mouse button, zoom with the wheel, rotate with the right button: it follows the mouse.
- [x] The picture looks like the CPU renderer's: switch *View → Renderer* between *CPU* and *GPU* and look at the same
  spot (fills, thin borders, colors). Colors must not look washed out or darker; edges may differ by a little. *(Checked by the owner 2026-10-16: colors, borders and fills clear and right.)*
- [ ] Zoom in far on one border and zoom out again: lines stay smooth, no gaps or flicker while zooming.
- [ ] Select a province and drag it: it moves with the pointer on the GPU renderer too. Undo (`Ctrl+Z`) puts it back.
- [x] Leave the editor alone for a few seconds with the map open: the fans do not spin up and the window stays quiet (it only
  redraws when something changes). *(Owner 2026-10-16: the fans run as normal.)*
- [x] Open the built-in sample (*File → New* or restart) with *GPU* chosen: it shows the notice about what the GPU renderer
  cannot draw yet, and looks exactly as with the CPU renderer. *(Owner 2026-10-16: the notice showed.)*
- [ ] Resize the window and drag the panel borders for a while: no flicker, no black frames, nothing breaks.
- [ ] Put the computer to sleep and wake it (or switch the screen resolution) with *GPU* chosen: the editor does not crash. If the
  graphics device was lost, the status line says so and the views carry on with the CPU renderer.
- [ ] Switch the renderer back and forth several times while the map is open (status bar box or *View → Renderer*): it
  never freezes. With *CPU* chosen the map is slow again (about 2 frames a second at this size): that is the reference
  renderer, and the comparison.
- [ ] Press Play (F5) with the map open: the frame readout stays short, and **dragging with the right mouse button moves the view
  around the map** (while editing the right button rotates it). Say what fps the readout shows while you move.
- [ ] Zoom in far with the wheel (to where one province fills the view) and out again: there is no notice at the bottom left
  of the view (it would mean the CPU renderer took over). One short hitch when the zoom crosses a bucket is known (P12).

### Playing a clip in the editor (plan 6.3)

- [ ] Open the built-in sample, open the Timeline, press the play button: the sun pulses, the stars rock and spin, and the tentacle
  waves, with *CPU* and with *GPU* chosen; both look the same and neither flickers.
- [ ] While the clip plays, drag a star or change its color in the Inspector: the change shows at once and the animation goes on. Stop
  playing: the scene is as you left it (nothing of the animation stays in the objects you did not animate).
- [ ] While it plays, add a keyframe to another object (Timeline) and take it away again: the object follows the new key and then goes
  back to where it was.
- [ ] With the map open (a long scene without animation), add a keyframe to one province and press play: the frame readout stays short
  (it used to jump by more than 10 ms while a clip played).

### Moving groups and instances (plan 6.4)

- [ ] Generate the sprites scene (`evolve_editor --gen-scene sprites sprites.evolve`, then *File → Open*) with *GPU* chosen: thousands
  of small yellow creatures on a green ground. Select a few and drag them with the mouse, then use arrow keys or the Move tool on a
  group of them: they follow at once, the others do not flicker, and nothing is left behind where they were.
- [ ] Drag a creature next to others so that they overlap: the one you move stays in front or behind as it was (the order of drawing
  does not change while you drag).
- [ ] In the built-in sample press Play (F5): the ball rolls, the sun pulses, the tentacle moves; compare with *CPU*. Nothing
  flickers, no object stays where it was.
- [ ] Group some shapes (`Ctrl+G`), then drag the group and change the color of one shape inside it while it is moved: both show at once.

### Skinned shapes on the GPU renderer (plan 6.1)

- [ ] Open the built-in sample (*File → New*, or restart) with *GPU* chosen: the picture looks the same as with *CPU*, **and there is no
  notice at the bottom left any more** (it used to say the GPU renderer cannot draw skinned shapes).
- [ ] The tentacle in the sample: press Play (F5). It follows its target as before, smoothly, with no flicker or holes, and the
  frame readout stays short. Compare with *CPU* if it looks different.
- [ ] Select a bone and rotate it (*Bone* tool) while the tentacle is skinned: the tentacle bends with it at once, on the GPU too.
  Paint weights (*Weights* tool): the shape changes under the brush.
- [ ] A skinned shape *inside* a clip group is drawn on the GPU (clipped by the mask). A clip group whose *mask* is a skinned shape, or
  a skinned shape inside a symbol you made: these two still use the CPU renderer and say so in a notice. Say if anything else gets a
  notice.

### Speed work of Phase 5 (reused frames, layers, clicks, delete and group)

These are changes you can feel; each should look exactly as before, only quicker. Use the real map with the *GPU* renderer, and the
built-in sample or a drawing with groups that have opacity (they need layers) for the layer lines.

- [ ] Pan the whole map with the middle mouse button and the space bar: it follows the pointer smoothly, with no jumps and no holes
  (areas without fill) at the edges of the view while you pan.
- [ ] Zoom with the wheel in small steps, in and out, over the whole map: smooth, and no province changes shape or loses its fill. When
  the zoom crosses a bigger step, one short hitch is known (P12); say if it is longer than before.
- [ ] Zoom in on one province until it fills the view, then pan around it: the curves of the border stay smooth (not made of straight
  pieces), also right after a zoom.
- [ ] In a drawing with groups that have opacity (a faded group of overlapping shapes): pan and zoom, and drag one of the shapes inside
  the group. The faded group looks the same while it moves, its edges at the border of the view are not cut off, and nothing flickers.
- [ ] Click on provinces and on the borders between them: the right object is selected each time, also right after you moved or edited
  another object (undo and redo too). Say if a click ever picks the wrong object or none.
- [ ] Select a few thousand provinces (rubber band over a large area) and press Delete: it is instant (it used to freeze the editor for
  more than a second), and *Undo* brings all of them back. Then select the same number again and press *Group* (`Ctrl+G`): instant
  too, the picture does not change, and *Undo* ungroups them.
- [ ] Edit one province outline with the Node tool (drag a point): only that province changes, at once, nothing else flickers.

### Quality of the GPU renderer (plan 5.8 and 5.9)

Start with `--quality auto` (the default), `high`, `balanced` or `low`, or choose it in *View → Renderer → Quality of the GPU
renderer*. *Automatic* starts from what the card suits (High for a discrete card) and draws smaller pictures while frames are slow.

- [ ] With the map open and *GPU* chosen, switch *Quality* to **Low**: the picture becomes a little softer (it is drawn at three
  quarters of the size and stretched) and nothing else changes: no notice, no flicker, the borders and fills still line up. Switch to
  **High**: it is sharp again at once.
- [ ] With *Quality* **Low** and the map's whole view, pan and zoom: it is no slower than with High. (On this desktop's card it
  should be about the same; the point of Low is a slower card.)
- [ ] Start with `--quality fast` or `--quality`: the editor prints the message and does not start (exit code 2).
- [ ] With *Quality* **Automatic**, move the whole map around for half a minute: the picture stays sharp (frames here are far from
  slow). If it ever turns soft by itself, say when: it means the frame times measured are higher than they should be.
