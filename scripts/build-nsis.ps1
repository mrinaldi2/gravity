# Builds the Windows installer (NSIS) from the checkout this script is in.
# Build only: installs nothing. Run through `hermesd release build-installer`,
# which first checks this file is exactly as committed (H-117 X3, H-104).
$ErrorActionPreference = 'Stop'
# A clean build environment: no home override, no Team ID; a dev build until
# Windows releases are signed.
Remove-Item Env:THEHERMES_HOME,Env:GRAVITY_HOME,Env:HERMES_TEAM_ID -ErrorAction SilentlyContinue
$env:HERMES_DEV_BUILD = "1"
# A release: the daemon and the app build into this checkout's own target,
# never a long-lived one another bot could have written (H-029, CE-013 G2).
$env:HERMES_RELEASE_BUILD = "1"
$env:CARGO_TARGET_DIR = Join-Path (Join-Path $PSScriptRoot '..') 'target'
Write-Output "HERMES_DEV_BUILD=[$env:HERMES_DEV_BUILD] HERMES_TEAM_ID=[$env:HERMES_TEAM_ID]"
# cargo from scoop's rustup, when that's where it lives and not on PATH.
$scoopCargo = Join-Path $env:USERPROFILE 'scoop\persist\rustup\.cargo\bin'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path $scoopCargo)) {
    $env:Path = "$scoopCargo;$env:Path"
}
Set-Location (Join-Path $PSScriptRoot '..')
Write-Output "HEAD $(git rev-parse HEAD)"
./scripts/prepare-sidecar.ps1
Set-Location apps/desktop
npx -y pnpm@11 install --frozen-lockfile
npx -y pnpm@11 tauri build --bundles nsis
