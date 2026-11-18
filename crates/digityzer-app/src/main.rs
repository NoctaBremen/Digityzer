//! `digityzer`: interfaz GTK4 para grabar vinilo (fase 2).

mod ui;

use gtk4 as gtk;

use gtk::prelude::*;
use gtk::{gio, glib};

/// Identificador de la aplicación.
const APP_ID: &str = "io.github.noctabremen.Digityzer";

/// Lo que hacer según los argumentos (en el AppImage, el binario de la interfaz es la entrada
/// única).
#[derive(Debug, PartialEq, Eq)]
enum Launch {
    /// Abrir la interfaz.
    Gui,
    /// Imprimir la versión.
    Version,
    /// Ejecutar `digityzer-cli` con el resto de argumentos.
    Cli(Vec<std::ffi::OsString>),
    /// Capturas de referencia de todos los temas en una carpeta (T2.9f).
    RenderThemes(std::path::PathBuf),
}

fn launch(args: &[std::ffi::OsString]) -> Launch {
    match args.get(1).and_then(|a| a.to_str()) {
        Some("--version" | "-V") => Launch::Version,
        Some("cli") => Launch::Cli(args[2..].to_vec()),
        Some("render-themes") => Launch::RenderThemes(
            args.get(2)
                .map_or_else(|| "capturas-temas".into(), std::path::PathBuf::from),
        ),
        _ => Launch::Gui,
    }
}

/// Ejecuta `digityzer-cli` (junto a este binario) y devuelve su código de salida.
fn run_cli(args: Vec<std::ffi::OsString>) -> glib::ExitCode {
    let cli = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("digityzer-cli")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| "digityzer-cli".into());
    match std::process::Command::new(&cli).args(args).status() {
        Ok(st) => glib::ExitCode::from(st.code().map_or(1, |c| c.clamp(0, 255) as u8)),
        Err(e) => {
            eprintln!("No se pudo ejecutar {}: {e}", cli.display());
            glib::ExitCode::FAILURE
        }
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    match launch(&args) {
        Launch::Version => {
            println!("digityzer {}", env!("CARGO_PKG_VERSION"));
            println!("{}", ui::about::credits_text());
            return glib::ExitCode::SUCCESS;
        }
        Launch::Cli(rest) => return run_cli(rest),
        Launch::RenderThemes(dir) => {
            return match ui::reference::render_all(&dir) {
                Ok(files) => {
                    for f in files {
                        println!("{}", f.display());
                    }
                    glib::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("No se pudieron generar las capturas: {e:#}");
                    glib::ExitCode::FAILURE
                }
            };
        }
        Launch::Gui => {}
    }
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
    // Los argumentos ya se han mirado arriba: GTK no recibe ninguno.
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argumentos_de_entrada() {
        let a = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert_eq!(launch(&a(&["digityzer"])), Launch::Gui);
        assert_eq!(launch(&a(&["digityzer", "--version"])), Launch::Version);
        assert_eq!(
            launch(&a(&["digityzer", "cli", "probe"])),
            Launch::Cli(a(&["probe"]))
        );
        assert_eq!(
            launch(&a(&["digityzer", "render-themes", "/tmp/x"])),
            Launch::RenderThemes("/tmp/x".into())
        );
    }
}
