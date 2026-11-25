#!/usr/bin/env bash
# El paquete de macOS: `CAC Nativa.app` dentro de un `.dmg`.
#
# A mano, con lo que trae el sistema (`codesign`, `hdiutil`): un `.app` es una
# carpeta con un `Info.plist`, y para un binario que no es de Tauri ninguna
# herramienta ahorra más de lo que esconde. Tampoco hay nada que instalar en
# los minutos de macOS, que son los caros.
#
# **Sin firmar de verdad.** No hay certificados de Apple en este proyecto, así
# que la firma es «ad hoc»: basta para que arranque en Apple Silicon, y no
# para Gatekeeper, que avisará de que no puede comprobar quién la hizo
# (se abre con clic derecho → Abrir, o desde Ajustes → Privacidad y seguridad).

source "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

BINARY="$RELEASE_DIR/$APP_BINARY"
require_binary "$BINARY"

ARCH="$(uname -m)"
DMG_ROOT="$STAGE_DIR/dmg"
APP="$DMG_ROOT/$APP_NAME.app"
rm -rf "$DMG_ROOT"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$OUT_DIR"

cp "$BINARY" "$APP/Contents/MacOS/$APP_BINARY"
cp "$PACKAGING_DIR/icons/icon.icns" "$APP/Contents/Resources/icon.icns"
# La licencia de las tipografías que lleva el binario (OFL).
mkdir -p "$APP/Contents/Resources/licenses"
cp "$CRATE_DIR"/assets/fonts/LICENSE-*.txt "$APP/Contents/Resources/licenses/"
sed -e "s/__APP_NAME__/$APP_NAME/g" \
    -e "s/__APP_ID__/$APP_ID/g" \
    -e "s/__APP_BINARY__/$APP_BINARY/g" \
    -e "s/__APP_VERSION__/$APP_VERSION/g" \
    "$PACKAGING_DIR/macos/Info.plist" > "$APP/Contents/Info.plist"

# Que no haya quedado ninguna marca sin sustituir: un `.app` con
# `__APP_ID__` de identificador se instala y nadie lo nota hasta más tarde.
if grep -q '__APP_' "$APP/Contents/Info.plist"; then
  echo "packaging: quedan marcas sin sustituir en Info.plist" >&2
  exit 1
fi
plutil -lint "$APP/Contents/Info.plist"

# --- Firma ------------------------------------------------------------------
# Con la identidad fija de `MACOS_SIGNING_IDENTITY` (el mismo certificado
# autofirmado que la app de Tauri, ver `app-release.yml`): macOS ata el permiso
# del micrófono y la cámara a la identidad de la firma, y con la firma ad hoc
# cambiaba en cada build, así que el aviso volvía en cada llamada y con cada
# actualización. No es una firma de Apple: Gatekeeper sigue avisando.
#
# **Sin `--options runtime`, a propósito.** Con el runtime reforzado y sin
# entitlements macOS deniega el micrófono sin preguntar; sólo hace falta para
# notarizar, que aquí no hay. Guardián: `mcp/tests/desktop_release.rs`.
#
# En la release (`REQUIRE_SIGNING_IDENTITY=1`, lo pone el workflow) sin
# identidad se aborta: nunca sale una release firmada ad hoc sin decirlo.
# Sin eso (a mano, en local) se firma ad hoc (`-`), que basta para arrancar.
#
# CUANDO HAYA CERTIFICADOS DE APPLE: «Developer ID Application» con
# `--options runtime --timestamp` y entitlements de audio y cámara, y después
# `xcrun notarytool submit … --wait` y `xcrun stapler staple` sobre el .dmg.
if [ -z "${MACOS_SIGNING_IDENTITY:-}" ]; then
  if [ "${REQUIRE_SIGNING_IDENTITY:-}" = "1" ]; then
    echo "packaging: falta MACOS_SIGNING_IDENTITY y esto es una release: no se firma ad hoc en silencio" >&2
    exit 1
  fi
  MACOS_SIGNING_IDENTITY="-"
fi
codesign --force --deep --sign "$MACOS_SIGNING_IDENTITY" "$APP"
codesign --verify --strict "$APP"

# --- .dmg -------------------------------------------------------------------
# Con el acceso a Aplicaciones al lado, para que instalar sea arrastrar.
ln -s /Applications "$DMG_ROOT/Applications"
DMG="$OUT_DIR/$APP_BINARY-$APP_VERSION-macos-$ARCH.dmg"
rm -f "$DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$DMG_ROOT" -fs HFS+ -format UDZO -ov "$DMG"

ls -l "$OUT_DIR"
