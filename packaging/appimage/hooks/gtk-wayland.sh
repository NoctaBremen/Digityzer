#! /usr/bin/env bash
# Hook de AppRun de Digityzer (sustituye al de linuxdeploy-plugin-gtk; copiado de Spector).
#
# Diferencias con el hook original del plugin:
#   - NO exporta GDK_BACKEND=x11: Digityzer es nativa en Wayland (Hyprland). Si el usuario
#     define GDK_BACKEND, GTK lo respeta.
#   - NO fija GTK_THEME: se usa el tema del sistema.
#   - Respeta GSK_RENDERER si el usuario lo define (p. ej. GSK_RENDERER=cairo o ngl
#     como solución de emergencia ante problemas del driver).
#   - Datos del AppDir delante de los del sistema en XDG_DATA_DIRS.
#
# build-appimage.sh añade debajo de este bloque las rutas que calcula el plugin
# (esquemas de GLib, módulos de GTK, loaders de gdk-pixbuf...), sin GDK_BACKEND ni GTK_THEME.

export APPDIR="${APPDIR:-"$(dirname "$(realpath "$0")")"}" # Permite ejecutar el AppImage extraído
export GTK_DATA_PREFIX="$APPDIR"
export XDG_DATA_DIRS="$APPDIR/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
if [ -n "${GSK_RENDERER:-}" ]; then
    export GSK_RENDERER
fi

# --- Rutas generadas por linuxdeploy-plugin-gtk (filtradas) ---
