# Builds the web canary (plan task 2.12) into dist/web-canary: a page that runs the GPU renderer in a browser.
#
# One-time setup (the same as for the web player):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked   # must match the wasm-bindgen crate in Cargo.lock
#
# Then serve the folder with any static web server and open the page, for example:
#   cd dist/web-canary; python -m http.server 8080       and open  http://localhost:8080/index.html?backend=webgl
# The page needs a scene: this script makes two with the editor's generator (scene-large.evolve, scene-illustration.evolve);
# use ?scene=scene-large.evolve. Any other .evolve file copied into the folder works too.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build -p web_canary --lib --target wasm32-unknown-unknown --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$out = Join-Path $root "dist/web-canary"
New-Item -ItemType Directory -Force $out | Out-Null
wasm-bindgen --target web --no-typescript --out-name web_canary --out-dir $out target/wasm32-unknown-unknown/release/web_canary.wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }
Copy-Item (Join-Path $root "tools/web-canary/index.html") $out -Force

cargo build --release -p evolve_editor
if ($LASTEXITCODE -ne 0) { throw "cargo build of the editor failed" }
foreach ($kind in "large", "illustration") {
    & (Join-Path $root "target/release/evolve_editor.exe") --gen-scene $kind (Join-Path $out "scene-$kind.evolve")
    if ($LASTEXITCODE -ne 0) { throw "could not generate the $kind scene" }
}
Get-ChildItem $out | Format-Table Name, @{ Label = "KB"; Expression = { [math]::Round($_.Length / 1024) } } -AutoSize
