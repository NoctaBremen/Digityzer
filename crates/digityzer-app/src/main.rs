//! `digityzer`: interfaz GTK4 para grabar vinilo (fase 2).

mod ui;

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
    app.connect_activate(ui::window::build);
    app.run()
}
