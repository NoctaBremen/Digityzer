//! `digityzer-cli`: grabación y diagnóstico sin interfaz. Las órdenes llegan en la fase 1
//! (`probe` en T1.1, `record` en T1.4).

use clap::Parser;

/// Digitalizador de vinilo bit-perfect (línea de órdenes).
#[derive(Debug, Parser)]
#[command(name = "digityzer-cli", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DIGITYZER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let _cli = Cli::parse();
    println!(
        "digityzer-cli {}: todavía no hay órdenes (llegan en la fase 1).",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_bien_definido() {
        Cli::command().debug_assert();
    }
}
