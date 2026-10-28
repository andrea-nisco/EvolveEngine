# Contributing to EvolveEngine

Thank you for wanting to help. EvolveEngine is a free, open-source (MIT) vector-first 2D game editor written in Rust,
and the goal is a base that people can change to fit their own games. Contributions of every size are welcome: bug
reports, testing on your platform, documentation, examples and code.

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Bug reports and feature
requests have forms when you open an issue; pull requests have a short checklist.

## Getting started

```
git clone <the repository>
cd EvolveEngine
cargo run -p evolve_editor --release     # the editor
cargo test --workspace                   # the tests
cargo clippy --workspace --all-targets   # lints; the project is kept free of warnings
```

Platform requirements are in the [README](README.md#run). Windows, macOS and Linux are all supported targets; the
code should stay portable (no hard-coded path separators or `.exe` names, use `cfg!` where the platforms differ, and
use `Modifiers::COMMAND`, not `CTRL`, for shortcuts).

## Where things are

- `crates/evolve_engine`: scene graph, geometry, rendering, animation, physics, scripting. No UI, no dependency on the
  editor, so a runtime can reuse it.
- `crates/evolve_editor`: the egui application (panels, tools, dialogs).
- `crates/evolve_player`: the standalone player (desktop and WebAssembly) and the export code.

The README has a file-by-file map. [TODO.md](TODO.md) is the roadmap: it lists what is planned, in priority order,
and the known limits of what exists. Pick something from it, or open an issue first if you want to discuss an idea.

## Changing the scene (change tracking)

The scene stamps every change so that the undo history, and later the GPU renderer, know what changed without
comparing everything ([crates/evolve_engine/src/revision.rs](crates/evolve_engine/src/revision.rs) explains it). Three
rules keep that true:

1. **Change a node through `Scene::find_mut`.** Writing a field directly (`node.name = ...`) is tracked, and assumes
   the worst. Code that runs every frame on many nodes (physics, scripts, animation, dragging) must use the precise
   methods instead (`set_transform`, `set_opacity`, `set_solid_fill_color`, ...), which stamp only what they change.
   Add a precise method when you need one. Model code that holds a node some other way (a node taken out of the tree,
   a copy) must bump its stamps by hand, and a scene-level change (the list of symbols, say) calls `Scene::touch`.
2. **In a panel, never hand `&mut node.field` to a widget.** The panel runs every frame, so that would count as a
   change every frame. Edit a copy, and write it back only when it differs (see `physics_section` in `game.rs`).
   A test (`idle_tests.rs`) runs the real panels on every node of the sample project and fails if the scene changes
   on a frame where nobody did anything.
3. **New way of changing a scene, new line in `revision_tests.rs`.** Its audit fingerprints every node before and
   after and fails if content changed without its stamp. Development builds also double-check the undo history
   against a full comparison and say so on the console when a change skipped the stamps.

Panels that edit a copy and write it back are tested with simulated input in `edit_tests.rs` (egui's own test harness
`egui_kittest`: click a field, type, look at the scene). Add a test there when you add an editing field.

## Code that runs on every frame must be linear in the scene

The editor runs its panels, overlays and the game on every frame, and the map has 21,000 shapes. A loop that calls
`scene.find(id)` for every id (or `world_transform(id)`, `is_bone(id)` and the like, which search the tree) is quadratic and cost
400 ms a frame once. To look at every node use `Scene::for_each_node`, which walks the tree once; use `find` for the few nodes
you already have an id for. Measure with the profile of the real map: `PROFILE_GPU=1 cargo test --release -p evolve_editor
profile_tests -- --ignored --nocapture` (see `profile_tests.rs`), and look at the frame times in the editor's status bar.

## GPU code: read GPU_SAFETY.md first

Anything that touches the graphics card (the `evolve_gpu` crate, the editor and player views, tests, benchmarks) follows
[GPU_SAFETY.md](GPU_SAFETY.md): frame pacing, clamped inputs, a tracked memory budget, survivable errors, and a short
list of things the code never does. A change that breaks one of its rules is not merged. Agents: do not start GPU stress
tests or long benchmark runs on the owner's machine without asking, and end every GPU process you start before you
report (rules E3 and F3).

## GPU tests

When a picture comparison fails, set `EVOLVE_DUMP_PICTURES` to a folder (`EVOLVE_DUMP_PICTURES=local/dump cargo test ...`) and both
pictures are saved there as PNG files, the CPU renderer's and the GPU renderer's: most failures are easy to see.

GPU tests open a device without a window and print "skipped" and pass when the machine has no graphics adapter, so that
CI on machines without one stays green. A place that has promised a GPU sets `EVOLVE_REQUIRE_GPU=native` (or `all`, to
require the WebGL2-limits device too), which turns "skipped" into a failure; the `gpu-required` CI job does this on Linux
with Mesa's software Vulkan driver. Write GPU tests with `Gpu::for_each_test_mode`, which runs them one at a time on a shared device of every kind the
machine has, and checks that the test returned all the graphics memory it took and that the card reported no error. The
tests of the egui glue need the cargo feature `egui` (the editor turns it on, so `cargo test --workspace` has them; use
`cargo test -p evolve_gpu --features egui` to run the GPU crate alone).

Where the pictures are compared with the CPU renderer's: `picture_tests.rs` (shapes, strokes, gradients and the helpers the other
files use), `clip_tests.rs` (clip groups), `layer_tests.rs` (groups with opacity, layers, their limits) and `instance_tests.rs`
(symbols). `renderer_tests.rs` is about what the renderer does between frames (what it uploads, rebuilds, refuses and gives back).
A feature that changes how the picture is made gets a test in both device modes **and a mutation check**: break the rule on
purpose (the scripts in the session notes of the plan's progress log show how) and see that a test fails, because a test that
cannot fail proves nothing. Run the whole workspace in a **debug** build too (`cargo test --workspace`): some tests check
`debug_assert!`s that a release build leaves out.

## Before you open a pull request

1. `cargo test --workspace` passes and `cargo clippy --workspace --all-targets` shows no warnings. CI runs both on
   Windows, macOS and Linux.
2. New behaviour has a unit test where that is practical. Things that need a person at the keyboard go into
   [MANUAL_TESTS.md](MANUAL_TESTS.md) as a checklist line.
3. Match the surrounding code: its naming, its comment density and its idiom. Comments explain *why*, not what.
4. Update [README.md](README.md) if you change what users see, and tick off or add lines in [TODO.md](TODO.md).
5. Keep a pull request to one topic. Smaller ones are reviewed faster.

## Changing the project file format

People's projects are the most valuable thing the editor touches, so an old `.evolve` file must always open.
`crates/evolve_engine/tests/format_compat.rs` loads one frozen file per format version
(`crates/evolve_engine/tests/fixtures`, see its README) and fails if one stops opening. The rules:

1. **Never edit a fixture to make a test pass.** Fix the loader instead (`#[serde(default)]`, `#[serde(alias = ...)]`,
   or an upgrade step in `io.rs`).
2. **Any change to what the editor writes is a new format version**, even adding a field: bump `FORMAT_VERSION` in
   `io.rs`. This is also what keeps an older editor from silently dropping data it does not know.
3. **Before you change the format,** save a genuine file with the current editor and add it as the fixture of the
   current version if it is not there yet. After the bump, add a fixture for the new version too (the test
   `there_is_a_fixture_for_every_format_version` reminds you).
4. Update the version list in the README and add a line to the CHANGELOG.

## Reporting bugs

Please include your operating system, what you did, what you expected and what happened. If a project file triggers
it, attach the `.evolve` file (it is plain JSON) or the smallest one that still shows the problem.

## License

By contributing you agree that your work is released under the [MIT license](LICENSE) of this project.
