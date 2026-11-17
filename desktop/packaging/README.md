# Los instaladores de la app nativa

Cómo se construye cada uno a mano. En el CI lo hace
`.github/workflows/desktop-release.yml`, que se lanza a mano, no publica nada y
deja los instaladores como artefactos de la ejecución.

Quién es la app (nombre, identificador, binario) está en **un solo sitio**:
`identity.env`. La versión sale de `Cargo.toml`. Para renombrarla se cambia
`identity.env` y se corre `cargo test identity`: la prueba dice qué fichero de
Linux hay que tocar a mano (el `.desktop` y `[package.metadata.deb]`).

Empaquetar **no compila**: siempre va detrás de

```sh
cd desktop && cargo build --release --locked
```

Todo sale en `desktop/target/packaging/out/`.

## Linux — `.deb` y `.tar.gz`

```sh
cargo install --locked cargo-deb@3.8.0   # una vez
bash packaging/linux/package.sh
```

- El `.deb` instala `/usr/bin/cac-desktop`, el lanzador y los iconos (hicolor).
- El `.tar.gz` lleva el mismo árbol (`bin/`, `share/`) para lo que no es Debian:
  `tar -xzf cac-desktop-*.tar.gz --strip-components=1 -C ~/.local` (con
  `~/.local/bin` en el `PATH`, que es por donde el lanzador encuentra el binario).
- Necesita en la máquina: Vulkan con su controlador (`libvulkan1` y el de la
  gráfica), `libxkbcommon`, `libxcb` y, en Wayland, `libwayland-client`.
- El binario pide la glibc de donde se compiló: el del CI (Ubuntu 24.04) no
  arranca en una distribución más vieja.
- No hay AppImage, a propósito: el porqué está en `linux/package.sh`.

## macOS — `.dmg`

```sh
bash packaging/macos/package.sh
```

Sólo con herramientas del sistema. La arquitectura es la de la máquina que
compila (en el CI, Apple Silicon); mínimo macOS 12. **Firma «ad hoc», sin
notarizar**: Gatekeeper avisa al abrirla (clic derecho → Abrir). Dónde irían la
firma y la notarización está comentado en el script.

## Windows — instalador y `.zip`

```powershell
./packaging/windows/package.ps1
```

Necesita NSIS 3 (`choco install nsis`). El instalador es por usuario, sin
permisos de administrador; el `.zip` lleva el `.exe` suelto, que no necesita
nada a su lado. **Sin firmar**: SmartScreen avisa al ejecutarlo.
