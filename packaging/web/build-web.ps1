# Builds the web version into target\web: open it through any static web server, e.g.
#   python -m http.server 8000 --directory target\web
# Needs the wasm32 target (rustup target add wasm32-unknown-unknown) and wasm-bindgen-cli
# of the same version as the wasm-bindgen crate (cargo install wasm-bindgen-cli --version
# <version from Cargo.lock> --locked).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Push-Location $root
try {
    # psm (under loro) compiles C for wasm32 and needs clang: LLVM's, if it is not on PATH.
    $llvm = Join-Path $env:ProgramFiles 'LLVM\bin'
    if (-not (Get-Command clang -ErrorAction SilentlyContinue) -and (Test-Path $llvm)) {
        $env:PATH = "$llvm;$env:PATH"
    }
    cargo build --release --target wasm32-unknown-unknown -p pb-app
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $out = Join-Path $root 'target\web'
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    wasm-bindgen --target web --no-typescript --out-dir $out --out-name paintboard `
        'target\wasm32-unknown-unknown\release\paintboard.wasm'
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed' }
    Copy-Item -Force 'packaging\web\index.html' $out
    Copy-Item -Force 'crates\pb-app\assets\icon.png' $out
    Get-ChildItem $out | Select-Object Name, Length
}
finally {
    Pop-Location
}
