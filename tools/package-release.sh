#!/usr/bin/env bash
# Packs the release build of the editor and the player into dist/evolve-engine-<label>.(zip|tar.gz).
# Used by .github/workflows/release.yml; also handy to try locally:
#   cargo build --release -p evolve_editor -p evolve_player
#   tools/build-web-player.sh                      # optional: adds "Export for Web"
#   tools/package-release.sh v0.1.0-macos-arm64
#
# The editor looks for the player next to its own program and for web-player/ in its folder,
# so everything goes into one folder.

set -euo pipefail
cd "$(dirname "$0")/.."

label="${1:?usage: tools/package-release.sh <label> [web-player-folder]}"
web="${2:-dist/web-player}"
name="evolve-engine-$label"

exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=".exe" ;; esac

rm -rf "dist/$name"
mkdir -p "dist/$name"
cp "target/release/evolve_editor$exe" "target/release/evolve_player$exe" "dist/$name/"
if [ -d "$web" ]; then
    cp -R "$web" "dist/$name/web-player"
else
    echo "warning: $web not found, the package will not offer Export for Web" >&2
fi
cp LICENSE README.md CHANGELOG.md "dist/$name/"

cd dist
rm -f "$name.zip" "$name.tar.gz"
if [ -n "$exe" ]; then
    7z a -tzip "$name.zip" "$name" > /dev/null
    echo "dist/$name.zip"
else
    tar -czf "$name.tar.gz" "$name" # tar keeps the executable bits; zip would lose them on some tools
    echo "dist/$name.tar.gz"
fi
