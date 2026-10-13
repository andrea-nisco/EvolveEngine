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
