//! Sesión de captura y grabación (T1.3, T1.4 y T2.1).
//!
//! Una [`Session`] abre el origen y lo mantiene «armado»: el hilo de captura llena el ring
//! buffer, el escritor lo vacía sin guardar nada y el análisis publica los medidores. Al pulsar
//! grabar, el escritor empieza a guardar el par elegido desde el siguiente frame completo; al
//! parar, cierra el master y se escribe el JSON, sin soltar el dispositivo.
//!
//! [`Recorder`] es la forma corta para la terminal: abre, graba y, al parar, primero detiene la
//! captura y después vacía todo lo pendiente antes de cerrar: Ctrl-C no pierde datos.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use digityzer_dsp::{LiveAnalyzer, LiveSnapshot};
use digityzer_project::{AudioHash, MASTER_INFO_VERSION, MasterInfo, XrunRecord, utc_iso8601};

use crate::engine::{Capture, CaptureEvent, EngineConfig};
use crate::error::AudioError;
use crate::format::SampleFormat;
use crate::source::{CaptureSource, StreamConfig};
use crate::wav::{BlockHasher, HASH_BLOCK_BYTES, MasterSummary, MasterWriter, read_header};

/// Cada cuánto publica el análisis una instantánea (≈ 60 por segundo).
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(16);

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
    /// Frames escritos en el master en curso (0 si no se graba).
    pub frames_written: u64,
    /// Segundos grabados.
    pub seconds: f64,
    /// Xruns desde que se abrió el dispositivo.
    pub xruns: u64,
    /// Frames perdidos por ring buffer lleno.
    pub frames_lost: u64,
    /// Frames capturados pendientes de pasar por el escritor.
    pub backlog_frames: u64,
    /// `true` si el hilo de captura tiene `SCHED_FIFO`.
    pub realtime: bool,
    /// `true` si se está grabando.
    pub recording: bool,
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

enum WriterCmd {
    Start(Box<MasterWriter>, mpsc::Sender<()>),
    Stop(mpsc::Sender<Result<WriterOutcome, AudioError>>),
}

struct Active {
    path: PathBuf,
    json: PathBuf,
    notes: String,
    started: SystemTime,
}

/// Dispositivo abierto, con medidores en vivo y grabación opcional.
pub struct Session {
    capture: Capture,
    pair: [u16; 2],
    cmd: Option<mpsc::Sender<WriterCmd>>,
    writer: Option<JoinHandle<()>>,
    written: Arc<AtomicU64>,
    consumed: Arc<AtomicU64>,
    analysis_stop: Arc<AtomicBool>,
    analysis: Option<JoinHandle<()>>,
    snapshot: triple_buffer::Output<LiveSnapshot>,
    active: Option<Active>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("device", &self.capture.device())
            .field("pair", &self.pair)
            .field("recording", &self.active.is_some())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Abre el origen y empieza a capturar y medir el par `pair` (desde 1), sin grabar.
    pub fn open(
        source: Box<dyn CaptureSource>,
        pair: [u16; 2],
        engine: EngineConfig,
    ) -> Result<Self, AudioError> {
        let cfg = source.config();
        if pair[0] == pair[1] || pair.iter().any(|&c| c == 0 || c > cfg.channels) {
            return Err(AudioError::Invalid(format!(
                "par de canales {}–{} no válido: {} tiene {} canales",
                pair[0],
                pair[1],
                source.card_name(),
                cfg.channels
            )));
        }
        let (capture, outputs) = Capture::start(source, engine)?;
        let written = Arc::new(AtomicU64::new(0));
        let consumed = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel();
        let (w, c) = (Arc::clone(&written), Arc::clone(&consumed));
        let fb = cfg.frame_bytes() as u64;
        let writer = std::thread::Builder::new()
            .name("digityzer-escritor".into())
            .spawn(move || writer_loop(outputs.audio, outputs.events, rx, fb, w, c))
            .map_err(|e| AudioError::io("hilo del escritor", e))?;

        let (input, snapshot) = triple_buffer::TripleBuffer::new(&LiveSnapshot::default()).split();
        let analysis_stop = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&analysis_stop);
        let pair0 = [usize::from(pair[0] - 1), usize::from(pair[1] - 1)];
        let analysis = std::thread::Builder::new()
            .name("digityzer-analisis".into())
            .spawn(move || analysis_loop(outputs.analysis, input, cfg, pair0, stop))
            .map_err(|e| AudioError::io("hilo del análisis", e))?;
        Ok(Self {
            capture,
            pair,
            cmd: Some(tx),
            writer: Some(writer),
            written,
            consumed,
            analysis_stop,
            analysis: Some(analysis),
            snapshot,
            active: None,
        })
    }

    /// Configuración del flujo.
    pub fn config(&self) -> StreamConfig {
        self.capture.config()
    }

    /// Par medido y grabado (desde 1).
    pub fn pair(&self) -> [u16; 2] {
        self.pair
    }

    /// Dispositivo.
    pub fn device(&self) -> &str {
        self.capture.device()
    }

    /// Nombre de la tarjeta.
    pub fn card_name(&self) -> &str {
        self.capture.card_name()
    }

    /// Última instantánea del análisis (sin locks).
    pub fn snapshot(&mut self) -> &LiveSnapshot {
        self.snapshot.read()
    }

    /// `true` si se está grabando.
    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// `true` si la captura ha terminado sola (fin del origen o error).
    pub fn capture_finished(&self) -> bool {
        self.capture.is_finished()
    }

    /// Estado en vivo.
    pub fn status(&self) -> RecordStatus {
        let c = self.capture.counters();
        let captured = c.frames.load(Ordering::Acquire);
        let lost = c.frames_lost.load(Ordering::Relaxed);
        let consumed = self.consumed.load(Ordering::Relaxed);
        let written = if self.active.is_some() {
            self.written.load(Ordering::Relaxed)
        } else {
            0
        };
        RecordStatus {
            frames_written: written,
            seconds: written as f64 / f64::from(self.config().rate),
            xruns: c.xruns.load(Ordering::Relaxed),
            frames_lost: lost,
            backlog_frames: captured.saturating_sub(lost).saturating_sub(consumed),
            realtime: c.realtime.load(Ordering::Relaxed),
            recording: self.active.is_some(),
        }
    }

    /// Empieza a grabar en `path` (no debe existir, ni su JSON).
    pub fn start_recording(&mut self, path: &Path, notes: &str) -> Result<(), AudioError> {
        if self.active.is_some() {
            return Err(AudioError::Invalid("ya se está grabando".into()));
        }
        let json = MasterInfo::path_for(path);
        if json.exists() {
            return Err(AudioError::Invalid(format!(
                "ya existe {}; no se sobrescribe",
                json.display()
            )));
        }
        let cfg = self.config();
        let pair0 = [self.pair[0] - 1, self.pair[1] - 1];
        let writer = MasterWriter::create(path, cfg.format, cfg.rate, cfg.channels, pair0)?;
        let (tx, rx) = mpsc::channel();
        self.send(WriterCmd::Start(Box::new(writer), tx))?;
        rx.recv()
            .map_err(|_| AudioError::ThreadPanicked("escritor"))?;
        self.active = Some(Active {
            path: path.to_path_buf(),
            json,
            notes: notes.to_string(),
            started: SystemTime::now(),
        });
        Ok(())
    }

    fn send(&self, cmd: WriterCmd) -> Result<(), AudioError> {
        self.cmd
            .as_ref()
            .and_then(|c| c.send(cmd).ok())
            .ok_or(AudioError::ThreadPanicked("escritor"))
    }

    /// Cierra el master en curso y escribe su JSON. Si la captura ya ha terminado, antes se
    /// escribe todo lo pendiente; si sigue en marcha, el master acaba en este instante.
    pub fn stop_recording(&mut self) -> Result<RecordingResult, AudioError> {
        let active = self
            .active
            .take()
            .ok_or_else(|| AudioError::Invalid("no se está grabando".into()))?;
        let (tx, rx) = mpsc::channel();
        self.send(WriterCmd::Stop(tx))?;
        let outcome = rx
            .recv()
            .map_err(|_| AudioError::ThreadPanicked("escritor"))??;
        let info = self.master_info(&active, outcome);
        info.write_new(&active.json)?;
        Ok(RecordingResult {
            master: active.path,
            json: active.json,
            info,
            capture_error: None,
        })
    }

    fn master_info(&self, active: &Active, outcome: WriterOutcome) -> MasterInfo {
        let cfg = self.config();
        let rate = f64::from(cfg.rate);
        let s = outcome.summary;
        MasterInfo {
            version: MASTER_INFO_VERSION,
            digityzer_version: crate::VERSION.to_string(),
            device: self.capture.device().to_string(),
            card_name: self.capture.card_name().to_string(),
            sample_rate: cfg.rate,
            sample_format: cfg.format.alsa_name().to_string(),
            container_bits: cfg.format.bits(),
            lsb_zero_bits: s.lsb_zero_bits,
            channels_opened: cfg.channels,
            channel_pair: self.pair,
            container: if s.rf64 { "RF64" } else { "WAV" }.to_string(),
            frames: s.frames,
            duration_s: s.frames as f64 / rate,
            data_bytes: s.data_bytes,
            started_at: utc_iso8601(active.started),
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
            notes: active.notes.clone(),
        }
    }

    /// Detiene la captura (sin cerrar la grabación en curso). Devuelve el error de la captura,
    /// si lo hubo.
    pub fn stop_capture(&mut self) -> Option<AudioError> {
        self.capture.stop().err()
    }

    /// Suelta el dispositivo. Si se estaba grabando, primero cierra el master.
    pub fn close(mut self) -> Result<Option<RecordingResult>, AudioError> {
        let capture_error = self.stop_capture();
        let result = if self.active.is_some() {
            let mut r = self.stop_recording()?;
            r.capture_error = capture_error;
            Some(r)
        } else {
            None
        };
        self.shutdown();
        Ok(result)
    }

    fn shutdown(&mut self) {
        let _ = self.capture.stop();
        self.cmd = None;
        self.analysis_stop.store(true, Ordering::Release);
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(a) = self.analysis.take() {
            let _ = a.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.active.is_some() {
            // Cierre de emergencia: que el master quede legible y con su JSON.
            let _ = self.capture.stop();
            if let Err(e) = self.stop_recording() {
                tracing::error!("no se pudo cerrar el master al soltar la sesión: {e}");
            }
        }
        self.shutdown();
    }
}

/// Grabación de punta a punta para la terminal.
#[derive(Debug)]
pub struct Recorder {
    session: Session,
}

impl Recorder {
    /// Abre el origen y empieza a grabar.
    pub fn start(
        source: Box<dyn CaptureSource>,
        request: RecordRequest,
    ) -> Result<Self, AudioError> {
        let json = MasterInfo::path_for(&request.path);
        if json.exists() {
            return Err(AudioError::Invalid(format!(
                "ya existe {}; no se sobrescribe",
                json.display()
            )));
        }
        if request.path.exists() {
            return Err(AudioError::io(
                &request.path,
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "el master ya existe"),
            ));
        }
        let mut session = Session::open(source, request.pair, request.engine)?;
        session.start_recording(&request.path, &request.notes)?;
        Ok(Self { session })
    }

    /// Configuración del flujo.
    pub fn config(&self) -> StreamConfig {
        self.session.config()
    }

    /// Estado en vivo.
    pub fn status(&self) -> RecordStatus {
        self.session.status()
    }

    /// `true` si la captura ha terminado sola (fin del origen o error).
    pub fn capture_finished(&self) -> bool {
        self.session.capture_finished()
    }

    /// Para la captura, escribe todo lo pendiente, cierra el master y escribe el JSON.
    pub fn stop(self) -> Result<RecordingResult, AudioError> {
        self.session
            .close()?
            .ok_or_else(|| AudioError::Invalid("no se estaba grabando".into()))
    }
}

fn drain_events(events: &mut rtrb::Consumer<CaptureEvent>, rec: Option<(u64, &mut Vec<u64>)>) {
    match rec {
        Some((start, xruns)) => {
            while let Ok(e) = events.pop() {
                if let CaptureEvent::Xrun { frame } = e
                    && frame >= start
                {
                    xruns.push(frame - start);
                }
            }
        }
        None => while events.pop().is_ok() {},
    }
}

struct Recording {
    writer: Box<MasterWriter>,
    start_frame: u64,
    skip: usize,
    xruns: Vec<u64>,
}

fn writer_loop(
    mut audio: rtrb::Consumer<u8>,
    mut events: rtrb::Consumer<CaptureEvent>,
    cmds: mpsc::Receiver<WriterCmd>,
    frame_bytes: u64,
    written: Arc<AtomicU64>,
    consumed_frames: Arc<AtomicU64>,
) {
    let mut rec: Option<Recording> = None;
    let mut consumed: u64 = 0;
    let mut failed: Option<AudioError> = None;
    let mut disconnected = false;
    loop {
        let finished = audio.is_abandoned() && audio.slots() == 0;
        // Órdenes. Con la captura terminada se espera a la siguiente orden; si no, se miran
        // sin bloquear.
        let cmd = if finished && !disconnected {
            cmds.recv().ok().or_else(|| {
                disconnected = true;
                None
            })
        } else {
            match cmds.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    None
                }
                Err(mpsc::TryRecvError::Empty) => None,
            }
        };
        match cmd {
            Some(WriterCmd::Start(writer, reply)) => {
                let skip = ((frame_bytes - consumed % frame_bytes) % frame_bytes) as usize;
                let start_frame = consumed.div_ceil(frame_bytes);
                // Eventos anteriores al inicio: fuera.
                drain_events(&mut events, Some((u64::MAX, &mut Vec::new())));
                rec = Some(Recording {
                    writer,
                    start_frame,
                    skip,
                    xruns: Vec::new(),
                });
                failed = None;
                written.store(0, Ordering::Relaxed);
                let _ = reply.send(());
            }
            Some(WriterCmd::Stop(reply)) => {
                let result = match (rec.take(), failed.take()) {
                    (_, Some(e)) => Err(e),
                    (None, None) => Err(AudioError::Invalid("no se estaba grabando".into())),
                    (Some(mut r), None) => {
                        // Si la captura ya terminó, primero todo lo pendiente.
                        let mut res = Ok(());
                        if audio.is_abandoned() {
                            while audio.slots() > 0 && res.is_ok() {
                                res = consume(&mut audio, Some(&mut r), &mut consumed);
                            }
                        }
                        drain_events(&mut events, Some((r.start_frame, &mut r.xruns)));
                        res.and_then(|()| r.writer.finalize())
                            .map(|summary| WriterOutcome {
                                summary,
                                xruns: r.xruns,
                            })
                    }
                };
                let _ = reply.send(result);
                continue;
            }
            None => {}
        }
        if finished {
            if disconnected {
                break;
            }
            continue;
        }
        if audio.slots() > 0 {
            if let Err(e) = consume(&mut audio, rec.as_mut(), &mut consumed) {
                // Error de E/S: se deja de escribir, se sigue vaciando, y se avisa al parar.
                tracing::error!("error al escribir el master: {e}");
                failed = Some(e);
                rec = None;
            }
            consumed_frames.store(consumed / frame_bytes, Ordering::Relaxed);
            match rec.as_mut() {
                Some(r) => {
                    drain_events(&mut events, Some((r.start_frame, &mut r.xruns)));
                    written.store(r.writer.frames(), Ordering::Relaxed);
                }
                None => drain_events(&mut events, None),
            }
        } else if disconnected && !audio.is_abandoned() {
            // La sesión se ha soltado sin parar la captura: no debería pasar.
            break;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn consume(
    audio: &mut rtrb::Consumer<u8>,
    rec: Option<&mut Recording>,
    consumed: &mut u64,
) -> Result<(), AudioError> {
    let n = audio.slots();
    let Ok(chunk) = audio.read_chunk(n) else {
        return Ok(());
    };
    let (a, b) = chunk.as_slices();
    let res = match rec {
        Some(r) => {
            let mut res = Ok(());
            for part in [a, b] {
                let skip = r.skip.min(part.len());
                r.skip -= skip;
                if res.is_ok() {
                    res = r.writer.write_interleaved(&part[skip..]);
                }
            }
            res
        }
        None => Ok(()),
    };
    *consumed += n as u64;
    chunk.commit_all();
    res
}

fn analysis_loop(
    mut audio: rtrb::Consumer<u8>,
    mut out: triple_buffer::Input<LiveSnapshot>,
    cfg: StreamConfig,
    pair: [usize; 2],
    stop: Arc<AtomicBool>,
) {
    let bits = match cfg.format {
        SampleFormat::S16Le => 16,
        SampleFormat::S24_3Le => 24,
        SampleFormat::S32Le => 32,
    };
    let mut analyzer = LiveAnalyzer::new(cfg.rate, bits);
    let fb = cfg.frame_bytes();
    let sb = cfg.format.bytes();
    let mut carry: Vec<u8> = Vec::with_capacity(fb);
    let mut lr: Vec<i32> = Vec::new();
    let mut last = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let n = audio.slots();
        if n == 0 {
            if audio.is_abandoned() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let Ok(chunk) = audio.read_chunk(n) else {
            continue;
        };
        let (a, b) = chunk.as_slices();
        lr.clear();
        for part in [a, b] {
            let mut part = part;
            if !carry.is_empty() {
                let take = (fb - carry.len()).min(part.len());
                carry.extend_from_slice(&part[..take]);
                part = &part[take..];
                if carry.len() == fb {
                    for &c in &pair {
                        lr.push(cfg.format.decode(&carry[c * sb..c * sb + sb]));
                    }
                    carry.clear();
                }
            }
            let whole = part.len() / fb * fb;
            for f in part[..whole].chunks_exact(fb) {
                for &c in &pair {
                    lr.push(cfg.format.decode(&f[c * sb..c * sb + sb]));
                }
            }
            carry.extend_from_slice(&part[whole..]);
        }
        chunk.commit_all();
        analyzer.push_interleaved(&lr);
        if last.elapsed() >= SNAPSHOT_INTERVAL {
            analyzer.snapshot_into(out.input_buffer_mut());
            out.publish();
            last = Instant::now();
        }
    }
    analyzer.snapshot_into(out.input_buffer_mut());
    out.publish();
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
