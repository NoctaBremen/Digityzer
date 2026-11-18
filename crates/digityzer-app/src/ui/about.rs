//! Ventana «Acerca de»: versión, créditos, licencia y dependencias.

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, glib};

/// Director del proyecto.
pub const DIRECTOR: &str = "David Gimenez";
/// Programación.
pub const PROGRAMMER: &str = "Claude (Anthropic)";
/// Repositorio.
pub const WEBSITE: &str = "https://github.com/NoctaBremen/Digityzer";

/// Icono de la aplicación (256 px), dentro del binario.
const ICON_PNG: &[u8] = include_bytes!("../../../../data/digityzer-256.png");

/// Créditos para la terminal (`digityzer --version`).
pub fn credits_text() -> String {
    format!("Director del proyecto: {DIRECTOR}\nProgramación: {PROGRAMMER}")
}

/// Bibliotecas y temas que se reconocen en «Acerca de».
pub const THIRD_PARTY: &[&str] = &[
    "Spector: espectrograma, análisis, AIFF y etiquetas (código compartido)",
    "GTK 4 y gtk4-rs · alsa-lib y alsa-rs · RustFFT y RealFFT",
    "rtrb · triple_buffer · blake3 · lofty · serde · clap",
    "Temas: Dracula, Catppuccin, Gruvbox, Nord, Tokyo Night, Rosé Pine, Everforest, \
     Kanagawa, Monokai Pro y Ayu",
];

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(52)
        .css_classes(classes.to_vec())
        .build()
}

/// Muestra la ventana «Acerca de» sobre `parent` (toda en castellano, sin depender de las
/// traducciones de GTK).
pub fn show(parent: &impl IsA<gtk::Window>) {
    let win = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Acerca de Digityzer")
        .resizable(false)
        .default_width(460)
        .build();
    let header = gtk::HeaderBar::builder().show_title_buttons(false).build();
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Cerrar")
        .css_classes(["close-button"])
        .build();
    header.pack_end(&close);
    win.set_titlebar(Some(&header));
    let w = win.clone();
    close.connect_clicked(move |_| w.close());

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(18)
        .margin_bottom(20)
        .margin_start(24)
        .margin_end(24)
        .build();
    if let Ok(tex) = gdk::Texture::from_bytes(&glib::Bytes::from_static(ICON_PNG)) {
        let img = gtk::Image::from_paintable(Some(&tex));
        img.set_pixel_size(128);
        img.set_halign(gtk::Align::Center);
        body.append(&img);
    }
    body.append(&label("Digityzer", &["about-title"]));
    body.append(&label(
        &format!("Versión {}", env!("CARGO_PKG_VERSION")),
        &["dim-label"],
    ));
    body.append(&label(
        "Digitalizador de vinilo bit-perfect: graba desde la Xone:96 por ALSA, mide la \
         captura y la cadena mecánica y guarda un master que nunca se modifica.",
        &[],
    ));
    let link = gtk::LinkButton::with_label(WEBSITE, "Código y manual en GitHub");
    link.set_halign(gtk::Align::Center);
    body.append(&link);

    let credits = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(4)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .build();
    for (row, (role, who)) in [
        ("Director del proyecto", DIRECTOR),
        ("Programación", PROGRAMMER),
    ]
    .into_iter()
    .enumerate()
    {
        let r = gtk::Label::builder()
            .label(role)
            .xalign(1.0)
            .css_classes(["dim-label"])
            .build();
        let n = gtk::Label::builder()
            .label(who)
            .xalign(0.0)
            .css_classes(["about-name"])
            .build();
        credits.attach(&r, 0, row as i32, 1, 1);
        credits.attach(&n, 1, row as i32, 1, 1);
    }
    body.append(&credits);

    let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
    sep.set_margin_top(6);
    body.append(&sep);
    for t in THIRD_PARTY {
        body.append(&label(t, &["dim-label", "about-small"]));
    }
    body.append(&label(
        &format!("© 2026 {DIRECTOR} · Licencia MIT, sin ninguna garantía."),
        &["dim-label", "about-small"],
    ));
    win.set_child(Some(&body));
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creditos_e_icono() {
        assert!(credits_text().contains("Director del proyecto: David Gimenez"));
        assert!(credits_text().contains("Programación: Claude (Anthropic)"));
        assert_eq!(&ICON_PNG[1..4], b"PNG");
    }
}
