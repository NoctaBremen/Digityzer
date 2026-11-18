//! Grabación completa: captura + escritor del master + JSON (T1.3 y T1.4).
//!
//! El escritor corre en su propio hilo (no de tiempo real): vacía el ring buffer, escribe el par
//! elegido y recoge los eventos. Al parar, se drena todo lo capturado antes de cerrar: Ctrl-C no
//! pierde datos.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use digityzer_project::{AudioHash, MASTER_INFO_VERSION, MasterInfo, XrunRecord, utc_iso8601};

use crate::engine::{Capture, CaptureEvent, CaptureOutputs, EngineConfig};
use crate::error::AudioError;
use crate::source::{CaptureSource, StreamConfig};
use crate::wav::{BlockHasher, HASH_BLOCK_BYTES, MasterSummary, MasterWriter, read_header};

/// Qué grabar.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordRequest {
    /// Ruta del master (no debe existir; el JSON va al lado con `.json`).
    pub path: PathBuf,
    /// Par de canales, numerado desde 1 como en la mesa (`[3, 4]`).
    pub pair: [u16; 2],
    /// Notas del disco.
    pub notes: String,
    /// Configuración del motor.
    pub engine: EngineConfig,
}

/// Estado en vivo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordStatus {
    /// Frames escritos en el master.
    pub frames_written: u64,
    /// Segundos grabados.
    pub seconds: f64,
    /// Xruns.
    pub xruns: u64,
    /// Frames perdidos por ring buffer lleno.
    pub frames_lost: u64,
    /// Frames capturados pendientes de escribir.
    pub backlog_frames: u64,
    /// `true` si el hilo de captura tiene `SCHED_FIFO`.
    pub realtime: bool,
}

/// Resultado de una grabación cerrada.
#[derive(Debug)]
pub struct RecordingResult {
    /// Master.
    pub master: PathBuf,
    /// JSON.
    pub json: PathBuf,
    /// Lo que se ha guardado en el JSON.
    pub info: MasterInfo,
    /// Error de la captura, si se cortó por un fallo (lo grabado hasta ahí está guardado).
    pub capture_error: Option<AudioError>,
}

struct WriterOutcome {
    summary: MasterSummary,
    xruns: Vec<u64>,
}

/// Grabación en marcha.
pub struct Recorder {
    capture: Capture,
    writer: Option<JoinHandle<Result<WriterOutcome, AudioError>>>,
    written: Arc<AtomicU64>,
    started: SystemTime,
    request: RecordRequest,
    json: PathBuf,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// Crea el master y arranca la captura.
    pub fn start(
        source: Box<dyn CaptureSource>,
        request: RecordRequest,
    ) -> Result<Self, AudioError> {
        let cfg = source.config();
        let json = MasterInfo::path_for(&request.path);
        if json.exists() {
            return Err(AudioError::Invalid(format!(
                "ya existe {}; no se sobrescribe",
                json.display()
            )));
        }
        let pair0 = [
            request.pair[0].wrapping_sub(1),
            request.pair[1].wrapping_sub(1),
        ];
        let writer = MasterWriter::create(&request.path, cfg.format, cfg.rate, cfg.channels, pair0)
            .map_err(|e| match e {
                AudioError::Invalid(_) => AudioError::Invalid(format!(
                    "par de canales {}–{} no válido: {} tiene {} canales",
                    request.pair[0],
                    request.pair[1],
                    source.card_name(),
                    cfg.channels
                )),
                e => e,
            })?;
        let started = SystemTime::now();
        let (capture, outputs) = match Capture::start(source, request.engine) {
            Ok(c) => c,
            Err(e) => {
                // El master vacío no sirve: se quita (todavía no tiene datos).
                drop(writer);
                let _ = std::fs::remove_file(&request.path);
                return Err(e);
            }
        };
        let written = Arc::new(AtomicU64::new(0));
        let w = Arc::clone(&written);
        let handle = std::thread::Builder::new()
            .name("digityzer-escritor".into())
            .spawn(move || write_loop(writer, outputs, w))
            .map_err(|e| AudioError::io("hilo del escritor", e))?;
        Ok(Self {
            capture,
            writer: Some(handle),
            written,
            started,
            request,
            json,
        })
    }

    /// Configuración del flujo.
    pub fn config(&self) -> StreamConfig {
        self.capture.config()
    }

    /// Estado en vivo.
    pub fn status(&self) -> RecordStatus {
        let c = self.capture.counters();
        let captured = c.frames.load(Ordering::Acquire);
        let lost = c.frames_lost.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Relaxed);
        RecordStatus {
            frames_written: written,
            seconds: written as f64 / f64::from(self.config().rate),
            xruns: c.xruns.load(Ordering::Relaxed),
            frames_lost: lost,
            backlog_frames: captured.saturating_sub(lost).saturating_sub(written),
            realtime: c.realtime.load(Ordering::Relaxed),
        }
    }

    /// `true` si la captura ha terminado sola (fin del origen o error).
    pub fn capture_finished(&self) -> bool {
        self.capture.is_finished()
    }

    /// Para, vacía lo pendiente, cierra el master y escribe el JSON.
    pub fn stop(mut self) -> Result<RecordingResult, AudioError> {
        let capture_error = self.capture.stop().err();
        let outcome = self
            .writer
            .take()
            .ok_or(AudioError::ThreadPanicked("escritor"))?
            .join()
            .map_err(|_| AudioError::ThreadPanicked("escritor"))??;
        let cfg = self.capture.config();
        let rate = f64::from(cfg.rate);
        let s = outcome.summary;
        let info = MasterInfo {
            version: MASTER_INFO_VERSION,
            digityzer_version: crate::VERSION.to_string(),
            device: self.capture.device().to_string(),
            card_name: self.capture.card_name().to_string(),
            sample_rate: cfg.rate,
            sample_format: cfg.format.alsa_name().to_string(),
            container_bits: cfg.format.bits(),
            lsb_zero_bits: s.lsb_zero_bits,
            channels_opened: cfg.channels,
            channel_pair: self.request.pair,
            container: if s.rf64 { "RF64" } else { "WAV" }.to_string(),
            frames: s.frames,
            duration_s: s.frames as f64 / rate,
            data_bytes: s.data_bytes,
            started_at: utc_iso8601(self.started),
            finished_at: utc_iso8601(SystemTime::now()),
            realtime: self.capture.counters().realtime.load(Ordering::Relaxed),
            xruns: outcome
                .xruns
                .iter()
                .map(|&frame| XrunRecord {
                    frame,
                    seconds: frame as f64 / rate,
                })
                .collect(),
            frames_lost_ring: self.capture.counters().frames_lost.load(Ordering::Relaxed),
            hash: AudioHash {
                algorithm: "blake3".into(),
                block_bytes: HASH_BLOCK_BYTES,
                blocks: s.block_hashes,
                total: s.total_hash,
            },
            notes: self.request.notes.clone(),
        };
        info.write_new(&self.json)?;
        Ok(RecordingResult {
            master: s.path,
            json: self.json.clone(),
            info,
            capture_error,
        })
    }
}

fn write_loop(
    mut writer: MasterWriter,
    mut out: CaptureOutputs,
    written: Arc<AtomicU64>,
) -> Result<WriterOutcome, AudioError> {
    let mut xruns = Vec::new();
    loop {
        while let Ok(e) = out.events.pop() {
            if let CaptureEvent::Xrun { frame } = e {
                xruns.push(frame);
            }
        }
        let n = out.audio.slots();
        if n > 0 {
            if let Ok(chunk) = out.audio.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                writer.write_interleaved(a)?;
                writer.write_interleaved(b)?;
                chunk.commit_all();
            }
            written.store(writer.frames(), Ordering::Relaxed);
        } else if out.audio.is_abandoned() {
            // El hilo de captura ha terminado y no queda nada: últimos eventos y cierre.
            while let Ok(e) = out.events.pop() {
                if let CaptureEvent::Xrun { frame } = e {
                    xruns.push(frame);
                }
            }
            break;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let summary = writer.finalize()?;
    Ok(WriterOutcome { summary, xruns })
}

/// Resultado de comprobar un master contra su JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Bloques comprobados.
    pub blocks: usize,
    /// Índices de los bloques que no coinciden.
    pub mismatched: Vec<usize>,
    /// `true` si la huella total coincide.
    pub total_ok: bool,
    /// `true` si el tamaño del audio coincide.
    pub size_ok: bool,
}

impl VerifyReport {
    /// `true` si todo coincide.
    pub fn ok(&self) -> bool {
        self.mismatched.is_empty() && self.total_ok && self.size_ok
    }
}

/// Comprueba que el audio del master coincide con la huella de su JSON.
pub fn verify_master(master: &Path) -> Result<VerifyReport, AudioError> {
    let info = MasterInfo::read(&MasterInfo::path_for(master))?;
    let io = |e| AudioError::io(master, e);
    let mut f = BufReader::with_capacity(1 << 20, File::open(master).map_err(io)?);
    let h = read_header(&mut f).map_err(io)?;
    f.seek(SeekFrom::Start(h.data_offset)).map_err(io)?;
    let mut hasher = BlockHasher::new(info.hash.block_bytes);
    let mut left = h.data_bytes;
    let mut buf = vec![0u8; 1 << 20];
    let started = Instant::now();
    while left > 0 {
        let n = (buf.len() as u64).min(left) as usize;
        f.read_exact(&mut buf[..n]).map_err(io)?;
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    tracing::debug!("verificado en {:?}", started.elapsed());
    let (blocks, total) = hasher.finish();
    let mismatched = (0..blocks.len().max(info.hash.blocks.len()))
        .filter(|&i| blocks.get(i) != info.hash.blocks.get(i))
        .collect();
    Ok(VerifyReport {
        blocks: blocks.len(),
        mismatched,
        total_ok: total == info.hash.total,
        size_ok: h.data_bytes == info.data_bytes,
    })
}
