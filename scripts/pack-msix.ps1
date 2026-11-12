<#
.SYNOPSIS
  Packs the app's Windows release build as an MSIX for the Microsoft Store.

.DESCRIPTION
  A post-build step only: it does not change how Tauri or Cargo build. Needs a release
  `tauri build` first.

  1. Stage: the exe plus Assets/ (the icons the manifest references).
  2. Patch: the __MSIX_*__ tokens of Package.appxmanifest get the real Partner Center identity
     (the listing's own Identity Name; the Publisher is the account's).
  3. Pack: `winapp pack` (official CLI: winget install microsoft.winappcli).
     WITHOUT -Cert the MSIX is unsigned, which is what a Store submission requires (the Store
     signs it).

  ⚠️ Not yet verified on a Windows runner.

.EXAMPLE
  pwsh scripts/pack-msix.ps1 -Version 1.2.3 `
    -IdentityName "12345Erplora.ERPlora" -Publisher "CN=xxxx-..." `
    -PublisherDisplay "Erplora" -OutDir dist-msix

.NOTES
  Windows only (winapp CLI). In CI it runs on the Windows leg of tauri-release.yml.
  To test a local INSTALL (not a submission) add -Cert with a dev certificate:
  `winapp cert generate` + `winapp cert install`.
#>
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$IdentityName,
  [Parameter(Mandatory = $true)][string]$Publisher,
  [Parameter(Mandatory = $true)][string]$PublisherDisplay,
  [string]$OutDir = "dist-msix",
  [string]$Cert = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

# Two unknowns, so both are probed:
#
#   * Name. `productName` in tauri.conf.json is "ERPlora" (and so is the manifest's
#     `Executable=`), but the cargo bin is `erplora-tauri`: Tauri renames the binary after
#     building. That rename has never been observed on a Windows runner, so both names are
#     candidates. Either way the file is staged as $p.ExeName below, so the package is correct
#     without knowing which one `tauri build` leaves behind.
#   * Target dir. `tauri build` may run from the workspace root or from src-tauri,
#     and those do not share a target/ directory.
#
# The renamed exe comes first: if both exist, that is the one the bundle ships.
$p = @{
  ExeCandidates = @(
    "$repoRoot/target/release/ERPlora.exe",
    "$repoRoot/src-tauri/target/release/ERPlora.exe",
    "$repoRoot/target/release/erplora-tauri.exe",
    "$repoRoot/src-tauri/target/release/erplora-tauri.exe"
  )
  ExeName       = "ERPlora.exe"
  Manifest      = "$repoRoot/src-tauri/msix/Package.appxmanifest"
  AssetsDir     = "$repoRoot/src-tauri/icons"
  OutName       = "erplora-app.msix"
}

# The Store requires four parts with revision 0 (v1.2.3 → 1.2.3.0).
$v = $Version.TrimStart("v")
if ($v -notmatch '^\d+\.\d+\.\d+(\.\d+)?$') { throw "Invalid version: $Version" }
if ($v -match '^\d+\.\d+\.\d+$') { $v = "$v.0" }

$exe = $p.ExeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exe) {
  throw ("No release binary found — run 'tauri build --release' first. Looked for:`n  " +
    ($p.ExeCandidates -join "`n  "))
}

# The same layout NSIS installs into $INSTDIR: the exe plus Assets/.
$stage = "$repoRoot/target/msix-stage-app"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path "$stage/Assets" | Out-Null
Copy-Item $exe "$stage/$($p.ExeName)"

foreach ($i in "Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png") {
  Copy-Item "$($p.AssetsDir)/$i" "$stage/Assets/$i"
}

$manifest = Get-Content $p.Manifest -Raw
$manifest = $manifest.
  Replace("__MSIX_IDENTITY_NAME__", $IdentityName).
  Replace("__MSIX_PUBLISHER__", $Publisher).
  Replace("__MSIX_PUBLISHER_DISPLAY__", $PublisherDisplay).
  Replace("__MSIX_VERSION__", $v)
Set-Content "$stage/Package.appxmanifest" $manifest -Encoding utf8

# winapp reads Package.appxmanifest from the working directory, so it runs from the stage.
# Without --cert the package is unsigned, as a Store submission needs.
$out = Join-Path $repoRoot $OutDir
New-Item -ItemType Directory -Path $out -Force | Out-Null
Push-Location $stage
try {
  $packArgs = @("pack", $stage)
  if ($Cert) { $packArgs += @("--cert", $Cert) }
  winapp @packArgs
  if ($LASTEXITCODE -ne 0) { throw "winapp pack failed ($LASTEXITCODE)" }
  $msix = Get-ChildItem -Path $stage -Filter "*.msix" | Select-Object -First 1
  if (-not $msix) { $msix = Get-ChildItem -Path . -Filter "*.msix" | Select-Object -First 1 }
  if (-not $msix) { throw "winapp pack produced no .msix" }
  Copy-Item $msix.FullName "$out/$($p.OutName)" -Force
}
finally { Pop-Location }

Write-Host "MSIX ready: $out/$($p.OutName) (version $v, unsigned: the Store signs it)"
