//! Espectro en vivo (T2.3) con el análisis por frames de `audio-core` (el mismo que usa Spector):
//! FFT de 4.096 con Hann sobre la mezcla del par, promediado exponencial en potencia y bandas
//! logarítmicas para la pantalla.

use audio_core::dsp::frame::power_to_db;
use audio_core::{ChannelView, FrameAnalyzer, PcmBlock, PcmData, WindowKind};

/// Tamaño de la FFT del espectro en vivo.
pub const LIVE_FFT: usize = 4096;

/// Espectro promediado de la mezcla de dos canales.
pub struct LiveSpectrum {
    analyzer: FrameAnalyzer,
    rate: u32,
    bits: u32,
    avg: Vec<f32>,
    alpha: f32,
    frames: u64,
    block: Vec<i32>,
}

impl std::fmt::Debug for LiveSpectrum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSpectrum")
            .field("rate", &self.rate)
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl LiveSpectrum {
    /// Espectro para muestras de `bits` bits a `rate` Hz. `alpha` es el peso de cada frame nuevo
    /// en el promedio (1 = sin promediar).
    pub fn new(rate: u32, bits: u32, alpha: f32) -> Self {
        #[allow(clippy::expect_used)]
        // Invariante: 4.096 es un tamaño admitido, la vista Mix y 2 canales son válidos.
        let analyzer = FrameAnalyzer::new(LIVE_FFT, WindowKind::Hann, ChannelView::Mix, 2)
            .expect("parámetros fijos válidos");
        let bins = analyzer.bins();
        Self {
            analyzer,
            rate,
            bits,
            avg: vec![0.0; bins],
            alpha: alpha.clamp(0.01, 1.0),
            frames: 0,
            block: Vec::new(),
        }
    }

    /// Añade muestras entrelazadas L/R justificadas a la derecha.
    pub fn push_interleaved(&mut self, lr: &[i32]) {
        self.block.clear();
        self.block.extend_from_slice(lr);
        let block = PcmBlock {
            channels: 2,
            frames: lr.len() / 2,
            data: PcmData::Int {
                bits: self.bits,
                samples: std::mem::take(&mut self.block),
            },
        };
        let (avg, alpha, frames) = (&mut self.avg, self.alpha, &mut self.frames);
        // Con la vista Mix y dos canales, `push` no puede fallar.
        let _ = self.analyzer.push(&block, |p| {
            let a = if *frames == 0 { 1.0 } else { alpha };
            for (s, &v) in avg.iter_mut().zip(p) {
                *s += a * (v - *s);
            }
            *frames += 1;
        });
        if let PcmData::Int { samples, .. } = block.data {
            self.block = samples;
        }
    }

    /// Hz por bin.
    pub fn hz_per_bin(&self) -> f64 {
        f64::from(self.rate) / LIVE_FFT as f64
    }

    /// Espectro promediado en dBFS por bin (seno a escala completa = 0 dBFS).
    pub fn bins_db(&self) -> impl Iterator<Item = f32> + '_ {
        self.avg.iter().map(|&p| power_to_db(p))
    }

    /// Frames de FFT procesados.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// `n` bandas logarítmicas entre `f_min` y `f_max` Hz con el **máximo** de cada banda (para
    /// que un tono no se diluya). Las bandas sin ningún bin toman el bin más cercano.
    pub fn log_bands(&self, n: usize, f_min: f64, f_max: f64, out: &mut Vec<f32>) {
        let db: Vec<f32> = self.bins_db().collect();
        log_bands(&db, self.hz_per_bin(), n, f_min, f_max, out);
    }
}

/// Ver [`LiveSpectrum::log_bands`].
pub fn log_bands(
    db: &[f32],
    hz_per_bin: f64,
    n: usize,
    f_min: f64,
    f_max: f64,
    out: &mut Vec<f32>,
) {
    out.clear();
    if db.is_empty() || n == 0 || f_min <= 0.0 || f_max <= f_min {
        return;
    }
    let ratio = (f_max / f_min).ln();
    let last = db.len() - 1;
    for i in 0..n {
        let lo = f_min * (ratio * i as f64 / n as f64).exp();
        let hi = f_min * (ratio * (i + 1) as f64 / n as f64).exp();
        let a = ((lo / hz_per_bin).ceil() as usize).min(last);
        let b = ((hi / hz_per_bin).floor() as usize).min(last);
        let v = if b >= a {
            db[a..=b].iter().copied().fold(f32::MIN, f32::max)
        } else {
            db[(((lo * hi).sqrt() / hz_per_bin).round() as usize).min(last)]
        };
        out.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_testkit::{interleave, quantize, sine};

    #[test]
    fn tono_en_su_banda_y_a_su_nivel() {
        let rate = 96_000;
        // 3.000 Hz exactos en un bin (96.000 / 4.096 = 23,4375 Hz; 128 bins).
        let f = 128.0 * 23.4375;
        let s = quantize(&sine(f, rate, 1.0, 0.5), 24);
        let lr = interleave(&[s.clone(), s]);
        let mut sp = LiveSpectrum::new(rate, 24, 0.2);
        for chunk in lr.chunks(2048) {
            sp.push_interleaved(chunk);
        }
        assert!(sp.frames() > 30);
        let db: Vec<f32> = sp.bins_db().collect();
        let peak = db
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
        assert_eq!(peak.0, 128);
        // Amplitud 0,5 → −6,02 dBFS.
        assert!((peak.1 + 6.02).abs() < 0.1, "{}", peak.1);

        let mut bands = Vec::new();
        sp.log_bands(64, 20.0, 40_000.0, &mut bands);
        assert_eq!(bands.len(), 64);
        let best = bands
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
        let lo = 20.0 * ((40_000.0f64 / 20.0).ln() * best.0 as f64 / 64.0).exp();
        let hi = 20.0 * ((40_000.0f64 / 20.0).ln() * (best.0 + 1) as f64 / 64.0).exp();
        assert!(lo <= f && f <= hi, "{lo}–{hi}");
    }

    #[test]
    fn bandas_sin_bins_usan_el_mas_cercano() {
        let db = vec![-10.0, -20.0, -30.0, -40.0];
        let mut out = Vec::new();
        log_bands(&db, 100.0, 4, 10.0, 300.0, &mut out);
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|v| db.contains(v)));
    }
}
