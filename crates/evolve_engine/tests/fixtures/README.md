# Project file fixtures

One frozen `.evolve` file per project format version. [format_compat.rs](../format_compat.rs) loads every one of
them, so a change that stops an old file from opening makes `cargo test` fail.

**Never edit these files to make a test pass.** If an old file stops loading, fix the loader (a `#[serde(default)]`,
a rename alias, an upgrade step in `io.rs`) so that the old file still opens. See "Changing the project file format" in
[CONTRIBUTING.md](../../../../CONTRIBUTING.md).

| File | What it is |
|---|---|
| `v4.evolve` | The output of the version 4 writer, byte for byte: the built-in sample project (`evolve_editor --sample`). Its physics settings are the defaults, so the `physics` block that version 4 added is left out. |
| `v3.evolve` | The output of the version 3 writer, byte for byte: the built-in sample project (two scenes). |
| `v2.evolve` | Reconstructed from `v3.evolve`: its "Sunset" scene alone, stored under `scene` as version 2 did. |
| `v1.evolve` | Reconstructed from the same scene: no camera object, the screen size and color stored on the scene instead. |

Versions 1 and 2 were reconstructed from the documented differences between versions, because no genuine files of
those versions were kept. From version 4 on, add the genuine output of the writer *before* changing the format.
