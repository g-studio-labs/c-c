#!/bin/sh
# Instala cac-mcp, el servidor MCP de cac, en esta máquina (Linux y macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/g-studio-labs/c-c/main/scripts/install-mcp.sh | sh
#
# Baja el binario de su release (etiqueta mcp-vX.Y.Z), comprueba su SHA-256
# contra el SHA256SUMS publicado y lo deja en ~/.local/bin/cac-mcp. Si la suma
# no coincide no instala nada. No pide permisos de administrador, y no pide,
# lee ni imprime ningún token.
#
# Es la misma lógica que `cac-mcp update` y que la app de escritorio
# (mcp/src/update.rs): los mismos nombres de fichero, las mismas direcciones y
# la misma carpeta. Los prueba mcp/tests/install_script.rs.
#
# Lo que se puede cambiar por el entorno:
#   CAC_MCP_VERSION        la versión a instalar (por defecto, la de mcp/VERSION en main)
#   CAC_MCP_INSTALL_DIR    dónde (por defecto, ~/.local/bin)
#   CAC_MCP_VERSION_URL    otra dirección para mcp/VERSION
#   CAC_MCP_DOWNLOAD_BASE  otra base para los ficheros de las releases
set -eu

# Todo dentro de una función que se llama en la última línea: si la descarga
# del script se corta a medias, la shell no ha ejecutado nada todavía.
main() {
    version_url="${CAC_MCP_VERSION_URL:-https://raw.githubusercontent.com/g-studio-labs/c-c/main/mcp/VERSION}"
    download_base="${CAC_MCP_DOWNLOAD_BASE:-https://github.com/g-studio-labs/c-c/releases/download}"
    dir="${CAC_MCP_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"
    dest="$dir/cac-mcp"

    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os $arch" in
        "Linux x86_64" | "Linux amd64") asset="cac-mcp-linux-x86_64" ;;
        "Darwin arm64" | "Darwin aarch64") asset="cac-mcp-macos-arm64" ;;
        "Darwin x86_64") asset="cac-mcp-macos-x86_64" ;;
        *) fail "there is no cac-mcp binary for $os $arch (published: Linux x86_64, macOS arm64 and x86_64, Windows x86_64)" ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        fetcher=curl
    elif command -v wget >/dev/null 2>&1; then
        fetcher=wget
    else
        fail "curl or wget is needed to download cac-mcp"
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        hasher=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        hasher=shasum
    else
        fail "sha256sum or shasum is needed to verify the download; nothing was installed"
    fi

    mkdir -p "$dir"
    # Al lado de donde va a quedar, no en /tmp: así el último paso es un
    # cambio de nombre dentro de la misma carpeta, que es atómico.
    tmp="$dir/.cac-mcp.install.$$"
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    rm -rf "$tmp"
    mkdir "$tmp"

    version="${CAC_MCP_VERSION:-}"
    if [ -z "$version" ]; then
        fetch "$version_url" "$tmp/VERSION" || fail "could not read the latest version from $version_url"
        version="$(tr -d ' \t\r\n' <"$tmp/VERSION")"
    fi
    # X.Y.Z y nada más: lo que contesta un portal cautivo no es una versión.
    if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        fail "the version to install is not X.Y.Z; nothing was installed"
    fi

    release="$download_base/mcp-v$version"
    echo "Downloading cac-mcp $version ($asset)..."
    fetch "$release/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $release/SHA256SUMS (is version $version published?)"
    fetch "$release/$asset" "$tmp/$asset" || fail "could not download $release/$asset"

    published="$(awk -v file="$asset" '{ name = $2; sub(/^\*/, "", name); if (name == file) { print tolower($1); exit } }' "$tmp/SHA256SUMS")"
    [ -n "$published" ] || fail "SHA256SUMS has no checksum for $asset; nothing was installed"
    if [ "$hasher" = sha256sum ]; then
        downloaded="$(sha256sum "$tmp/$asset" | awk '{ print $1 }')"
    else
        downloaded="$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')"
    fi
    if [ "$published" != "$downloaded" ]; then
        fail "checksum mismatch for $asset: published $published, downloaded $downloaded. Nothing was installed."
    fi

    chmod 755 "$tmp/$asset"
    mv -f "$tmp/$asset" "$dest"

    echo "Installed cac-mcp $version at $dest (checksum verified)."
    case ":${PATH:-}:" in
        *":$dir:"*) ;;
        *) echo "Note: $dir is not in your PATH. The command below uses the full path, so it works anyway." ;;
    esac
    cat <<EOF

To register it in Claude Code, put your personal cac token where it says PASTE_YOUR_TOKEN:

  claude mcp add cac -e CAC_URL=https://cac.guz-studio.dev -e CAC_TOKEN=PASTE_YOUR_TOKEN -- "$dest"

A personal token is created in the cac app (account menu > Connect Claude Code).
If cac-mcp was already registered, reconnect it (/mcp > cac > reconnect) or open a new session.
To update later: "$dest" update
EOF
}

fail() {
    echo "install-mcp: $1" >&2
    exit 1
}

# fetch <url> <fichero>: lo baja entero o falla; nunca deja medio fichero por bueno.
fetch() {
    if [ "$fetcher" = curl ]; then
        curl -fsSL --retry 2 -o "$2" "$1"
    else
        wget -q -O "$2" "$1"
    fi
}

main "$@"
