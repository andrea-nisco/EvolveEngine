# Builds the browser version of the player into dist/web-player.
# The editor's "Export for Web" copies these files next to the exported index.html.
#
# One-time setup:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked   # must match the wasm-bindgen crate in Cargo.lock

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# The "web" profile (see Cargo.toml) optimizes for size.
cargo build -p evolve_player --lib --target wasm32-unknown-unknown --profile web
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$out = Join-Path $root "dist/web-player"
New-Item -ItemType Directory -Force $out | Out-Null
wasm-bindgen --target web --no-typescript --out-name evolve_player --out-dir $out target/wasm32-unknown-unknown/web/evolve_player_core.wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

# Optional extra shrinking when binaryen's wasm-opt is installed (https://github.com/WebAssembly/binaryen/releases).
# It saves about 9% of the raw file but makes no difference after gzip, so it is not required.
$wasm = Join-Path $out "evolve_player_bg.wasm"
if (Get-Command wasm-opt -ErrorAction SilentlyContinue) {
    wasm-opt -Os --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals -o $wasm $wasm
    if ($LASTEXITCODE -ne 0) { throw "wasm-opt failed" }
} else {
    Write-Host "wasm-opt not found: skipping the extra size optimization"
}

Get-ChildItem $out | ForEach-Object { "{0,-28} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB) }
