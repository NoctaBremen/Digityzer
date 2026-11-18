//! `audio-core`: código compartido con Spector, copiado de su repositorio (ver `PROVENANCE.md`).
//!
//! - [`dsp`] y [`spectrogram`]: ventanas, FFT por frames y espectrograma de toda una pista.
//! - [`palette`] y [`ticks`]: render RGBA del espectrograma y marcas de los ejes.
//! - [`analysis`]: detección de cortes en frecuencia (fuente con pérdida) y de falsos hi-res
//!   (sobremuestreo).
//! - [`aiff`] y [`tags`]: escritor y lector AIFF y etiquetas ID3 en el chunk `ID3 `.
//! - [`gtk`] (feature `gtk`): widget del espectrograma, exportación a PNG y estilo base.
//!
//! Spector no se modifica: los cambios se hacen aquí y se anotan en `PROVENANCE.md`.

pub mod aiff;
pub mod analysis;
pub mod cancel;
pub mod dsp;
pub mod error;
pub mod fsutil;
#[cfg(feature = "gtk")]
pub mod gtk;
pub mod palette;
pub mod pcm;
pub mod spectrogram;
pub mod tags;
pub mod ticks;

pub use analysis::{
    CutoffKind, CutoffResult, SourceEstimate, SpectrumStats, Thresholds, UpsamplingResult,
    analyze_spectrum, detect_cutoff, detect_upsampling,
};
pub use cancel::{CancelToken, NoProgress, Progress};
pub use dsp::{FrameAnalyzer, WindowKind};
pub use error::{CoreError, TagError};
pub use palette::{Palette, RgbaImage, palette_strip, render_rgba};
pub use pcm::{ChannelView, PcmBlock, PcmData, PcmFormat};
pub use spectrogram::{
    DbGrid, SpectrogramBuilder, SpectrogramData, SpectrogramParams, SpectrogramPreview,
    compute_spectrogram,
};
pub use ticks::{Tick, db_ticks, freq_ticks, time_ticks};

/// Commit de Spector del que se copió el código.
pub const SPECTOR_COMMIT: &str = "49724e80e279386a3d68ed85e4c9637527d53171";

/// Versión de la crate (la del workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
