<#
.SYNOPSIS
  Empaqueta el build Windows de la app como MSIX para Microsoft Store (ADR-0136 · hub#120).

.DESCRIPTION
  Paso POST-BUILD puro: no toca cómo compila Tauri/Cargo. Empaqueta **la app** (este repo,
  ficha única «ERPlora» / `com.erplora.app` — ADR-0160). Requiere un `tauri build` de release.

  1. Stage: exe + Assets/ (iconos que referencia el manifest).
  2. Patch: tokens __MSIX_*__ del Package.appxmanifest con la identidad real
     de Partner Center (Identity Name propio; el Publisher es el de la cuenta).
  3. Pack: `winapp pack` (CLI oficial: winget install microsoft.winappcli).
     SIN -Cert ⇒ MSIX sin firmar, que es lo que exige la submission (la Store firma).

  🪦 Hasta hub#340 el único preset era el del **Bridge standalone** (`-Flavor bridge`,
  `erplora-bridge.exe`), y `tauri-release.yml` llamaba a este script SIN `-Flavor`, así que caía
  en ese preset: buscaba el exe del bridge y escribía `erplora-bridge.msix`, mientras el workflow
  subía `dist-msix/erplora-app.msix`. El paso nunca llegó a correr —está detrás del gate
  `vars.MSIX_IDENTITY_NAME`, que sigue vacío—, pero era un paso roto. Con el bridge retirado ya no
  hay dos sabores: queda **uno**, el de la app, que es lo que el workflow siempre quiso empaquetar.
  ⚠️ Sigue **sin verificar en Windows** (hub#577).

.EXAMPLE
  pwsh scripts/pack-msix.ps1 -Version 1.2.3 `
    -IdentityName "12345Erplora.ERPlora" -Publisher "CN=xxxx-..." `
    -PublisherDisplay "Erplora" -OutDir dist-msix

.NOTES
  Solo Windows (winapp CLI). En CI corre en el leg windows de tauri-release.yml.
  Para probar la INSTALACIÓN local (no la submission) añade -Cert con un devcert:
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
$repoRoot = Split-Path -Parent $PSScriptRoot   # repo root

# --- The installable app. Two axes of uncertainty, so both are probed:
#
#   * Name. `productName` in tauri.conf.json is "ERPlora" (and so is the manifest's
#     `Executable=`), but the cargo bin is `erplora-tauri`: Tauri renames the binary after
#     building. That rename has never been observed on a Windows runner here (hub#577 §1), so
#     both names are candidates. Either way the file is staged as $p.ExeName below, so the
#     package is correct without having to know which one `tauri build` leaves behind.
#   * Target dir. `tauri build` may run from the workspace root or from src-tauri,
#     and those do not share a target/ directory.
#
# Preference order: the renamed exe first — if both exist, that is the one the bundle ships. ---
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

# --- Versión: la Store exige 4 partes con revisión 0 (v1.2.3 → 1.2.3.0). ---
$v = $Version.TrimStart("v")
if ($v -notmatch '^\d+\.\d+\.\d+(\.\d+)?$') { throw "Versión inválida: $Version" }
if ($v -match '^\d+\.\d+\.\d+$') { $v = "$v.0" }

# --- Localizar el exe de release. ---
$exe = $p.ExeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exe) {
  throw ("No release binary found — run 'tauri build --release' first. Looked for:`n  " +
    ($p.ExeCandidates -join "`n  "))
}

# --- Stage: mismo layout que instala NSIS ($INSTDIR): exe + Assets/. ---
$stage = "$repoRoot/target/msix-stage-app"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path "$stage/Assets" | Out-Null
Copy-Item $exe "$stage/$($p.ExeName)"

foreach ($i in "Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png") {
  Copy-Item "$($p.AssetsDir)/$i" "$stage/Assets/$i"
}

# --- Manifest: patch de tokens → stage. ---
$manifest = Get-Content $p.Manifest -Raw
$manifest = $manifest.
  Replace("__MSIX_IDENTITY_NAME__", $IdentityName).
  Replace("__MSIX_PUBLISHER__", $Publisher).
  Replace("__MSIX_PUBLISHER_DISPLAY__", $PublisherDisplay).
  Replace("__MSIX_VERSION__", $v)
Set-Content "$stage/Package.appxmanifest" $manifest -Encoding utf8

# --- Pack: winapp lee el Package.appxmanifest del cwd (lo copia al target), así que corremos
# desde el propio stage. Sin --cert = sin firmar (submission a la Store). ---
$out = Join-Path $repoRoot $OutDir
New-Item -ItemType Directory -Path $out -Force | Out-Null
Push-Location $stage
try {
  $packArgs = @("pack", $stage)
  if ($Cert) { $packArgs += @("--cert", $Cert) }
  winapp @packArgs
  if ($LASTEXITCODE -ne 0) { throw "winapp pack falló ($LASTEXITCODE)" }
  $msix = Get-ChildItem -Path $stage -Filter "*.msix" | Select-Object -First 1
  if (-not $msix) { $msix = Get-ChildItem -Path . -Filter "*.msix" | Select-Object -First 1 }
  if (-not $msix) { throw "winapp pack no produjo ningún .msix" }
  Copy-Item $msix.FullName "$out/$($p.OutName)" -Force
}
finally { Pop-Location }

Write-Host "MSIX listo: $out/$($p.OutName) (versión $v, sin firmar — la Store firma)"
