//! Estilo base de la interfaz, copiado de `spector-gtk/src/app.rs` (constante `CSS` y
//! `load_css`). Usa los colores con nombre del tema Adwaita de GTK 4.

use gtk4 as gtk;

/// Hoja de estilo base (la misma que Spector).
pub const CSS: &str = "
.verdict-dot { min-width: 10px; min-height: 10px; border-radius: 9999px;
               background-color: alpha(@window_fg_color, 0.25); }
.verdict-dot.valid { background-color: @success_color; }
.verdict-dot.suspicious { background-color: @warning_color; }
.verdict-dot.fake { background-color: @error_color; }
.manual-tag { padding: 0 6px; border-radius: 6px; background-color: alpha(@accent_bg_color, 0.3); }
/* Cifras de ancho fijo en la lista, la barra de progreso y la ficha (no bailan al cambiar). */
.numeric, columnview label.caption { font-feature-settings: \"tnum\"; }
/* Lista algo más densa que la de por defecto. */
columnview.data-table > listview > row { min-height: 26px; }
";

/// Carga [`CSS`] en la pantalla por defecto con prioridad de aplicación. Llamar en `startup`.
pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
