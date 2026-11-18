//! Detección de cortes en frecuencia y de falsos hi-res, copiada de `spector-core::analysis`.
//!
//! [`analyze_spectrum`] es la parte espectral del pase único de Spector (`analyze_raw`): una FFT
//! **fija** (8.192, Blackman-Harris, salto de 4.096, vista Mezcla), independiente de la FFT de
//! pantalla para que los resultados sean reproducibles. Sus [`SpectrumStats`] alimentan a
//! [`detect_cutoff`] y este a [`detect_upsampling`].

pub mod cutoff;
pub mod stats;
pub mod thresholds;
pub mod upsampling;

pub use cutoff::{CutoffKind, CutoffResult, SourceEstimate, detect_cutoff};
pub use stats::{FloatStats, LtasStats, RunMap, RunStats, SampleStats, SpectrumStats};
pub use thresholds::{BitrateStep, LossyExpectation, Thresholds};
pub use upsampling::{UpsamplingResult, detect_upsampling};

use crate::cancel::CancelToken;
use crate::dsp::{FrameAnalyzer, WindowKind, window};
use crate::error::CoreError;
use crate::pcm::{ChannelView, PcmBlock, PcmFormat};

/// Parámetros del pase espectral. Los valores por defecto son los de Spector; cambiarlos altera
/// los resultados, así que solo se tocan en los tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisParams {
    /// Tamaño de la FFT de análisis.
    pub fft_size: usize,
    /// Ventana de análisis.
    pub window: WindowKind,
    /// Un frame es "activo" si su nivel RMS es ≥ este valor (dBFS). Los silencios no cuentan.
    pub activity_threshold_db: f32,
}

impl Default for AnalysisParams {
    fn default() -> Self {
        Self {
            fft_size: 8192,
            window: WindowKind::BlackmanHarris,
            activity_threshold_db: -60.0,
        }
    }
}

/// Ancho de banda de ruido equivalente de una ventana, en bins: `N · Σw² / (Σw)²`.
pub fn enbw_bins(kind: WindowKind, n: usize) -> f64 {
    let w = window(kind, n);
    let s1: f64 = w.iter().map(|&x| f64::from(x)).sum();
    let s2: f64 = w.iter().map(|&x| f64::from(x).powi(2)).sum();
    if s1 == 0.0 {
        1.0
    } else {
        n as f64 * s2 / (s1 * s1)
    }
}

/// Nivel RMS (dBFS, con seno a escala completa = −3,01 dBFS) de un frame a partir de sus potencias
/// normalizadas (ver `dsp::frame`), por Parseval: `RMS² = Σ p / (2 · ENBW)`.
pub fn frame_rms_db(power: &[f32], enbw: f64) -> f32 {
    let total: f64 = power.iter().map(|&p| f64::from(p)).sum();
    (10.0 * (total / (2.0 * enbw)).max(1e-30).log10()) as f32
}

/// Estadísticas espectrales de la mezcla de canales de una secuencia de bloques.
///
/// Adaptado de `spector-core::analyze_raw`: solo la parte espectral y con bloques en lugar de
/// un archivo. Comprueba `cancel` en cada bloque.
pub fn analyze_spectrum<I>(
    blocks: I,
    format: PcmFormat,
    params: &AnalysisParams,
    cancel: &CancelToken,
) -> Result<SpectrumStats, CoreError>
where
    I: IntoIterator<Item = PcmBlock>,
{
    let mut mix = FrameAnalyzer::new(
        params.fft_size,
        params.window,
        ChannelView::Mix,
        format.channels,
    )?;
    let enbw = enbw_bins(params.window, params.fft_size);
    let threshold = params.activity_threshold_db;
    let mut spectrum = SpectrumStats::new(format.sample_rate, params.fft_size).with_enbw(enbw);
    let mut on_frame = |p: &[f32]| {
        let active = frame_rms_db(p, enbw) >= threshold;
        spectrum.add_frame(p, active);
    };
    for block in blocks {
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        mix.push(&block, &mut on_frame)?;
    }
    mix.finish(&mut on_frame);
    Ok(spectrum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcm::PcmData;

    #[test]
    fn enbw_conocidos() {
        // Hann ≈ 1,5 bins; Blackman-Harris de 4 términos ≈ 2,0 bins.
        assert!((enbw_bins(WindowKind::Hann, 8192) - 1.5).abs() < 1e-3);
        assert!((enbw_bins(WindowKind::BlackmanHarris, 8192) - 2.004).abs() < 1e-2);
    }

    #[test]
    fn rms_de_un_seno_por_parseval() {
        // Seno de amplitud 1 → RMS = 1/√2 → −3,01 dBFS.
        let n = 8192;
        let s: Vec<f32> = (0..n * 2)
            .map(|i| (std::f64::consts::TAU * 1500.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let block = PcmBlock {
            channels: 1,
            frames: s.len(),
            data: PcmData::F32(s),
        };
        let mut a = FrameAnalyzer::new(n, WindowKind::BlackmanHarris, ChannelView::Mix, 1).unwrap();
        let enbw = enbw_bins(WindowKind::BlackmanHarris, n);
        let mut db = 0.0;
        a.push(&block, |p| db = frame_rms_db(p, enbw)).unwrap();
        assert!((db + 3.01).abs() < 0.1, "{db}");
    }
}
