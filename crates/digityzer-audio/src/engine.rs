//! Motor de captura (T1.2): un hilo que solo lee del origen y empuja los bytes al ring buffer
//! `rtrb`, sin bloqueos ni reservas de memoria dentro del bucle.
//!
//! - El hilo intenta `SCHED_FIFO`; si el sistema no lo permite (sin `rtprio`), sigue con la
//!   prioridad normal y lo anota.
//! - Cada xrun se cuenta y se manda por un segundo ring buffer de eventos con su posición.
//! - Si el ring buffer se llena (el escritor no da abasto), se descartan frames enteros y se
//!   cuentan: nunca se corta un frame a medias.
//! - Los mismos bytes van a un segundo ring buffer para el análisis en vivo. Si el análisis no da
//!   abasto, ahí se pierden fotogramas de los medidores, nunca muestras del master.
//! - Al terminar el hilo, los productores se sueltan: los consumidores lo ven con
//!   `is_abandoned()`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use crate::error::AudioError;
use crate::source::{CaptureSource, ReadOutcome, StreamConfig};

/// Capacidad del ring buffer de eventos.
pub const EVENT_CAPACITY: usize = 4096;
/// Segundos de audio del ring buffer del análisis.
pub const ANALYSIS_RING_SECONDS: f64 = 1.0;

/// Configuración del motor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineConfig {
    /// Segundos de audio que caben en el ring buffer.
    pub ring_seconds: f64,
    /// Prioridad `SCHED_FIFO` (1–99); `None` = no intentarlo.
    pub realtime_priority: Option<u8>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            ring_seconds: 10.0,
            realtime_priority: Some(70),
        }
    }
}

/// Evento del hilo de captura.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptureEvent {
    /// Xrun de ALSA tras `frame` frames capturados.
    Xrun {
        /// Frames capturados antes del xrun.
        frame: u64,
    },
    /// El ring buffer estaba lleno: se han descartado `frames` frames tras `frame`.
    RingOverflow {
        /// Frames capturados antes del descarte.
        frame: u64,
        /// Frames descartados.
        frames: u64,
    },
}

/// Contadores compartidos con el hilo de captura (lectura sin locks).
#[derive(Debug, Default)]
pub struct CaptureCounters {
    /// Frames leídos del origen (incluidos los descartados).
    pub frames: AtomicU64,
    /// Xruns.
    pub xruns: AtomicU64,
    /// Frames descartados por ring buffer lleno.
    pub frames_lost: AtomicU64,
    /// `true` si el hilo tiene `SCHED_FIFO`.
    pub realtime: AtomicBool,
    /// `true` cuando el hilo ha terminado.
    pub finished: AtomicBool,
}

/// Captura en marcha.
#[derive(Debug)]
pub struct Capture {
    stop: Arc<AtomicBool>,
    counters: Arc<CaptureCounters>,
    thread: Option<JoinHandle<Result<(), AudioError>>>,
    config: StreamConfig,
    device: String,
    card_name: String,
}

/// Extremos de lectura de una captura: audio entrelazado y eventos.
#[derive(Debug)]
pub struct CaptureOutputs {
    /// Bytes entrelazados de todos los canales.
    pub audio: rtrb::Consumer<u8>,
    /// Eventos (xruns, desbordamientos).
    pub events: rtrb::Consumer<CaptureEvent>,
    /// Los mismos bytes que `audio`, para el análisis en vivo (con pérdidas si no se lee).
    pub analysis: rtrb::Consumer<u8>,
}

impl Capture {
    /// Arranca el hilo de captura.
    pub fn start(
        mut source: Box<dyn CaptureSource>,
        cfg: EngineConfig,
    ) -> Result<(Self, CaptureOutputs), AudioError> {
        let config = source.config();
        let fb = config.frame_bytes();
        if fb == 0 || config.period_frames == 0 {
            return Err(AudioError::Invalid("configuración de flujo vacía".into()));
        }
        let ring_frames =
            ((cfg.ring_seconds * f64::from(config.rate)) as usize).max(config.period_frames * 4);
        let (mut producer, audio) = rtrb::RingBuffer::<u8>::new(ring_frames * fb);
        let (mut ev_producer, events) = rtrb::RingBuffer::new(EVENT_CAPACITY);
        let an_frames = ((ANALYSIS_RING_SECONDS * f64::from(config.rate)) as usize)
            .max(config.period_frames * 4);
        let (mut an_producer, analysis) = rtrb::RingBuffer::<u8>::new(an_frames * fb);
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CaptureCounters::default());
        let device = source.device().to_string();
        let card_name = source.card_name().to_string();

        let (stop_t, counters_t) = (Arc::clone(&stop), Arc::clone(&counters));
        let thread = std::thread::Builder::new()
            .name("digityzer-captura".into())
            .spawn(move || {
                let result = (|| {
                    if let Some(p) = cfg.realtime_priority {
                        counters_t
                            .realtime
                            .store(set_realtime(p), Ordering::Relaxed);
                    }
                    // Única reserva: antes del bucle.
                    let mut buf = vec![0u8; config.period_frames * fb];
                    source.start()?;
                    while !stop_t.load(Ordering::Acquire) {
                        match source.read(&mut buf)? {
                            ReadOutcome::Frames(n) => {
                                let at = counters_t.frames.load(Ordering::Relaxed);
                                let fit = (producer.slots() / fb).min(n);
                                if fit > 0 {
                                    push(&mut producer, &buf[..fit * fb]);
                                }
                                if fit < n {
                                    let lost = (n - fit) as u64;
                                    counters_t.frames_lost.fetch_add(lost, Ordering::Relaxed);
                                    let _ = ev_producer.push(CaptureEvent::RingOverflow {
                                        frame: at + fit as u64,
                                        frames: lost,
                                    });
                                }
                                let an_fit = (an_producer.slots() / fb).min(n);
                                if an_fit > 0 {
                                    push(&mut an_producer, &buf[..an_fit * fb]);
                                }
                                counters_t.frames.fetch_add(n as u64, Ordering::Release);
                            }
                            ReadOutcome::Xrun => {
                                counters_t.xruns.fetch_add(1, Ordering::Relaxed);
                                let frame = counters_t.frames.load(Ordering::Relaxed);
                                let _ = ev_producer.push(CaptureEvent::Xrun { frame });
                            }
                            ReadOutcome::End => break,
                        }
                    }
                    Ok(())
                })();
                counters_t.finished.store(true, Ordering::Release);
                result
            })
            .map_err(|e| AudioError::io("hilo de captura", e))?;
        Ok((
            Self {
                stop,
                counters,
                thread: Some(thread),
                config,
                device,
                card_name,
            },
            CaptureOutputs {
                audio,
                events,
                analysis,
            },
        ))
    }

    /// Configuración del flujo.
    pub fn config(&self) -> StreamConfig {
        self.config
    }

    /// Dispositivo.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Nombre de la tarjeta.
    pub fn card_name(&self) -> &str {
        &self.card_name
    }

    /// Contadores en vivo.
    pub fn counters(&self) -> &CaptureCounters {
        &self.counters
    }

    /// `true` si el hilo ha terminado por sí solo (fin del origen o error).
    pub fn is_finished(&self) -> bool {
        self.counters.finished.load(Ordering::Acquire)
    }

    /// Pide parar y espera al hilo. Devuelve el error del hilo, si lo hubo.
    pub fn stop(&mut self) -> Result<(), AudioError> {
        self.stop.store(true, Ordering::Release);
        match self.thread.take() {
            Some(t) => t
                .join()
                .map_err(|_| AudioError::ThreadPanicked("captura"))?,
            None => Ok(()),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Copia `bytes` al ring buffer; quien llama ya ha comprobado que caben.
fn push(producer: &mut rtrb::Producer<u8>, bytes: &[u8]) {
    if let Ok(mut chunk) = producer.write_chunk(bytes.len()) {
        let (a, b) = chunk.as_mut_slices();
        a.copy_from_slice(&bytes[..a.len()]);
        b.copy_from_slice(&bytes[a.len()..]);
        chunk.commit_all();
    }
}

/// Pone el hilo actual en `SCHED_FIFO` con prioridad `p`. `false` si el sistema no lo permite.
fn set_realtime(p: u8) -> bool {
    use thread_priority::unix::{
        RealtimeThreadSchedulePolicy, ThreadSchedulePolicy, set_thread_priority_and_policy,
        thread_native_id,
    };
    let Ok(value) = p.min(99).try_into() else {
        return false;
    };
    set_thread_priority_and_policy(
        thread_native_id(),
        thread_priority::ThreadPriority::Crossplatform(value),
        ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::SampleFormat;
    use crate::source::SyntheticSource;
    use crate::source::tests::{config, drain};

    fn collect(mut out: CaptureOutputs) -> (Vec<u8>, Vec<CaptureEvent>) {
        let mut bytes = Vec::new();
        loop {
            let n = out.audio.slots();
            if n > 0 {
                let chunk = out.audio.read_chunk(n).unwrap();
                let (a, b) = chunk.as_slices();
                bytes.extend_from_slice(a);
                bytes.extend_from_slice(b);
                chunk.commit_all();
            } else if out.audio.is_abandoned() {
                break;
            } else {
                std::thread::yield_now();
            }
        }
        let mut events = Vec::new();
        while let Ok(e) = out.events.pop() {
            events.push(e);
        }
        (bytes, events)
    }

    #[test]
    fn todos_los_bytes_en_orden_y_xruns_registrados() {
        let c = config(SampleFormat::S32Le, 12);
        let src = SyntheticSource::new(c)
            .with_total_frames(96_000)
            .with_xruns_at(vec![10_000, 50_000]);
        let expected = drain(src.clone());
        let (mut cap, out) = Capture::start(Box::new(src), EngineConfig::default()).unwrap();
        let (bytes, events) = collect(out);
        cap.stop().unwrap();
        assert_eq!(bytes, expected);
        assert_eq!(
            events,
            vec![
                CaptureEvent::Xrun { frame: 10_000 },
                CaptureEvent::Xrun { frame: 50_000 }
            ]
        );
        assert_eq!(cap.counters().xruns.load(Ordering::Relaxed), 2);
        assert_eq!(cap.counters().frames.load(Ordering::Relaxed), 96_000);
        assert!(cap.is_finished());
    }

    #[test]
    fn ring_lleno_descarta_frames_enteros_y_lo_cuenta() {
        let c = config(SampleFormat::S24_3Le, 2);
        let src = SyntheticSource::new(c).with_total_frames(20_000);
        let cfg = EngineConfig {
            ring_seconds: 0.0, // mínimo: 4 períodos
            realtime_priority: None,
        };
        let (mut cap, out) = Capture::start(Box::new(src), cfg).unwrap();
        // Nadie lee hasta que el hilo termina: casi todo se descarta.
        while !cap.is_finished() {
            std::thread::yield_now();
        }
        let (bytes, events) = collect(out);
        cap.stop().unwrap();
        let lost = cap.counters().frames_lost.load(Ordering::Relaxed);
        assert_eq!(bytes.len() % 6, 0);
        assert_eq!(bytes.len() as u64 / 6 + lost, 20_000);
        assert_eq!(bytes.len(), 4 * 480 * 6);
        assert!(matches!(
            events[0],
            CaptureEvent::RingOverflow { frame: 1920, .. }
        ));
    }

    #[test]
    fn parar_un_origen_infinito() {
        let c = config(SampleFormat::S16Le, 2);
        let (mut cap, _out) = Capture::start(
            Box::new(SyntheticSource::new(c).realtime()),
            EngineConfig::default(),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        cap.stop().unwrap();
        assert!(cap.is_finished());
    }
}
