//! `digityzer`: interfaz GTK4. De momento solo abre la ventana principal vacía; la ventana de
//! grabación llega en T2.1.

use gtk4 as gtk;

use gtk::prelude::*;
use gtk::{gio, glib};

/// Identificador de la aplicación.
const APP_ID: &str = "io.github.noctabremen.Digityzer";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DIGITYZER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| audio_core::gtk::load_css());
    app.connect_activate(build_window);
    app.run()
}

fn build_window(app: &gtk::Application) {
    let placeholder = gtk::Label::builder()
        .label("Digityzer")
        .css_classes(["title-1", "dim-label"])
        .build();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Digityzer")
        .default_width(960)
        .default_height(640)
        .child(&placeholder)
        .build();
    window.present();
}
