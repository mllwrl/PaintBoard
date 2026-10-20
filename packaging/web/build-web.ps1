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
    # Function names only help reading stack traces; without them the file is ~10% smaller.
    wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section `
        --out-dir $out --out-name paintboard 'target\wasm32-unknown-unknown\release\paintboard.wasm'
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed' }
    Copy-Item -Force 'packaging\web\index.html' $out
    Copy-Item -Force 'crates\pb-app\assets\icon.png' $out
    # The service worker keeps this build: its version is the build's own, so browsers
    # notice a new one.
    $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $out 'paintboard_bg.wasm')).Hash.Substring(0, 16)
    $worker = [System.IO.File]::ReadAllText((Join-Path $root 'packaging\web\sw.js'))
    [System.IO.File]::WriteAllText((Join-Path $out 'sw.js'), $worker.Replace('__VERSION__', $hash.ToLower()))
    # Compressed copies for servers that send them as they are (paintboard-server does):
    # the .wasm shrinks to about a third.
    foreach ($name in 'paintboard_bg.wasm', 'paintboard.js', 'index.html') {
        $source = Join-Path $out $name
        $bytes = [System.IO.File]::ReadAllBytes($source)
        $target = [System.IO.File]::Create("$source.gz")
        $gzip = New-Object System.IO.Compression.GZipStream($target, [System.IO.Compression.CompressionLevel]::Optimal)
        $gzip.Write($bytes, 0, $bytes.Length)
        $gzip.Dispose()
        $target.Dispose()
    }
    Get-ChildItem $out | Select-Object Name, Length
}
finally {
    Pop-Location
}
