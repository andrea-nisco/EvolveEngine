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

- [x] **Duplicate**: right-click B, *Duplicate*. A copy opens, with the same contents and a different name;
      editing the copy leaves B unchanged.
- [x] **Set as start scene**: right-click C, *Set as start scene*. The "start" tag moves to C and the project
      counts as changed. *Play from start* begins in C.
- [x] **Play from another scene**: open B, press Play (`F5`). The game starts in B (not in the start
      scene); `next_scene()` from a script goes to the scene below B. Stop, and the editor is back on B.
- [x] **Delete a scene that is not open**: right-click C while B is open, *Delete*. A confirmation asks first;
      **Cancel** keeps it, **Delete** removes it and B stays open. Start scene moves to a valid scene.
- [x] **Delete the open scene**: confirm. The editor continues in a neighbouring scene with an empty undo history.
- [x] **Last scene**: with one scene left, *Delete* is disabled.
- [x] **Esc key** on the delete confirmation: with the "Delete scene" box open, press the `Esc` key on the
      keyboard. The box closes and the scene is still there. (Clicking outside the box does nothing: it stays
      until you choose Delete or Cancel.)
- [x] **Unsaved-changes prompt**, repeat these steps once for each scene-list change (add a scene, rename one,
      move one up or down, delete one, set another as start):
      1. *File → Save* so the title bar has no `*` after the file name.
      2. Make the one change. A `*` appears in the title bar.
      3. *File → New* (or *Open*, or close the window). A box "Unsaved changes" must appear with
         Save / Don't save / Cancel. Press Cancel.
      4. If no box appears for a change, that change is not marking the project as modified: note which one.
- [x] **Save and reopen with the dialogs**: *File → Save As*, close the editor, reopen with *File → Open*. All
      scenes, their order, names and the start scene are back, and the open scene is the start scene.
- [x] **Game view with the Scene tab closed**: close the Scene tab, leave the Game view open. It still draws,
      Play works and the mouse works through the camera.
- [x] **Camera protection**: select the camera and press Delete, use *Edit → Delete*, the row menu *Delete* in
      the Objects panel, and *Cut*. Each shows a message "A scene needs a camera, so it was not deleted" in a
      box near the bottom of the window for a few seconds, and the camera stays.
- [x] Group the camera with a shape and delete the group: refused too. Selecting the shape and the camera
      together deletes only the shape.

## Renaming and duplicating

- [x] Double-click a scene in the Scenes panel, type a name, press Enter: the name is saved and the box closes.
- [x] Same, but click somewhere else instead of Enter: the name is saved.
- [x] Same, but press `Esc`: the old name stays.
- [x] Rename an object (including the camera) in the Objects panel: Enter closes the box and the name stays.
- [x] Duplicate an object (`Ctrl+D` or the row menu): the copy is called "<name> copy". Duplicating a scene
      does the same.

## PNG sequence export

Setup: the built-in sample is enough. It opens on the "Sunset" scene whose "Idle" clip is 3 s at 30 fps
(90 frames). In the export box set the scale to 3 or 4 so each frame takes longer and there is time to press
Cancel.

- [x] *File → Export Picture → PNG Sequence (current clip)...*, then choose a folder. A progress window appears, the editor stays
      responsive (move the pointer, switch tabs) and the count goes up.
- [x] **Cancel** mid-way: the window closes, the status line says how many frames were written, and the
      files that were written are on disk.
- [x] A finished export reports "Exported N frames" and the folder holds N numbered PNGs.
- [x] Starting a second sequence while one runs is refused with a status message.

## Navigating the Scene view (trackpad and mouse)

Setup: the Scene view open on any scene. On a Mac with a trackpad (or a Windows precision touchpad, where only the
pinch and `Space`+drag lines apply, because Windows reports touchpad scrolling like a mouse wheel).

- [x] **Two-finger scroll** over the Scene view moves the view in any direction, following the fingers; letting go
      glides to a stop and does not zoom.
- [x] **Pinch** zooms at the cursor.
- [x] **`Cmd` + two-finger scroll** (up / down) zooms at the cursor.
- [x] A **mouse wheel** (a real mouse, not the trackpad) still zooms at the cursor. Using the wheel right after the
      trackpad, and the other way round, switches behaviour without sticking.
- [x] `Space` + drag with one finger (press-and-hold the trackpad button, or tap-and-drag) pans too.
- [x] Right-drag rotates the view; on a Mac that is `Ctrl` + click-drag, or a two-finger click if enabled in System Settings.

## Exporting a game

Setup: the built-in sample project, and a release build next to its player (`cargo build --release --workspace`, then
`cargo run -p evolve_editor --release`).

- [ ] *File → Export Game (desktop)...* proposes `game.app` on macOS (`game.exe` on Windows, `game` on Linux). Choose a
      folder and Save: the status line reports the export.
- [ ] Double-click the exported game in Finder / Explorer: it opens without the editor and plays the sample. Exporting
      again to the same name replaces it.
- [ ] macOS only: copy the `.app` to another folder and open it from there; it still plays.
