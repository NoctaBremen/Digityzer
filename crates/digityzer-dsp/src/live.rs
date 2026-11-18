//! Todo lo que se mide en vivo sobre el par grabado, y la instantánea que se pasa a la interfaz
//! por triple buffer (sin locks).

use crate::meters::{ChannelLevel, LevelMeter, MeterConfig};
use crate::spectrum::LiveSpectrum;
use crate::stereo::{Correlation, Goniometer};
use crate::waveform::Waveform;

/// Bandas del espectro de la instantánea.
pub const SPECTRUM_BANDS: usize = 192;
/// Frecuencia mínima del espectro (Hz).
pub const SPECTRUM_F_MIN: f64 = 20.0;
/// Puntos del goniómetro.
pub const GONIO_POINTS: usize = 2048;
/// Segundos por columna de la forma de onda.
pub const WAVEFORM_COLUMN_S: f64 = 0.05;
/// Columnas de la forma de onda (60 s a 0,05 s).
pub const WAVEFORM_COLUMNS: usize = 1200;

/// Instantánea para la interfaz. Los `Vec` se reutilizan entre publicaciones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveSnapshot {
    /// Frecuencia de muestreo.
    pub rate: u32,
    /// Niveles L y R.
    pub levels: [ChannelLevel; 2],
    /// Correlación de fase; `None` en silencio.
    pub correlation: Option<f32>,
    /// Goniómetro `(lado, medio)`.
    pub gonio: Vec<(f32, f32)>,
    /// Espectro en dBFS por banda logarítmica de [`SPECTRUM_F_MIN`] a Nyquist.
    pub spectrum: Vec<f32>,
    /// Forma de onda `(mín, máx)` por columna, la más reciente al final.
    pub waveform: Vec<(f32, f32)>,
    /// Frames analizados desde el principio.
    pub frames: u64,
}

/// Analizador en vivo de un par de canales.
#[derive(Debug)]
pub struct LiveAnalyzer {
    rate: u32,
    scale: f64,
    meters: [LevelMeter; 2],
    corr: Correlation,
    gonio: Goniometer,
    spectrum: LiveSpectrum,
    waveform: Waveform,
    frames: u64,
    since_snapshot: u64,
}

impl LiveAnalyzer {
    /// Analizador para muestras de `bits` bits a `rate` Hz.
    pub fn new(rate: u32, bits: u32) -> Self {
        let cfg = MeterConfig::default();
        Self {
            rate,
            scale: (1i64 << (bits.clamp(1, 32) - 1)) as f64,
            meters: [
                LevelMeter::new(rate, bits, cfg),
                LevelMeter::new(rate, bits, cfg),
            ],
            corr: Correlation::new((0.3 * f64::from(rate)) as usize),
            gonio: Goniometer::new(GONIO_POINTS, (rate / 24_000).max(1) as usize),
            spectrum: LiveSpectrum::new(rate, bits, 0.3),
            waveform: Waveform::new(
                (WAVEFORM_COLUMN_S * f64::from(rate)) as usize,
                WAVEFORM_COLUMNS,
            ),
            frames: 0,
            since_snapshot: 0,
        }
    }

    /// Procesa muestras entrelazadas L/R justificadas a la derecha.
    pub fn push_interleaved(&mut self, lr: &[i32]) {
        for f in lr.chunks_exact(2) {
            self.meters[0].push(f[0]);
            self.meters[1].push(f[1]);
            let (l, r) = (f64::from(f[0]) / self.scale, f64::from(f[1]) / self.scale);
            self.corr.push(l, r);
            self.gonio.push(l, r);
            self.waveform.push(((l + r) / 2.0) as f32);
        }
        self.spectrum.push_interleaved(lr);
        let n = (lr.len() / 2) as u64;
        self.frames += n;
        self.since_snapshot += n;
    }

    /// Rellena `out` con el estado actual (reutilizando sus `Vec`) y reinicia los picos.
    pub fn snapshot_into(&mut self, out: &mut LiveSnapshot) {
        out.rate = self.rate;
        out.levels = [
            self.meters[0].take(self.since_snapshot),
            self.meters[1].take(self.since_snapshot),
        ];
        self.since_snapshot = 0;
        out.correlation = self.corr.value();
        out.gonio.clear();
        out.gonio.extend(self.gonio.points());
        let nyquist = f64::from(self.rate) / 2.0;
        self.spectrum
            .log_bands(SPECTRUM_BANDS, SPECTRUM_F_MIN, nyquist, &mut out.spectrum);
        out.waveform.clear();
        out.waveform.extend(self.waveform.columns());
        out.frames = self.frames;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_testkit::{interleave, quantize, sine};

    /// T2.3: el análisis en vivo debe ir muy por delante del tiempo real para no robar CPU a la
    /// captura. `cargo test --release -p digityzer-dsp -- --ignored --nocapture`
    #[test]
    #[ignore = "rendimiento: solo en release"]
    fn rendimiento_frente_a_tiempo_real() {
        let rate = 96_000;
        let s = quantize(&sine(1000.0, rate, 10.0, 0.5), 32);
        let lr = interleave(&[s.clone(), s]);
        let mut a = LiveAnalyzer::new(rate, 32);
        let mut snap = LiveSnapshot::default();
        let t = std::time::Instant::now();
        for (i, chunk) in lr.chunks(2 * 1024).enumerate() {
            a.push_interleaved(chunk);
            if i % 2 == 0 {
                a.snapshot_into(&mut snap);
            }
        }
        let secs = t.elapsed().as_secs_f64();
        println!(
            "10 s de audio a 96 kHz analizados en {secs:.3} s ({:.0}× tiempo real)",
            10.0 / secs
        );
        assert!(secs < 1.0, "{secs}");
    }

    #[test]
    fn instantanea_completa() {
        let s = quantize(&sine(1000.0, 48_000, 2.0, 0.5), 24);
        let inv: Vec<i32> = s.iter().map(|v| -v).collect();
        let mut a = LiveAnalyzer::new(48_000, 24);
        a.push_interleaved(&interleave(&[s.clone(), inv]));
        let mut snap = LiveSnapshot::default();
        a.snapshot_into(&mut snap);
        assert_eq!(snap.frames, 96_000);
        assert!((snap.levels[0].peak_dbfs + 6.02).abs() < 0.1);
        assert!((snap.correlation.unwrap() + 1.0).abs() < 1e-4);
        assert_eq!(snap.gonio.len(), GONIO_POINTS);
        assert_eq!(snap.spectrum.len(), SPECTRUM_BANDS);
        assert_eq!(snap.waveform.len(), 40);
        // Mezcla de un par invertido: la forma de onda es plana.
        assert!(
            snap.waveform
                .iter()
                .all(|&(lo, hi)| lo.abs() < 1e-6 && hi.abs() < 1e-6)
        );
        // El pico del intervalo se reinicia; la retención se mantiene.
        a.push_interleaved(&[0, 0]);
        a.snapshot_into(&mut snap);
        assert!(snap.levels[0].peak_dbfs < -100.0);
        assert!((snap.levels[0].peak_hold_dbfs + 6.02).abs() < 0.1);
    }
}
