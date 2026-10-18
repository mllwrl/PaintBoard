# Builds the Windows installer: target\installer\PaintBoard-<version>-setup.exe.
# Needs Inno Setup 6 (winget install JRSoftware.InnoSetup) and cargo-about
# (cargo install cargo-about --locked --features cli).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Push-Location $root
try {
    cargo build --release -p pb-app
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    # "path+file:///.../crates/pb-app#0.1.0" or "...#pb-app@0.1.0"
    $version = ((cargo pkgid -p pb-app) -split '[#@]')[-1]
    $out = Join-Path $root 'target\installer'
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    # Licenses of everything built into the app, then the font's.
    $notices = Join-Path $out 'THIRD-PARTY.txt'
    cargo about generate -c packaging\windows\about.toml -o $notices packaging\windows\about.hbs
    if ($LASTEXITCODE -ne 0) { throw 'cargo about failed' }
    $font = Get-Content -Raw -Encoding utf8 -Path 'crates\pb-text\assets\fonts\Inter-OFL.txt'
    Add-Content -Encoding utf8 -Path $notices -Value "`r`nInter typeface (the app's font)`r`n--------------------------------------------------------------------------------`r`n`r`n$font"

    $iscc = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw 'Inno Setup 6 not found: winget install JRSoftware.InnoSetup' }
    & $iscc "/DAppVersion=$version" 'packaging\windows\PaintBoard.iss'
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }
    Get-ChildItem -Path $out -Filter '*-setup.exe' | Select-Object Name, Length
}
finally {
    Pop-Location
}
