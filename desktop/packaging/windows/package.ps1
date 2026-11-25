# Los paquetes de Windows: un instalador (NSIS) y un .zip con el .exe suelto.
#
# NSIS y no un .msi: un .msi (WiX) pide un GUID de actualización que hay que
# custodiar para siempre y trae media herramienta más; NSIS es un programa y un
# script, y es lo que ya instala la app de Tauri. El .zip va siempre, para
# quien no quiera instalar nada: el .exe no necesita nada a su lado (el icono
# va dentro, y la biblioteca de C también — ver `.cargo/config.toml`).
#
# PowerShell y no bash: es lo que hay en cualquier Windows, y a `makensis` le
# llegan las rutas tal cual, sin que nadie se las traduzca por el camino.

$ErrorActionPreference = "Stop"

$packagingDir = Split-Path -Parent $PSScriptRoot
$crateDir = Split-Path -Parent $packagingDir

# La identidad, del mismo fichero que leen el binario y los otros paquetes.
$identity = @{}
foreach ($line in Get-Content -Encoding UTF8 (Join-Path $packagingDir "identity.env")) {
    if ($line -match '^\s*([A-Z_]+)="(.*)"\s*$') { $identity[$Matches[1]] = $Matches[2] }
}

# La versión, de la sección [package] de Cargo.toml y de ningún otro sitio.
$version = $null
$inPackage = $false
foreach ($line in Get-Content -Encoding UTF8 (Join-Path $crateDir "Cargo.toml")) {
    if ($line -match '^\[') { $inPackage = ($line.Trim() -eq "[package]"); continue }
    if ($inPackage -and $line -match '^version\s*=\s*"([^"]+)"') { $version = $Matches[1]; break }
}

$name = $identity["APP_NAME"]
$id = $identity["APP_ID"]
$binary = $identity["APP_BINARY"]
foreach ($pair in @(@("APP_NAME", $name), @("APP_ID", $id), @("APP_BINARY", $binary), @("APP_VERSION", $version))) {
    if ([string]::IsNullOrWhiteSpace($pair[1])) { throw "packaging: falta $($pair[0])" }
}

# Empaquetar no compila: lo que se empaqueta es lo que se acaba de construir.
$exe = Join-Path $crateDir "target\release\$binary.exe"
if (-not (Test-Path $exe)) { throw "packaging: no está $exe; primero ``cargo build --release --locked``" }

$outDir = Join-Path $crateDir "target\packaging\out"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x86_64" }
$base = "$binary-$version-windows-$arch"

# --- .zip -------------------------------------------------------------------
$zip = Join-Path $outDir "$base.zip"
if (Test-Path $zip) { Remove-Item $zip }
# Con la licencia de las tipografías que lleva el .exe (OFL).
$licenses = Join-Path $crateDir "assets\fonts\LICENSE-*.txt"
Compress-Archive -Path $exe, $licenses -DestinationPath $zip

# --- Instalador -------------------------------------------------------------
# `makensis` puede estar instalado sin estar en el PATH: es lo normal.
$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) {
    foreach ($root in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if (-not $root) { continue }
        $candidate = Join-Path $root "NSIS\makensis.exe"
        if (Test-Path $candidate) { $makensis = $candidate; break }
    }
}
if (-not $makensis) { throw "packaging: no encuentro makensis (NSIS 3)" }

$installer = Join-Path $outDir "$base-setup.exe"
$icon = Join-Path $packagingDir "icons\icon.ico"
& $makensis /V2 /INPUTCHARSET UTF8 `
    "/DAPP_NAME=$name" "/DAPP_ID=$id" "/DAPP_BINARY=$binary" "/DAPP_VERSION=$version" `
    "/DSOURCE_EXE=$exe" "/DLICENSES=$licenses" "/DICON_FILE=$icon" "/DOUT_FILE=$installer" `
    (Join-Path $PSScriptRoot "installer.nsi")
if ($LASTEXITCODE -ne 0) { throw "packaging: makensis salió con $LASTEXITCODE" }

Get-ChildItem $outDir
