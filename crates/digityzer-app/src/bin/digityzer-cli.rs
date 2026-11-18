//! `digityzer-cli`: grabación y diagnóstico sin interfaz.
//!
//! - `probe` (T1.1): tarjetas ALSA y lo que admite cada `hw:`.
//! - `record` (T1.4): graba un par de canales a un master WAV/RF64 con su JSON; para con Ctrl-C
//!   sin perder datos.
//! - `verify`: comprueba un master contra la huella de su JSON.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use digityzer_audio::probe::{format_backends, format_report, khz, probe, probe_backends};
use digityzer_audio::recover::{find_partials, is_partial, recover};
use digityzer_audio::{
    AlsaRequest, AlsaSource, AudioBackend, AudioError, CaptureSource, EngineConfig, RecordRequest,
    RecordStatus, Recorder, SampleFormat, StreamConfig, SyntheticSource, verify_master,
};

/// Digitalizador de vinilo bit-perfect (línea de órdenes).
#[derive(Debug, Parser)]
#[command(name = "digityzer-cli", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lista las tarjetas ALSA con las frecuencias, formatos y canales de cada hw: en captura.
    Probe,
    /// Graba un par de canales a un master WAV (RF64 por encima de 4 GiB) con su JSON.
    Record {
        /// Archivo del master (no debe existir).
        out: PathBuf,
        /// Capa de audio: alsa (hw: directo, bit-perfect), pipewire, pulse o default.
        #[arg(short, long, default_value = "alsa", value_parser = parse_backend)]
        backend: AudioBackend,
        /// Dispositivo ALSA hw: (ver `probe`), p. ej. hw:CARD=USB1,DEV=0. Solo con --backend alsa.
        #[arg(short, long)]
        device: Option<String>,
        /// Par de canales, numerados desde 1 como en la mesa (plato A: 3-4, plato B: 5-6).
        #[arg(short, long, value_parser = parse_pair)]
        pair: [u16; 2],
        /// Frecuencia de muestreo en Hz (debe ser nativa: no se remuestrea).
        #[arg(short, long, default_value_t = 96_000)]
        rate: u32,
        /// Formato (S32_LE, S24_3LE o S16_LE). Por defecto, el de más resolución que admita.
        #[arg(short, long, value_parser = parse_format)]
        format: Option<SampleFormat>,
        /// Notas del disco (sello, catálogo, cara, estado…), se guardan en el JSON.
        #[arg(short, long, default_value = "")]
        notes: String,
        /// Para sola tras estos segundos (si no, con Ctrl-C).
        #[arg(long)]
        duration: Option<f64>,
        /// Graba de un origen sintético de 12 canales en lugar de la tarjeta (para probar).
        #[arg(long)]
        simulate: bool,
    },
    /// Recupera grabaciones cortadas de golpe (archivos .partial): las corta en la última muestra
    /// confirmada y las cierra como master con su JSON.
    Recover {
        /// Un archivo .partial o una carpeta con ellos (por defecto, la actual).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Comprueba que el audio de un master coincide con la huella de su JSON.
    Verify {
        /// Archivo del master.
        master: PathBuf,
    },
}

fn parse_pair(s: &str) -> Result<[u16; 2], String> {
    let (a, b) = s
        .split_once(['-', ','])
        .ok_or_else(|| format!("«{s}»: escribe el par como 3-4"))?;
    let n = |x: &str| {
        x.trim()
            .parse::<u16>()
            .ok()
            .filter(|&v| v >= 1)
            .ok_or_else(|| format!("«{x}» no es un canal (desde 1)"))
    };
    let (a, b) = (n(a)?, n(b)?);
    if a == b {
        return Err("los dos canales del par deben ser distintos".into());
    }
    Ok([a, b])
}

fn parse_backend(s: &str) -> Result<AudioBackend, String> {
    AudioBackend::from_id(s)
        .ok_or_else(|| format!("«{s}»: capas posibles: alsa, pipewire, pulse, default"))
}

fn parse_format(s: &str) -> Result<SampleFormat, String> {
    SampleFormat::from_alsa_name(s).ok_or_else(|| {
        format!("«{s}»: formatos que se graban sin convertir: S32_LE, S24_3LE, S16_LE")
    })
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DIGITYZER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            // Dispositivo ocupado (T1.5): mensaje claro y código propio.
            if let Some(AudioError::Busy { .. }) = e.downcast_ref::<AudioError>() {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
            eprintln!("Error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Probe => {
            print!("{}", format_report(&probe()));
            println!();
            print!("{}", format_backends(&probe_backends()));
            Ok(ExitCode::SUCCESS)
        }
        Command::Recover { path } => {
            let list = if path.is_dir() {
                find_partials(&path)
            } else if is_partial(&path) {
                vec![path.clone()]
            } else {
                bail!(
                    "{} no es un archivo .partial ni una carpeta",
                    path.display()
                );
            };
            if list.is_empty() {
                println!("No hay grabaciones que recuperar en {}.", path.display());
                return Ok(ExitCode::SUCCESS);
            }
            let mut ok = true;
            for p in list {
                match recover(&p) {
                    Ok(r) => println!(
                        "Recuperado {} ({}, perdidos ≈ {:.1} s)",
                        r.master.display(),
                        clock(r.info.duration_s),
                        r.info.seconds_lost.unwrap_or(0.0)
                    ),
                    Err(e) => {
                        ok = false;
                        eprintln!("No se pudo recuperar {}: {e}", p.display());
                    }
                }
            }
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Verify { master } => {
            let r = verify_master(&master)
                .with_context(|| format!("no se pudo comprobar {}", master.display()))?;
            if r.ok() {
                println!("{}: correcto ({} bloques)", master.display(), r.blocks);
                Ok(ExitCode::SUCCESS)
            } else {
                println!(
                    "{}: NO coincide (bloques distintos: {:?}; tamaño {}; huella total {})",
                    master.display(),
                    r.mismatched,
                    if r.size_ok { "bien" } else { "distinto" },
                    if r.total_ok { "bien" } else { "distinta" }
                );
                Ok(ExitCode::FAILURE)
            }
        }
        Command::Record {
            out,
            backend,
            device,
            pair,
            rate,
            format,
            notes,
            duration,
            simulate,
        } => record(
            out, backend, device, pair, rate, format, notes, duration, simulate,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn record(
    out: PathBuf,
    backend: AudioBackend,
    device: Option<String>,
    pair: [u16; 2],
    rate: u32,
    format: Option<SampleFormat>,
    notes: String,
    duration: Option<f64>,
    simulate: bool,
) -> anyhow::Result<ExitCode> {
    let source: Box<dyn CaptureSource> = if simulate {
        Box::new(
            SyntheticSource::new(StreamConfig {
                rate,
                format: format.unwrap_or(SampleFormat::S32Le),
                channels: 12,
                period_frames: 1024,
            })
            .realtime(),
        )
    } else if backend == AudioBackend::Alsa {
        let Some(device) = device else {
            bail!("falta --device (ver `digityzer-cli probe`), o elige otra capa con --backend");
        };
        Box::new(AlsaSource::open(&AlsaRequest::new(device, rate, format))?)
    } else {
        if device.is_some() {
            bail!(
                "--device solo vale con --backend alsa; {backend} graba su fuente predeterminada"
            );
        }
        eprintln!(
            "Aviso: {} no es bit-perfect: el servidor de sonido puede remuestrear o mezclar. \
             Para un master bit-perfect usa --backend alsa.",
            backend.label()
        );
        Box::new(AlsaSource::open(&AlsaRequest::plugin(
            backend, rate, format,
        ))?)
    };
    let cfg = source.config();
    eprintln!(
        "Grabando {} ({}): {}, {}, {} canales abiertos, par {}–{} → {}",
        source.card_name(),
        source.device(),
        khz(cfg.rate),
        cfg.format,
        cfg.channels,
        pair[0],
        pair[1],
        out.display()
    );
    let stop = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(sig, Arc::clone(&stop))
            .context("no se pudo instalar el manejador de Ctrl-C")?;
    }
    let rec = Recorder::start(
        source,
        RecordRequest {
            path: out,
            pair,
            notes,
            engine: EngineConfig::default(),
        },
    )?;
    if !simulate && !rec.status().realtime {
        // El estado de tiempo real se fija al arrancar el hilo: se mira tras un instante.
        std::thread::sleep(Duration::from_millis(50));
        if !rec.status().realtime {
            eprintln!(
                "Aviso: sin prioridad de tiempo real (SCHED_FIFO); revisa los límites rtprio \
                 de tu usuario."
            );
        }
    }
    eprintln!("Ctrl-C para parar.");
    let t0 = Instant::now();
    let mut last = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let done_by_time = duration.is_some_and(|d| t0.elapsed().as_secs_f64() >= d);
        if stop.load(Ordering::SeqCst) || done_by_time || rec.capture_finished() {
            break;
        }
        if last.elapsed() >= Duration::from_secs(1) {
            last = Instant::now();
            eprint!("\r{}   ", status_line(&rec.status(), rec.config().rate));
            let _ = std::io::stderr().flush();
        }
    }
    eprintln!("\nParando y cerrando el master…");
    let r = rec.stop()?;
    let i = &r.info;
    println!("Master: {} ({})", r.master.display(), i.container);
    println!("JSON:   {}", r.json.display());
    println!(
        "{} frames, {}, {} xruns, {} frames perdidos en el búfer, tiempo real: {}",
        i.frames,
        clock(i.duration_s),
        i.xruns.len(),
        i.frames_lost_ring,
        if i.realtime { "sí" } else { "no" }
    );
    if i.frames == 0 {
        eprintln!(
            "Aviso: no ha llegado audio de {}. Comprueba la fuente (en PipeWire o PulseAudio, la \
             predeterminada del servidor) y que el servidor esté en marcha.",
            r.info.card_name
        );
    }
    if let Some(e) = r.capture_error {
        eprintln!("La captura se cortó: {e}. Lo grabado hasta ahí está guardado.");
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// `HH:MM:SS`.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// Línea de estado de la grabación.
fn status_line(st: &RecordStatus, rate: u32) -> String {
    let backlog = st.backlog_frames as f64 / f64::from(rate);
    format!(
        "{}  xruns {}  perdidos {}  pendiente {:.1} s",
        clock(st.seconds),
        st.xruns,
        st.frames_lost,
        backlog
    )
    .replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_bien_definido() {
        Cli::command().debug_assert();
    }

    #[test]
    fn pares_y_formatos() {
        assert_eq!(parse_pair("3-4").unwrap(), [3, 4]);
        assert_eq!(parse_pair("5,6").unwrap(), [5, 6]);
        assert!(parse_pair("3").is_err());
        assert!(parse_pair("0-1").is_err());
        assert!(parse_pair("4-4").is_err());
        assert_eq!(parse_format("s24_3le").unwrap(), SampleFormat::S24_3Le);
        assert!(parse_format("S24_LE").is_err());
        assert_eq!(parse_backend("pipewire").unwrap(), AudioBackend::PipeWire);
        assert!(parse_backend("jack").is_err());
    }

    #[test]
    fn reloj_y_estado() {
        assert_eq!(clock(3723.9), "01:02:03");
        let st = RecordStatus {
            frames_written: 0,
            seconds: 61.0,
            xruns: 1,
            frames_lost: 0,
            backlog_frames: 9_600,
            realtime: true,
            recording: true,
        };
        assert_eq!(
            status_line(&st, 96_000),
            "00:01:01  xruns 1  perdidos 0  pendiente 0,1 s"
        );
    }
}
