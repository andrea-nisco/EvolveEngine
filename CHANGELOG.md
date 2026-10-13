# Changelog

All notable changes to EvolveEngine. The format follows [Keep a Changelog](https://keepachangelog.com);
the project is not released yet, so everything is under "Unreleased".

## Unreleased

### Added
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
