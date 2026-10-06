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
    # Where to build, and then copy from, in this one step (H-029, CE-013
    # G2). A release (HERMES_RELEASE_BUILD=1, as build-nsis.ps1 sets) never
    # trusts $env:CARGO_TARGET_DIR: it builds into this checkout's own target,
    # which no other bot may write. A dev build uses the session's target.
    $target = if ($env:HERMES_RELEASE_BUILD -ne '1' -and $env:CARGO_TARGET_DIR) {
        $env:CARGO_TARGET_DIR
    } else {
        Join-Path $repoRoot 'target'
    }
    cargo build --release --locked -p hermesd --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw 'Daemon build failed' }
    $destination = Join-Path $repoRoot 'apps/desktop/src-tauri/binaries'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item -LiteralPath (Join-Path $target 'release/hermesd.exe') -Destination (Join-Path $destination "hermesd-$triple.exe")
    Write-Output "Staged hermesd-$triple.exe"
} finally {
    Pop-Location
}
