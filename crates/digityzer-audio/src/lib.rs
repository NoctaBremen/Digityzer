//! Captura ALSA bit-perfect y escritura del master (fase 1 del plan).
//!
//! - [`probe`]: tarjetas y lo que admite cada `hw:` (T1.1).
//! - [`alsa_source`] y [`engine`]: captura en `hw:` en exclusiva con un hilo `SCHED_FIFO` que
//!   solo empuja al ring buffer `rtrb`, y registro de xruns (T1.2).
//! - [`wav`] y [`recorder`]: master WAV/RF64 con el formato nativo, JSON y huella por bloques
//!   (T1.3), parada sin perder datos (T1.4).
//! - [`error::AudioError::Busy`]: dispositivo ocupado con un mensaje claro (T1.5).
//! - [`source::SyntheticSource`]: origen sintético para los tests y para probar sin la mesa.

pub mod alsa_source;
pub mod engine;
pub mod error;
pub mod format;
pub mod probe;
pub mod recorder;
pub mod source;
pub mod wav;

pub use alsa_source::{AlsaRequest, AlsaSource};
pub use engine::{Capture, CaptureEvent, EngineConfig};
pub use error::AudioError;
pub use format::SampleFormat;
pub use recorder::{RecordRequest, RecordStatus, Recorder, RecordingResult, verify_master};
pub use source::{CaptureSource, ReadOutcome, StreamConfig, SyntheticSource};

/// Versión de la crate (la del workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
