//! Espectro en vivo (T2.3, T2.6, T2.7) con el análisis por frames de `audio-core` (el mismo que
//! usa Spector).
//!
//! - FFT larga según la frecuencia ([`live_fft_size`]: 16.384 puntos a 96 kHz, ≈ 5,9 Hz por
//!   bin) para que los graves tengan resolución.
//! - Tres capas por bin, en potencia lineal: instantánea (casi sin promediar), media (≈ 1 s) y
//!   retención de picos con caída lenta ([`PEAK_DECAY_DB_PER_S`]).
//! - Para pantalla, curvas en una rejilla logarítmica: [`smooth_curve`] (fracción de octava sobre
//!   la potencia interpolada linealmente entre bins, sin escalones) y [`log_bands`] (barras).
//!
//! Escala: un seno de amplitud `A` centrado en un bin da potencia `A²` (0 dBFS a escala
//! completa), la convención de `audio-core`.

use audio_core::dsp::frame::power_to_db;
use audio_core::{ChannelView, FrameAnalyzer, PcmBlock, PcmData, WindowKind};

/// Caída de la retención de picos (dB por segundo).
pub const PEAK_DECAY_DB_PER_S: f64 = 6.0;
/// Peso de cada frame nuevo en la capa instantánea.
pub const ALPHA_INSTANT: f32 = 0.6;
/// Constante de tiempo de la capa media (s).
pub const AVERAGE_TAU_S: f64 = 1.5;
/// Constante de tiempo de la balística de pantalla (s): la curva se desliza hacia cada valor
/// nuevo en lugar de saltar cuando llega un frame de FFT.
pub const DISPLAY_TAU_S: f64 = 0.25;

/// Tamaño de FFT para `rate` Hz: la potencia de 2 que deja ≤ 6 Hz por bin (entre 4.096 y 32.768).
pub fn live_fft_size(rate: u32) -> usize {
    ((f64::from(rate) / 6.0).ceil().max(1.0) as usize)
        .next_power_of_two()
        .clamp(4096, 32_768)
}

/// Las tres capas del espectro, en potencia lineal por bin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpectrumLayers {
    /// Hz por bin.
    pub hz_per_bin: f64,
    /// Capa instantánea.
    pub instant: Vec<f32>,
    /// Capa media.
    pub average: Vec<f32>,
    /// Retención de picos.
    pub peak: Vec<f32>,
    /// Frames de FFT procesados (cambia cuando hay espectro nuevo).
    pub frames: u64,
}

/// Espectro en vivo de la mezcla de dos canales.
pub struct LiveSpectrum {
    analyzer: FrameAnalyzer,
    rate: u32,
    bits: u32,
    fft: usize,
    layers: SpectrumLayers,
    alpha_avg: f32,
    peak_decay: f32,
    block: Vec<i32>,
}

impl std::fmt::Debug for LiveSpectrum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSpectrum")
            .field("rate", &self.rate)
            .field("fft", &self.fft)
            .field("frames", &self.layers.frames)
            .finish_non_exhaustive()
    }
}

impl LiveSpectrum {
    /// Espectro para muestras de `bits` bits a `rate` Hz.
    pub fn new(rate: u32, bits: u32) -> Self {
        let fft = live_fft_size(rate);
        #[allow(clippy::expect_used)]
        // Invariante: `live_fft_size` da un tamaño admitido; Mix con 2 canales es válido.
        let analyzer = FrameAnalyzer::new(fft, WindowKind::Hann, ChannelView::Mix, 2)
            .expect("parámetros fijos válidos");
        let bins = analyzer.bins();
        let dt = analyzer.hop() as f64 / f64::from(rate.max(1));
        Self {
            analyzer,
            rate,
            bits,
            fft,
            layers: SpectrumLayers {
                hz_per_bin: f64::from(rate) / fft as f64,
                instant: vec![0.0; bins],
                average: vec![0.0; bins],
                peak: vec![0.0; bins],
                frames: 0,
            },
            alpha_avg: (1.0 - (-dt / AVERAGE_TAU_S).exp()) as f32,
            peak_decay: 10f64.powf(-PEAK_DECAY_DB_PER_S * dt / 10.0) as f32,
            block: Vec::new(),
        }
    }

    /// Tamaño de la FFT.
    pub fn fft_size(&self) -> usize {
        self.fft
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
        let (l, a_avg, decay) = (&mut self.layers, self.alpha_avg, self.peak_decay);
        // Con la vista Mix y dos canales, `push` no puede fallar.
        let _ = self.analyzer.push(&block, |p| {
            let first = l.frames == 0;
            for (i, &v) in p.iter().enumerate() {
                if first {
                    l.instant[i] = v;
                    l.average[i] = v;
                    l.peak[i] = v;
                } else {
                    l.instant[i] += ALPHA_INSTANT * (v - l.instant[i]);
                    l.average[i] += a_avg * (v - l.average[i]);
                    l.peak[i] = (l.peak[i] * decay).max(v);
                }
            }
            l.frames += 1;
        });
        if let PcmData::Int { samples, .. } = block.data {
            self.block = samples;
        }
    }

    /// Capas actuales.
    pub fn layers(&self) -> &SpectrumLayers {
        &self.layers
    }

    /// Hz por bin.
    pub fn hz_per_bin(&self) -> f64 {
        self.layers.hz_per_bin
    }

    /// Frames de FFT procesados.
    pub fn frames(&self) -> u64 {
        self.layers.frames
    }
}

/// Rejilla de `n` frecuencias espaciadas logarítmicamente de `f_min` a `f_max` (ambas incluidas).
pub fn log_grid(n: usize, f_min: f64, f_max: f64) -> Vec<f64> {
    if n < 2 || f_min <= 0.0 || f_max <= f_min {
        return vec![f_min; n.min(1)];
    }
    let r = (f_max / f_min).ln();
    (0..n)
        .map(|i| f_min * (r * i as f64 / (n - 1) as f64).exp())
        .collect()
}

/// Potencia en `hz`, interpolada linealmente entre los dos bins vecinos.
pub fn interp_power(power: &[f32], hz_per_bin: f64, hz: f64) -> f32 {
    if power.is_empty() || hz_per_bin <= 0.0 {
        return 0.0;
    }
    let x = (hz / hz_per_bin).clamp(0.0, (power.len() - 1) as f64);
    let k = x.floor() as usize;
    let t = (x - k as f64) as f32;
    let next = power[(k + 1).min(power.len() - 1)];
    power[k] + t * (next - power[k])
}

/// Integral de la potencia interpolada linealmente desde el bin 0 hasta la posición `x` (en bins).
/// `cum[k]` = integral hasta el bin `k`.
fn integral(power: &[f32], cum: &[f64], x: f64) -> f64 {
    let last = (power.len() - 1) as f64;
    let x = x.clamp(0.0, last);
    let k = (x.floor() as usize).min(power.len() - 1);
    let t = x - k as f64;
    let p0 = f64::from(power[k]);
    let p1 = f64::from(power[(k + 1).min(power.len() - 1)]);
    cum[k] + p0 * t + (p1 - p0) * t * t / 2.0
}

/// Curva suavizada por fracción de octava (`fraction` = 3, 6 o 12 para 1/3, 1/6 o 1/12) en las
/// frecuencias de `grid`, en dBFS. Cada punto es la media de la potencia (interpolada
/// linealmente entre bins) en una ventana de `1/fraction` octava centrada en la frecuencia; si la
/// ventana es más estrecha que un bin, queda la interpolación en el centro. Así la curva es
/// continua también en los graves.
pub fn smooth_curve(
    power: &[f32],
    hz_per_bin: f64,
    grid: &[f64],
    fraction: f64,
    out: &mut Vec<f32>,
) {
    out.clear();
    if power.len() < 2 || hz_per_bin <= 0.0 {
        out.resize(grid.len(), power_to_db(0.0));
        return;
    }
    let mut cum = Vec::with_capacity(power.len());
    let mut acc = 0.0f64;
    cum.push(0.0);
    for w in power.windows(2) {
        acc += (f64::from(w[0]) + f64::from(w[1])) / 2.0;
        cum.push(acc);
    }
    let half = 2f64.powf(1.0 / (2.0 * fraction.max(1.0)));
    for &f in grid {
        let a = f / half / hz_per_bin;
        let b = f * half / hz_per_bin;
        let p = if b - a < 1e-3 {
            f64::from(interp_power(power, hz_per_bin, f))
        } else {
            (integral(power, &cum, b) - integral(power, &cum, a)) / (b - a)
        };
        out.push(power_to_db(p as f32));
    }
}

/// `n` barras logarítmicas entre `f_min` y `f_max` Hz, en dBFS, con el **máximo** de los bins de
/// cada banda (para que un tono no se diluya). Una banda más estrecha que un bin toma la potencia
/// interpolada en su centro geométrico, así que en los graves no hay bloques planos (T2.6).
pub fn log_bands(
    power: &[f32],
    hz_per_bin: f64,
    n: usize,
    f_min: f64,
    f_max: f64,
    out: &mut Vec<f32>,
) {
    out.clear();
    if power.is_empty() || n == 0 || f_min <= 0.0 || f_max <= f_min || hz_per_bin <= 0.0 {
        return;
    }
    let ratio = (f_max / f_min).ln();
    let last = power.len() - 1;
    for i in 0..n {
        let lo = f_min * (ratio * i as f64 / n as f64).exp();
        let hi = f_min * (ratio * (i + 1) as f64 / n as f64).exp();
        let a = ((lo / hz_per_bin).ceil() as usize).min(last);
        let b = ((hi / hz_per_bin).floor() as usize).min(last);
        let center = interp_power(power, hz_per_bin, (lo * hi).sqrt());
        let p = if b >= a {
            power[a..=b].iter().copied().fold(center, f32::max)
        } else {
            center
        };
        out.push(power_to_db(p));
    }
}

/// Balística de pantalla de una curva en dB: cada punto se acerca a su objetivo con constante de
/// tiempo `tau_s`. Se llama una vez por fotograma de la interfaz.
#[derive(Debug, Clone)]
pub struct Ballistics {
    tau_s: f64,
    values: Vec<f32>,
}

impl Ballistics {
    /// Balística con constante de tiempo `tau_s`.
    pub fn new(tau_s: f64) -> Self {
        Self {
            tau_s,
            values: Vec::new(),
        }
    }

    /// Avanza `dt_s` segundos hacia `target` y devuelve la curva mostrada. Si cambia el número de
    /// puntos, salta directamente al objetivo.
    pub fn update(&mut self, target: &[f32], dt_s: f64) -> &[f32] {
        if self.values.len() != target.len() {
            self.values.clear();
            self.values.extend_from_slice(target);
        } else {
            let a = (1.0 - (-dt_s.max(0.0) / self.tau_s.max(1e-6)).exp()) as f32;
            for (v, &t) in self.values.iter_mut().zip(target) {
                *v += a * (t - *v);
            }
        }
        &self.values
    }

    /// Olvida el estado (al cambiar de vista o de dispositivo).
    pub fn reset(&mut self) {
        self.values.clear();
    }
}

/// Nombres de las notas (cifrado americano).
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Nota más cercana a `hz` (La4 = 440 Hz) y desviación en cents: `("B1", 10)` para 62,1 Hz.
pub fn note_of(hz: f64) -> Option<(String, i32)> {
    if !(hz.is_finite() && hz > 0.0) {
        return None;
    }
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let n = midi.round();
    let cents = ((midi - n) * 100.0).round() as i32;
    let n = n as i64;
    let name = NOTES[n.rem_euclid(12) as usize];
    Some((format!("{name}{}", n.div_euclid(12) - 1), cents))
}

/// Texto de la nota para el cursor: `B1 +10 cents`.
pub fn note_text(hz: f64) -> String {
    match note_of(hz) {
        Some((n, c)) => format!("{n} {c:+} cents"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_testkit::{interleave, pink_noise, quantize, sine};

    fn feed(sp: &mut LiveSpectrum, l: &[i32], r: &[i32]) {
        let lr = interleave(&[l.to_vec(), r.to_vec()]);
        for chunk in lr.chunks(2048) {
            sp.push_interleaved(chunk);
        }
    }

    #[test]
    fn fft_segun_frecuencia() {
        assert_eq!(live_fft_size(96_000), 16_384);
        assert_eq!(live_fft_size(48_000), 8192);
        assert_eq!(live_fft_size(44_100), 8192);
        let sp = LiveSpectrum::new(96_000, 24);
        assert!((sp.hz_per_bin() - 5.859_375).abs() < 1e-9);
    }

    #[test]
    fn tono_en_su_bin_y_a_su_nivel() {
        let rate = 96_000;
        let f = 512.0 * 96_000.0 / 16_384.0; // 3.000 Hz exactos en el bin 512
        let s = quantize(&sine(f, rate, 2.0, 0.5), 24);
        let mut sp = LiveSpectrum::new(rate, 24);
        feed(&mut sp, &s, &s);
        let l = sp.layers();
        let peak = l
            .average
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
        assert_eq!(peak.0, 512);
        assert!(
            (power_to_db(peak.1) + 6.02).abs() < 0.1,
            "{}",
            power_to_db(peak.1)
        );
        assert!(l.peak[512] >= l.average[512] * 0.99);
    }

    #[test]
    fn retencion_de_picos_cae_despacio() {
        let rate = 48_000;
        let f = 100.0 * 48_000.0 / 8192.0;
        let s = quantize(&sine(f, rate, 1.0, 0.5), 24);
        let mut sp = LiveSpectrum::new(rate, 24);
        feed(&mut sp, &s, &s);
        let before = power_to_db(sp.layers().peak[100]);
        let silence = vec![0; 48_000];
        feed(&mut sp, &silence, &silence);
        let after = power_to_db(sp.layers().peak[100]);
        // 1 s de silencio a 6 dB/s: unos 6 dB menos (más o menos un frame).
        assert!((before - after - 6.0).abs() < 1.0, "{before} → {after}");
    }

    /// T2.6: el ruido rosa es una curva continua, sin bloques planos por debajo de 100 Hz.
    #[test]
    fn ruido_rosa_sin_escalones_en_graves() {
        let rate = 96_000;
        let n = pink_noise(rate, 6.0, 0.1, 5);
        let q = quantize(&n, 24);
        let mut sp = LiveSpectrum::new(rate, 24);
        feed(&mut sp, &q, &q);
        let l = sp.layers();
        let mut bars = Vec::new();
        log_bands(&l.average, l.hz_per_bin, 192, 20.0, 48_000.0, &mut bars);
        let below_100 = (192.0 * (100.0f64 / 20.0).ln() / (48_000.0f64 / 20.0).ln()) as usize;
        for w in bars[..below_100].windows(2) {
            assert_ne!(
                w[0],
                w[1],
                "bloque plano en graves: {:?}",
                &bars[..below_100]
            );
            assert!(
                (w[0] - w[1]).abs() < 3.0,
                "salto en graves: {:?}",
                &bars[..below_100]
            );
        }
        let grid = log_grid(256, 20.0, 48_000.0);
        let mut curve = Vec::new();
        smooth_curve(&l.average, l.hz_per_bin, &grid, 6.0, &mut curve);
        for (w, f) in curve.windows(2).zip(&grid) {
            if *f < 100.0 {
                assert_ne!(w[0], w[1]);
                assert!((w[0] - w[1]).abs() < 2.0, "salto a {f} Hz");
            }
        }
    }

    /// T2.7: la curva media de un ruido estable, tal como se dibuja (con la balística de pantalla
    /// a 60 fps), no parpadea entre un fotograma y el siguiente.
    #[test]
    fn curva_media_estable_con_ruido() {
        let rate = 96_000;
        let q = quantize(&pink_noise(rate, 8.0, 0.1, 9), 24);
        let mut sp = LiveSpectrum::new(rate, 24);
        let lr = interleave(&[q.clone(), q]);
        let grid = log_grid(256, 50.0, 20_000.0);
        let (mut prev, mut cur): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        let mut target = Vec::new();
        let mut ball = Ballistics::new(DISPLAY_TAU_S);
        let mut worst = 0.0f32;
        for (i, chunk) in lr.chunks(2 * 1600).enumerate() {
            sp.push_interleaved(chunk);
            let l = sp.layers();
            smooth_curve(&l.average, l.hz_per_bin, &grid, 6.0, &mut target);
            cur.clear();
            cur.extend_from_slice(ball.update(&target, 1600.0 / 96_000.0));
            // Tras 3 s de calentamiento, compara cada instantánea (≈ 60 por segundo) con la anterior.
            if i * 1600 > 3 * 96_000 && !prev.is_empty() {
                for (a, b) in cur.iter().zip(&prev) {
                    worst = worst.max((a - b).abs());
                }
            }
            std::mem::swap(&mut prev, &mut cur);
        }
        assert!(worst < 0.5, "la curva cambia {worst} dB entre fotogramas");
    }

    #[test]
    fn balistica_se_acerca_sin_saltar() {
        let mut b = Ballistics::new(0.25);
        assert_eq!(b.update(&[0.0], 1.0 / 60.0), &[0.0]);
        let v = b.update(&[-12.0], 1.0 / 60.0)[0];
        assert!(v < 0.0 && v > -1.0, "{v}");
        let mut last = v;
        for _ in 0..120 {
            last = b.update(&[-12.0], 1.0 / 60.0)[0];
        }
        assert!((last + 12.0).abs() < 0.01);
    }

    #[test]
    fn suavizado_conserva_el_nivel_del_ruido() {
        // Potencia plana: la curva es plana con cualquier fracción.
        let p = vec![1e-4f32; 4097];
        let grid = log_grid(64, 20.0, 20_000.0);
        let mut out = Vec::new();
        for frac in [3.0, 6.0, 12.0] {
            smooth_curve(&p, 5.86, &grid, frac, &mut out);
            assert!(out.iter().all(|&v| (v + 40.0).abs() < 1e-3), "{out:?}");
        }
    }

    #[test]
    fn notas_y_cents() {
        assert_eq!(note_of(440.0), Some(("A4".into(), 0)));
        assert_eq!(note_text(62.1), "B1 +10 cents");
        assert_eq!(note_of(261.63).map(|n| n.0), Some("C4".into()));
        assert_eq!(note_text(27.5), "A0 +0 cents");
        assert_eq!(note_of(0.0), None);
        // Medio semitono por encima de La4 se redondea a La#4 −50.
        let (n, c) = note_of(440.0 * 2f64.powf(0.5 / 12.0) + 0.01).unwrap();
        assert_eq!((n.as_str(), c), ("A#4", -50));
    }

    #[test]
    fn rejilla_e_interpolacion() {
        let g = log_grid(3, 10.0, 1000.0);
        assert!((g[1] - 100.0).abs() < 1e-9 && (g[2] - 1000.0).abs() < 1e-9);
        let p = [0.0, 1.0, 3.0];
        assert!((interp_power(&p, 10.0, 15.0) - 2.0).abs() < 1e-6);
        assert_eq!(interp_power(&p, 10.0, 999.0), 3.0);
    }
}
