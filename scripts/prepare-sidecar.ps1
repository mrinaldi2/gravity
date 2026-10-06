# Build and stage the native Windows daemon for Tauri.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    $hostLine = rustc -vV | Where-Object { $_.StartsWith('host: ') }
    if ($LASTEXITCODE -ne 0 -or !$hostLine) { throw 'Cannot determine Rust host triple' }
    $triple = $hostLine.Substring(6)
    if (!$triple.EndsWith('windows-msvc')) { throw 'Use a native Windows MSVC Rust toolchain' }
    cargo build --release --locked -p hermesd
    if ($LASTEXITCODE -ne 0) { throw 'Daemon build failed' }
    $destination = Join-Path $repoRoot 'apps/desktop/src-tauri/binaries'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    # Cargo builds into $env:CARGO_TARGET_DIR when set: a bot's own target or
    # the workers' shared one (H-029, H-109).
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot 'target' }
    Copy-Item -LiteralPath (Join-Path $target 'release/hermesd.exe') -Destination (Join-Path $destination "hermesd-$triple.exe")
    Write-Output "Staged hermesd-$triple.exe"
} finally {
    Pop-Location
}
