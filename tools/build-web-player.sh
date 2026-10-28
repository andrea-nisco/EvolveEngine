#!/usr/bin/env bash
# Builds the browser version of the player into dist/web-player (macOS and Linux twin of build-web-player.ps1).
# The editor's "Export for Web" copies these files next to the exported index.html.
#
# One-time setup:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked   # must match the wasm-bindgen crate in Cargo.lock

set -euo pipefail
cd "$(dirname "$0")/.."

# The "web" profile (see Cargo.toml) optimizes for size.
cargo build -p evolve_player --lib --target wasm32-unknown-unknown --profile web

out="dist/web-player"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-name evolve_player --out-dir "$out" \
    target/wasm32-unknown-unknown/web/evolve_player_core.wasm

# Optional extra shrinking when binaryen's wasm-opt is installed (https://github.com/WebAssembly/binaryen/releases).
# It saves about 9% of the raw file but makes no difference after gzip, so it is not required.
wasm="$out/evolve_player_bg.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
    wasm-opt -Os --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals -o "$wasm" "$wasm"
else
    echo "wasm-opt not found: skipping the extra size optimization"
fi

ls -l "$out" | awk 'NR > 1 { printf "%-28s %8.0f KB\n", $NF, $5 / 1024 }'
