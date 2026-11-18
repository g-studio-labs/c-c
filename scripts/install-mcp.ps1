# Instala cac-mcp, el servidor MCP de cac, en Windows (PowerShell 5.1 o posterior).
#
#   irm https://raw.githubusercontent.com/g-studio-labs/c-c/main/scripts/install-mcp.ps1 | iex
#
# El espejo de scripts/install-mcp.sh, paso por paso: baja el binario de su
# release (etiqueta mcp-vX.Y.Z), comprueba su SHA-256 contra el SHA256SUMS
# publicado y lo deja en %LOCALAPPDATA%\Programs\cac-mcp\cac-mcp.exe. Si la
# suma no coincide no instala nada. No pide administrador, y no pide, lee ni
# imprime ningún token.
#
# Por el entorno: CAC_MCP_VERSION, CAC_MCP_INSTALL_DIR, CAC_MCP_VERSION_URL y
# CAC_MCP_DOWNLOAD_BASE, como en el de sh.

# Dentro de una función que se llama al final: una descarga del script cortada
# a medias no ejecuta nada.
function Install-CacMcp {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $versionUrl = if ($env:CAC_MCP_VERSION_URL) { $env:CAC_MCP_VERSION_URL } else { 'https://raw.githubusercontent.com/g-studio-labs/c-c/main/mcp/VERSION' }
    $downloadBase = if ($env:CAC_MCP_DOWNLOAD_BASE) { $env:CAC_MCP_DOWNLOAD_BASE } else { 'https://github.com/g-studio-labs/c-c/releases/download' }
    $dir = if ($env:CAC_MCP_INSTALL_DIR) { $env:CAC_MCP_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\cac-mcp' }
    $dest = Join-Path $dir 'cac-mcp.exe'

    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw "install-mcp: there is no cac-mcp binary for Windows $($env:PROCESSOR_ARCHITECTURE) (published: Linux x86_64, macOS arm64 and x86_64, Windows x86_64)"
    }
    $asset = 'cac-mcp-windows-x86_64.exe'

    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    # Al lado de donde va a quedar: el último paso es un cambio de nombre.
    $tmp = Join-Path $dir ".cac-mcp.install.$PID"
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $version = $env:CAC_MCP_VERSION
        if (-not $version) {
            $version = (Invoke-WebRequest -UseBasicParsing -Uri $versionUrl).Content
            if ($version -is [byte[]]) { $version = [Text.Encoding]::UTF8.GetString($version) }
            $version = "$version".Trim()
        }
        # X.Y.Z y nada más: lo que contesta un portal cautivo no es una versión.
        if ($version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
            throw 'install-mcp: the version to install is not X.Y.Z; nothing was installed'
        }

        $release = "$downloadBase/mcp-v$version"
        Write-Host "Downloading cac-mcp $version ($asset)..."
        $sums = Join-Path $tmp 'SHA256SUMS'
        $file = Join-Path $tmp $asset
        Invoke-WebRequest -UseBasicParsing -Uri "$release/SHA256SUMS" -OutFile $sums
        Invoke-WebRequest -UseBasicParsing -Uri "$release/$asset" -OutFile $file

        $published = $null
        foreach ($line in Get-Content $sums) {
            $parts = $line -split '\s+', 2
            if ($parts.Count -eq 2 -and $parts[1].Trim().TrimStart('*') -eq $asset) { $published = $parts[0].ToLower(); break }
        }
        if (-not $published) { throw "install-mcp: SHA256SUMS has no checksum for $asset; nothing was installed" }
        $downloaded = (Get-FileHash -Algorithm SHA256 -Path $file).Hash.ToLower()
        if ($published -ne $downloaded) {
            throw "install-mcp: checksum mismatch for ${asset}: published $published, downloaded $downloaded. Nothing was installed."
        }

        # Un .exe en marcha no se puede pisar ni borrar, pero sí renombrar: el
        # viejo se aparta, el nuevo ocupa su sitio, y si eso falla el viejo
        # vuelve. Lo mismo que hace `cac-mcp update` (update::replace_as).
        $old = "$dest.old"
        if (Test-Path $old) { Remove-Item -Force $old -ErrorAction SilentlyContinue }
        $had = Test-Path $dest
        if ($had) { Move-Item -Force $dest $old }
        try {
            Move-Item -Force $file $dest
        } catch {
            if ($had) { Move-Item -Force $old $dest }
            throw
        }
        if (Test-Path $old) { Remove-Item -Force $old -ErrorAction SilentlyContinue }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    Write-Host "Installed cac-mcp $version at $dest (checksum verified)."
    Write-Host ''
    Write-Host 'To register it in Claude Code, put your personal cac token where it says PASTE_YOUR_TOKEN:'
    Write-Host ''
    Write-Host "  claude mcp add cac -e CAC_URL=https://cac.guz-studio.dev -e CAC_TOKEN=PASTE_YOUR_TOKEN -- `"$dest`""
    Write-Host ''
    Write-Host 'A personal token is created in the cac app (account menu > Connect Claude Code).'
    Write-Host 'If cac-mcp was already registered, reconnect it (/mcp > cac > reconnect) or open a new session.'
    Write-Host "To update later: & `"$dest`" update"
}

Install-CacMcp
