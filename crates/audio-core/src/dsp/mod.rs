//! Procesado de señal: ventanas y análisis espectral por frames.

pub mod frame;
pub mod window;

pub use frame::FrameAnalyzer;
pub use window::{WindowKind, window};
