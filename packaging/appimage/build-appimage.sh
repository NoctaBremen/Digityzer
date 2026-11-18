#!/usr/bin/env bash
# Construye Digityzer-<VERSION>-x86_64.AppImage (adaptado del script de Spector).
#
# Uso:   packaging/appimage/build-appimage.sh            # VERSION = v<versión de Cargo>
#        VERSION=v0.1 packaging/appimage/build-appimage.sh
#
# Requisitos (Ubuntu 24.04, igual que el CI): libgtk-4-dev libasound2-dev pkg-config
# librsvg2-dev file desktop-file-utils, curl y Rust estable.
#
# El AppImage lleva los dos binarios: sin argumentos abre la interfaz; `cli …` ejecuta
# digityzer-cli (p. ej. `./Digityzer-v0.1.0-x86_64.AppImage cli probe`). libasound viene
# siempre del sistema (está en la excludelist de linuxdeploy), con su configuración de ALSA.
#
# Herramientas descargadas en versiones FIJADAS y comprobadas con SHA-256:
#   - linuxdeploy (incluye el plugin de salida appimage)
#   - linuxdeploy-plugin-gtk (por commit)
# Para actualizarlas, cambia la URL Y el hash a la vez.
#
# Salida: dist/Digityzer-<VERSION>-x86_64.AppImage y su .sha256.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PKG="$ROOT/packaging/appimage"
TOOLS="$PKG/tools"
BUILD="$ROOT/target/appimage"
APPDIR="$BUILD/AppDir"
DIST="$ROOT/dist"
ARCH="x86_64"

LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_SHA256="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"
PLUGIN_GTK_URL="https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/7a3fbc31a9e5075073ff8790f26effbac5f84453/linuxdeploy-plugin-gtk.sh"
PLUGIN_GTK_SHA256="b0f4cbc684a0103a9651f0955b635eaea0096b3a66c0f5a2c2aa337960375171"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# Descarga $1 en $2 si no existe o si su hash no coincide con $3.
fetch() {
    local url="$1" dest="$2" sha="$3"
    if [ -f "$dest" ] && echo "$sha  $dest" | sha256sum --check --status; then
        chmod +x "$dest"
        return 0
    fi
    log "Descargando $(basename "$dest")"
    curl -fsSL --retry 3 -o "$dest.part" "$url"
    echo "$sha  $dest.part" | sha256sum --check --status \
        || die "SHA-256 incorrecto para $url (¿se ha cambiado la versión sin actualizar el hash?)"
    mv "$dest.part" "$dest"
    chmod +x "$dest"
}

# --- Versión ---
if [ -z "${VERSION:-}" ]; then
    CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
    VERSION="v${CARGO_VERSION:-dev}"
fi
OUTPUT_NAME="Digityzer-${VERSION}-${ARCH}.AppImage"
log "Construyendo $OUTPUT_NAME"

# --- Herramientas ---
mkdir -p "$TOOLS"
fetch "$LINUXDEPLOY_URL" "$TOOLS/linuxdeploy-x86_64.AppImage" "$LINUXDEPLOY_SHA256"
fetch "$PLUGIN_GTK_URL" "$TOOLS/linuxdeploy-plugin-gtk.sh" "$PLUGIN_GTK_SHA256"
# linuxdeploy busca los plugins en el PATH. Sin FUSE (CI, contenedores): extraer y ejecutar.
export PATH="$TOOLS:$PATH"
export APPIMAGE_EXTRACT_AND_RUN=1

# --- Compilación ---
log "cargo build --release -p digityzer-app"
cargo build --release --locked -p digityzer-app --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release/digityzer"
CLI="$ROOT/target/release/digityzer-cli"
[ -x "$BIN" ] || die "No se encuentra $BIN"
[ -x "$CLI" ] || die "No se encuentra $CLI"

# Bibliotecas que deben venir SIEMPRE del sistema (drivers gráficos: Mesa/AMD en Arch).
# La excludelist de linuxdeploy ya quita libGL/libEGL/libdrm/libgbm, pero no el
# cargador de Vulkan ni libwayland-egl, que GTK 4.14 enlaza.
SYSTEM_LIBS=(
    'libvulkan.so*' 'libwayland-egl.so*'
    'libGL.so*' 'libGLX*.so*' 'libGLdispatch.so*' 'libEGL*.so*' 'libOpenGL.so*'
    'libdrm*.so*' 'libgbm.so*'
)
EXCLUDE_ARGS=()
for lib in "${SYSTEM_LIBS[@]}"; do EXCLUDE_ARGS+=("--exclude-library=$lib"); done

# Borra del AppDir cualquier biblioteca de SYSTEM_LIBS que se haya colado (p. ej. vía el plugin).
purge_system_libs() {
    local lib
    for lib in "${SYSTEM_LIBS[@]}"; do
        find "$APPDIR" -name "$lib" \( -type f -o -type l \) -delete
    done
}

# Falla si el AppDir contiene alguna biblioteca de SYSTEM_LIBS.
check_system_libs() {
    local lib found=""
    for lib in "${SYSTEM_LIBS[@]}"; do
        found+="$(find "$APPDIR" -name "$lib" \( -type f -o -type l \))"
    done
    [ -z "$found" ] || die "El AppDir contiene bibliotecas gráficas del sistema: $found"
}

# --- AppDir + bibliotecas + GTK ---
rm -rf "$BUILD"
mkdir -p "$BUILD" "$DIST"
log "linuxdeploy + plugin gtk"
(
    cd "$BUILD"
    DEPLOY_GTK_VERSION=4 linuxdeploy-x86_64.AppImage \
        --appdir "$APPDIR" \
        --plugin gtk \
        --executable "$BIN" \
        --executable "$CLI" \
        --desktop-file "$PKG/digityzer.desktop" \
        --icon-file "$PKG/digityzer.svg" \
        "${EXCLUDE_ARGS[@]}"
)
purge_system_libs

# --- Hook propio: sin GDK_BACKEND=x11 ni GTK_THEME (D13) ---
HOOK="$APPDIR/apprun-hooks/linuxdeploy-plugin-gtk.sh"
[ -f "$HOOK" ] || die "El plugin gtk no generó $HOOK"
log "Sustituyendo el hook del plugin gtk por hooks/gtk-wayland.sh"
GENERATED="$(grep -E '^export [A-Z_]+=' "$HOOK" \
    | grep -vE '^export (GDK_BACKEND|GTK_THEME|APPDIR|GTK_DATA_PREFIX|XDG_DATA_DIRS)=' || true)"
{
    cat "$PKG/hooks/gtk-wayland.sh"
    printf '%s\n' "$GENERATED"
} > "$HOOK.new"
mv "$HOOK.new" "$HOOK"
chmod +x "$HOOK"
if grep -qE '^[^#]*(GDK_BACKEND|GTK_THEME)=' "$HOOK"; then
    die "El hook final todavía fuerza GDK_BACKEND o GTK_THEME"
fi
grep -q 'GSETTINGS_SCHEMA_DIR=' "$HOOK" || die "El hook final no contiene GSETTINGS_SCHEMA_DIR"

# --- Empaquetado ---
log "Generando el AppImage"
(
    cd "$BUILD"
    LDAI_OUTPUT="$OUTPUT_NAME" OUTPUT="$OUTPUT_NAME" \
        linuxdeploy-x86_64.AppImage --appdir "$APPDIR" --output appimage "${EXCLUDE_ARGS[@]}"
)
# El AppImage se genera a partir del AppDir: si este quedó con bibliotecas del sistema, se descarta.
if ! (check_system_libs); then
    rm -f "$BUILD/$OUTPUT_NAME"
    die "AppImage descartado: contiene bibliotecas gráficas del sistema"
fi
mv -f "$BUILD/$OUTPUT_NAME" "$DIST/$OUTPUT_NAME"
(cd "$DIST" && sha256sum "$OUTPUT_NAME" > "$OUTPUT_NAME.sha256")

log "Listo: dist/$OUTPUT_NAME"
