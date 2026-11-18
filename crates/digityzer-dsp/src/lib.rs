//! Medidores y medidas de la señal (fase 2 del plan; la calibración y el diagnóstico de la
//! cadena, en la fase 3).
//!
//! - [`meters`]: pico y RMS en dBFS, retención de pico y clipping (T2.2).
//! - [`spectrum`]: espectro en vivo sobre el análisis de `audio-core`: capas, suavizado por
//!   fracción de octava, barras sin escalones, balística y notas (T2.3, T2.6, T2.7).
//! - [`stereo`]: correlación de fase y goniómetro (T2.4).
//! - [`waveform`]: forma de onda que se desplaza (T2.5).
//! - [`live`]: todo junto y la instantánea para la interfaz.
//!
//! Funciones puras sobre muestras: no dependen de ALSA ni de GTK.

pub mod live;
pub mod meters;
pub mod sliding;
pub mod spectrum;
pub mod stereo;
pub mod waveform;

pub use live::{LiveAnalyzer, LiveSnapshot};
pub use meters::{ChannelLevel, LevelMeter, MeterConfig};

/// Versión de la crate (la del workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
