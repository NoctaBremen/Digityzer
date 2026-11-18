//! Estadísticas que acumula el pase único de análisis (F2-T1).
//!
//! Los detectores (F2-T2 y F2-T3) son funciones **puras** sobre estas estructuras, así que se
//! pueden probar con estadísticas construidas a mano.

use std::collections::HashMap;

/// dB mínimo de los histogramas espectrales.
pub const HIST_DB_MIN: i32 = -160;
/// dB máximo de los histogramas espectrales.
pub const HIST_DB_MAX: i32 = 10;
/// Número de cubetas de 1 dB.
pub const HIST_BUCKETS: usize = (HIST_DB_MAX - HIST_DB_MIN + 1) as usize;

/// Umbral de las rachas de clipping: −1 dBFS (0,891 de la escala completa).
pub const RUN_THRESHOLD: f64 = 0.891;
/// Máximo de valores distintos de rachas que se guardan (se conservan los más altos).
pub const MAX_RUN_KEYS: usize = 4096;

/// Estadísticas espectrales por bin: histograma de dB (solo frames activos) y LTAS.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumStats {
    /// Frecuencia de muestreo.
    pub sample_rate: u32,
    /// Tamaño de la FFT.
    pub fft_size: usize,
    /// Número de bins (`fft_size / 2 + 1`).
    pub bins: usize,
    /// Frames activos (los que entran en los histogramas).
    pub active_frames: u64,
    /// Frames totales.
    pub total_frames: u64,
    /// Ancho de banda de ruido equivalente de la ventana usada, en bins (para pasar niveles por
    /// bin a niveles RMS totales). 1,0 si no se indica (ver [`Self::with_enbw`]).
    pub enbw_bins: f64,
    /// Histogramas: `hist[cubeta * bins + bin]`; cubeta 0 = −160 dB. Disposición por cubeta: los
    /// bins vecinos de un frame suelen caer en cubetas parecidas, así que sus escrituras quedan
    /// cerca en memoria (con `bin * HIST_BUCKETS + cubeta` cada bin tocaba otra línea de caché).
    hist: Vec<u32>,
    /// Suma de potencias lineales por bin (frames activos), para el LTAS.
    power_sum: Vec<f64>,
    /// Límites inferiores de las cubetas 1.. en potencia lineal (`10^((dB − 0,5)/10)`), para
    /// clasificar sin calcular un logaritmo por bin.
    bounds: Vec<f32>,
}

impl SpectrumStats {
    /// Crea estadísticas vacías.
    pub fn new(sample_rate: u32, fft_size: usize) -> Self {
        let bins = fft_size / 2 + 1;
        Self {
            sample_rate,
            fft_size,
            bins,
            active_frames: 0,
            total_frames: 0,
            enbw_bins: 1.0,
            hist: vec![0; bins * HIST_BUCKETS],
            power_sum: vec![0.0; bins],
            bounds: (1..HIST_BUCKETS)
                .map(|i| 10f64.powf((f64::from(HIST_DB_MIN + i as i32) - 0.5) / 10.0) as f32)
                .collect(),
        }
    }

    /// Indica el ENBW de la ventana del análisis.
    pub fn with_enbw(mut self, enbw_bins: f64) -> Self {
        self.enbw_bins = enbw_bins;
        self
    }

    /// Convierte un nivel **por bin** de ruido blanco en el nivel RMS total equivalente (dBFS):
    /// `RMS² = p · N / (4 · ENBW)` (ver `dsp::frame` y `frame_rms_db`).
    pub fn per_bin_to_rms_db(&self, per_bin_db: f64) -> f64 {
        per_bin_db + 10.0 * (self.fft_size as f64 / (4.0 * self.enbw_bins)).log10()
    }

    /// Cubeta del histograma de una potencia lineal (redondeo al dB más cercano, recortado).
    ///
    /// Estimación rápida con un `log2` aproximado a partir de los bits del `f32` (error < 0,3 dB)
    /// y corrección exacta con los límites precalculados: el resultado es el mismo que redondear
    /// `10·log10(p)`, con una o dos comparaciones en vez de un logaritmo.
    fn bucket(&self, p: f32) -> usize {
        let last = HIST_BUCKETS - 1;
        if p.is_nan() || p <= self.bounds[0] {
            return 0;
        }
        if p >= self.bounds[last - 1] {
            return last;
        }
        // log2(p) ≈ bits · 2⁻²³ − 127 (mantisa interpolada linealmente).
        let log2 = p.to_bits() as f32 * (1.0 / 8_388_608.0) - 127.0;
        let db = log2 * 3.010_3;
        let mut idx = ((db - HIST_DB_MIN as f32).round() as i32).clamp(0, last as i32) as usize;
        // bounds[i − 1] es el límite inferior de la cubeta i.
        while idx > 0 && p < self.bounds[idx - 1] {
            idx -= 1;
        }
        while idx < last && p >= self.bounds[idx] {
            idx += 1;
        }
        idx
    }

    /// Hz por bin.
    pub fn hz_per_bin(&self) -> f64 {
        f64::from(self.sample_rate) / self.fft_size as f64
    }

    /// Bin que contiene `hz` (recortado al rango).
    pub fn bin_for_hz(&self, hz: f64) -> usize {
        ((hz / self.hz_per_bin()).round().max(0.0) as usize).min(self.bins - 1)
    }

    /// Añade un frame de potencias (lineales). Si no es activo, solo cuenta en `total_frames`.
    pub fn add_frame(&mut self, power: &[f32], active: bool) {
        self.total_frames += 1;
        if !active {
            return;
        }
        self.active_frames += 1;
        for (b, &p) in power.iter().take(self.bins).enumerate() {
            self.power_sum[b] += f64::from(p);
            let idx = self.bucket(p);
            self.hist[idx * self.bins + b] += 1;
        }
    }

    /// Percentil `p` (0–100) del nivel en dB del bin `bin`, sobre los frames activos.
    ///
    /// Resolución de 1 dB. Sin frames activos devuelve −160 dB.
    pub fn percentile_db(&self, bin: usize, p: f64) -> f32 {
        if self.active_frames == 0 || bin >= self.bins {
            return HIST_DB_MIN as f32;
        }
        let target = ((p.clamp(0.0, 100.0) / 100.0) * self.active_frames as f64)
            .ceil()
            .max(1.0) as u64;
        let mut acc = 0u64;
        for i in 0..HIST_BUCKETS {
            acc += u64::from(self.hist[i * self.bins + bin]);
            if acc >= target {
                return (HIST_DB_MIN + i as i32) as f32;
            }
        }
        HIST_DB_MAX as f32
    }

    /// Perfil de un percentil para todos los bins.
    pub fn percentile_profile(&self, p: f64) -> Vec<f32> {
        (0..self.bins).map(|b| self.percentile_db(b, p)).collect()
    }

    /// Espectro medio a largo plazo (LTAS), en dB, sobre los frames activos.
    pub fn ltas_db(&self) -> Vec<f32> {
        ltas(&self.power_sum, self.active_frames)
    }
}

/// Espectro medio a largo plazo (LTAS) sin histogramas: la versión ligera de [`SpectrumStats`].
///
/// Se usa para la vista Side (L − R), donde de momento solo interesa el nivel medio por bin
/// (p. ej. para detectar el joint-stereo de los MP3 en el futuro). Cuesta una suma por bin.
#[derive(Debug, Clone, PartialEq)]
pub struct LtasStats {
    /// Frecuencia de muestreo.
    pub sample_rate: u32,
    /// Tamaño de la FFT.
    pub fft_size: usize,
    /// Frames activos.
    pub active_frames: u64,
    /// Frames totales.
    pub total_frames: u64,
    power_sum: Vec<f64>,
}

impl LtasStats {
    /// Crea estadísticas vacías.
    pub fn new(sample_rate: u32, fft_size: usize) -> Self {
        Self {
            sample_rate,
            fft_size,
            active_frames: 0,
            total_frames: 0,
            power_sum: vec![0.0; fft_size / 2 + 1],
        }
    }

    /// Añade un frame (solo los activos suman).
    pub fn add_frame(&mut self, power: &[f32], active: bool) {
        self.total_frames += 1;
        if active {
            self.active_frames += 1;
            for (s, &p) in self.power_sum.iter_mut().zip(power) {
                *s += f64::from(p);
            }
        }
    }

    /// LTAS en dB (−160 dB sin frames activos).
    pub fn ltas_db(&self) -> Vec<f32> {
        ltas(&self.power_sum, self.active_frames)
    }
}

fn ltas(power_sum: &[f64], active: u64) -> Vec<f32> {
    let n = active.max(1) as f64;
    power_sum
        .iter()
        .map(|&s| (10.0 * (s / n).max(1e-30).log10()).max(f64::from(HIST_DB_MIN)) as f32)
        .collect()
}

/// Estadísticas de una racha de muestras idénticas en un valor absoluto concreto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunStats {
    /// Número de rachas (≥ 2 muestras).
    pub runs: u64,
    /// Muestras totales en esas rachas.
    pub run_samples: u64,
    /// Racha más larga.
    pub max_run: u64,
    /// Número de rachas por longitud: índice 0 = 2 muestras, 1 = 3, … 6 = 8 o más.
    pub by_length: [u64; 7],
}

impl RunStats {
    /// Rachas de al menos `min_len` muestras (las de 8 o más se cuentan juntas).
    pub fn runs_at_least(&self, min_len: u64) -> u64 {
        let first = min_len.clamp(2, 8) as usize - 2;
        self.by_length[first..].iter().sum()
    }
}

/// Rachas de muestras idénticas cerca de la escala completa, por valor absoluto.
///
/// Clave: el valor absoluto (enteros: el valor; float: los bits de `|v|` como `f32`). Se cuentan
/// por canal y continúan entre bloques. Solo se registran rachas de ≥ 2 muestras con
/// `|v| ≥ umbral`. Como mucho [`MAX_RUN_KEYS`] claves (se conservan las más altas).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunMap {
    /// Rachas por clave.
    pub by_value: HashMap<u32, RunStats>,
    /// Estado por canal: (clave actual, longitud) de la racha abierta.
    open: Vec<Option<(u32, u64)>>,
}

impl RunMap {
    fn ensure_channels(&mut self, channels: usize) {
        if self.open.len() < channels {
            self.open.resize(channels, None);
        }
    }

    /// Añade una muestra del canal `ch`. `key = None` si está por debajo del umbral.
    #[inline]
    fn push(&mut self, ch: usize, key: Option<u32>) {
        // Caso más común: muestra bajo el umbral y sin racha abierta.
        if key.is_none() && self.open[ch].is_none() {
            return;
        }
        match (self.open[ch], key) {
            (Some((k, n)), Some(new)) if k == new => self.open[ch] = Some((k, n + 1)),
            (prev, new) => {
                if let Some((k, n)) = prev {
                    self.record(k, n);
                }
                self.open[ch] = new.map(|k| (k, 1));
            }
        }
    }

    fn record(&mut self, key: u32, len: u64) {
        if len < 2 {
            return;
        }
        if !self.by_value.contains_key(&key) && self.by_value.len() >= MAX_RUN_KEYS {
            // Lleno: se descarta la clave más baja si la nueva es mayor.
            let Some(&min) = self.by_value.keys().min() else {
                return;
            };
            if key <= min {
                return;
            }
            self.by_value.remove(&min);
        }
        let e = self.by_value.entry(key).or_default();
        e.runs += 1;
        e.run_samples += len;
        e.max_run = e.max_run.max(len);
        e.by_length[(len.min(8) - 2) as usize] += 1;
    }

    /// Cierra las rachas abiertas (al final del archivo).
    pub fn finish(&mut self) {
        for ch in 0..self.open.len() {
            if let Some((k, n)) = self.open[ch].take() {
                self.record(k, n);
            }
        }
    }
}

/// Estadísticas de muestras enteras.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleStats {
    /// Profundidad declarada.
    pub bits_declared: u32,
    /// OR de los bits bajos (`v & (2^bits − 1)`) de todas las muestras.
    pub or_mask_low: u32,
    /// Histograma del byte bajo (`v & 0xFF`), solo si `bits ≥ 24`.
    pub low_byte_hist: Option<Box<[u64; 256]>>,
    /// Pico absoluto.
    pub peak_abs: i64,
    /// Muestras totales (todos los canales).
    pub samples: u64,
    /// Escala completa: `2^(bits − 1)`.
    pub full_scale: i64,
    /// Rachas de muestras idénticas por encima de −1 dBFS (clave = `|v|`).
    pub runs: RunMap,
}

impl SampleStats {
    /// Crea estadísticas vacías para `bits` bits.
    pub fn new(bits: u32) -> Self {
        Self {
            bits_declared: bits,
            or_mask_low: 0,
            low_byte_hist: (bits >= 24).then(|| Box::new([0u64; 256])),
            peak_abs: 0,
            samples: 0,
            full_scale: 1i64 << bits.saturating_sub(1),
            runs: RunMap::default(),
        }
    }

    /// Añade muestras entrelazadas de `channels` canales.
    pub fn add(&mut self, samples: &[i32], channels: usize) {
        let mask: u32 = if self.bits_declared >= 32 {
            u32::MAX
        } else {
            (1u32 << self.bits_declared) - 1
        };
        let threshold = (RUN_THRESHOLD * self.full_scale as f64) as i64;
        self.runs.ensure_channels(channels);
        let channels = channels.max(1);
        let mut or = 0u32;
        let mut peak = self.peak_abs;
        // Por frames: el canal es la posición dentro del frame (sin una división por muestra).
        for frame in samples.chunks(channels) {
            for (ch, &v) in frame.iter().enumerate() {
                or |= v as u32;
                let a = i64::from(v).abs();
                peak = peak.max(a);
                let key = (a >= threshold).then_some(a.min(i64::from(u32::MAX)) as u32);
                self.runs.push(ch, key);
            }
        }
        self.or_mask_low |= or & mask;
        self.peak_abs = peak;
        if let Some(h) = self.low_byte_hist.as_mut() {
            for &v in samples {
                h[(v as u32 & 0xFF) as usize] += 1;
            }
        }
        self.samples += samples.len() as u64;
    }
}

/// Estadísticas de muestras en coma flotante.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatStats {
    /// Pico absoluto (1,0 = escala completa).
    pub peak: f64,
    /// Todas las muestras están en la rejilla de 16 bits (`v · 32768` entero).
    pub on_16bit_grid: bool,
    /// Todas las muestras están en la rejilla de 24 bits (`v · 8388608` entero).
    pub on_24bit_grid: bool,
    /// Muestras totales.
    pub samples: u64,
    /// Rachas de muestras idénticas por encima de −1 dBFS (clave = bits de `|v|` como `f32`).
    pub runs: RunMap,
}

impl Default for FloatStats {
    fn default() -> Self {
        Self {
            peak: 0.0,
            on_16bit_grid: true,
            on_24bit_grid: true,
            samples: 0,
            runs: RunMap::default(),
        }
    }
}

impl FloatStats {
    /// Añade muestras entrelazadas de `channels` canales.
    pub fn add(&mut self, samples: impl Iterator<Item = f64>, channels: usize) {
        self.runs.ensure_channels(channels);
        for (i, v) in samples.enumerate() {
            let a = v.abs();
            if a.is_finite() {
                self.peak = self.peak.max(a);
            }
            if self.on_16bit_grid && !on_grid(v, 32_768.0) {
                self.on_16bit_grid = false;
            }
            if self.on_24bit_grid && !on_grid(v, 8_388_608.0) {
                self.on_24bit_grid = false;
            }
            let key = (a >= RUN_THRESHOLD && a.is_finite()).then(|| (a as f32).to_bits());
            self.runs.push(i % channels.max(1), key);
            self.samples += 1;
        }
    }
}

fn on_grid(v: f64, scale: f64) -> bool {
    let x = v * scale;
    x.is_finite() && x.fract() == 0.0 && x.abs() <= scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_del_histograma() {
        let mut s = SpectrumStats::new(48_000, 8);
        // 5 bins. Bin 2: potencias 1e-6 (−60 dB) ×3 y 1e-2 (−20 dB) ×1.
        for p in [1e-6f32, 1e-6, 1e-6, 1e-2] {
            s.add_frame(&[0.0, 0.0, p, 0.0, 0.0], true);
        }
        s.add_frame(&[1.0; 5], false); // no activo: no cuenta
        assert_eq!((s.active_frames, s.total_frames), (4, 5));
        assert_eq!(s.percentile_db(2, 50.0), -60.0);
        assert_eq!(s.percentile_db(2, 75.0), -60.0);
        assert_eq!(s.percentile_db(2, 90.0), -20.0);
        assert_eq!(s.percentile_db(2, 100.0), -20.0);
        assert_eq!(s.percentile_db(0, 50.0), -160.0, "potencia 0 → suelo");
        // LTAS: media lineal (3·1e-6 + 1e-2)/4 ≈ 2,5e-3 → −26 dB.
        let ltas = s.ltas_db();
        assert!((ltas[2] - 10.0 * (0.010003f32 / 4.0).log10()).abs() < 0.01);
        assert_eq!(s.bin_for_hz(12_000.0), 2);
        assert_eq!(s.bin_for_hz(1e9), 4);
    }

    #[test]
    fn cubetas_exactas() {
        let s = SpectrumStats::new(48_000, 8);
        let db = |d: f64| 10f64.powf(d / 10.0) as f32;
        assert_eq!(s.bucket(0.0), 0);
        assert_eq!(s.bucket(f32::NAN), 0);
        assert_eq!(s.bucket(db(-200.0)), 0);
        assert_eq!(s.bucket(db(-160.0)), 0);
        assert_eq!(s.bucket(db(-60.0)), 100);
        assert_eq!(s.bucket(db(-60.4)), 100);
        assert_eq!(s.bucket(db(-59.6)), 100);
        assert_eq!(s.bucket(db(-59.4)), 101);
        assert_eq!(s.bucket(db(0.0)), 160);
        assert_eq!(s.bucket(db(10.0)), 170);
        assert_eq!(s.bucket(db(40.0)), 170);
        assert_eq!(s.bucket(f32::INFINITY), 170);
    }

    #[test]
    fn cubeta_rapida_igual_a_la_exacta() {
        let s = SpectrumStats::new(48_000, 8);
        let mut x = 1e-25f64;
        while x < 1e5 {
            let p = x as f32;
            let exact = s.bounds.partition_point(|&b| b <= p);
            assert_eq!(s.bucket(p), exact, "p = {p:e}");
            x *= 1.0137;
        }
    }

    #[test]
    fn sin_frames_activos() {
        let s = SpectrumStats::new(44_100, 8);
        assert_eq!(s.percentile_db(1, 50.0), -160.0);
        assert!(s.ltas_db().iter().all(|&d| d == -160.0));
    }

    #[test]
    fn mascara_y_byte_bajo() {
        let mut s = SampleStats::new(24);
        s.add(&[256, -256, 512 * 3, -8_388_608], 2);
        assert_eq!(
            s.or_mask_low & 0xFF,
            0,
            "16 bits desplazados: byte bajo vacío"
        );
        let h = s.low_byte_hist.as_ref().unwrap();
        assert_eq!(h[0], 4);
        assert_eq!(s.peak_abs, 8_388_608);
        assert_eq!(s.samples, 4);
        let mut s16 = SampleStats::new(16);
        s16.add(&[1, -1], 1);
        assert!(s16.low_byte_hist.is_none());
        assert_eq!(s16.or_mask_low, 0xFFFF, "−1 pone todos los bits bajos");
    }

    #[test]
    fn rachas_por_canal_y_entre_bloques() {
        let fs = 32_767;
        let mut s = SampleStats::new(16);
        // Estéreo: canal 0 con una racha de 3 en FS que cruza dos bloques; canal 1 sin rachas.
        s.add(&[fs, 0, fs, 1], 2);
        s.add(&[fs, 2, 100, 3], 2);
        s.runs.finish();
        let r = s.runs.by_value.get(&(fs as u32)).copied().unwrap();
        assert_eq!(
            r,
            RunStats {
                runs: 1,
                run_samples: 3,
                max_run: 3,
                by_length: [0, 1, 0, 0, 0, 0, 0]
            }
        );
        assert_eq!((r.runs_at_least(3), r.runs_at_least(4)), (1, 0));
        // Rachas de 1 muestra o por debajo del umbral no cuentan.
        let mut s = SampleStats::new(16);
        s.add(&[fs, 100, 100, 100, -fs, -fs], 1);
        s.runs.finish();
        assert_eq!(s.runs.by_value.len(), 1, "solo la racha de −FS ×2");
        assert_eq!(s.runs.by_value[&(fs as u32)].max_run, 2);
        assert_eq!(s.runs.by_value[&(fs as u32)].runs_at_least(2), 1);
        assert_eq!(s.runs.by_value[&(fs as u32)].runs_at_least(3), 0);
        // Longitudes ≥ 8 se acumulan juntas.
        let mut s = SampleStats::new(16);
        s.add(&[fs; 20], 1);
        s.runs.finish();
        assert_eq!(s.runs.by_value[&(fs as u32)].by_length[6], 1);
        assert_eq!(s.runs.by_value[&(fs as u32)].runs_at_least(30), 1);
    }

    #[test]
    fn mapa_de_rachas_acotado() {
        let mut m = RunMap::default();
        m.ensure_channels(1);
        for k in 0..(MAX_RUN_KEYS as u32 + 10) {
            m.push(0, Some(k + 1000));
            m.push(0, Some(k + 1000));
            m.push(0, None);
        }
        assert_eq!(m.by_value.len(), MAX_RUN_KEYS);
        assert!(
            m.by_value.contains_key(&(MAX_RUN_KEYS as u32 + 1009)),
            "se conservan las más altas"
        );
        assert!(!m.by_value.contains_key(&1000));
    }

    #[test]
    fn rejilla_float() {
        let mut f = FloatStats::default();
        f.add([0.5, -1.0, 1.0 / 32_768.0, 0.0].into_iter(), 1);
        assert!(f.on_16bit_grid && f.on_24bit_grid);
        f.add([1.0 / 65_536.0].into_iter(), 1);
        assert!(!f.on_16bit_grid && f.on_24bit_grid);
        f.add([0.1].into_iter(), 1);
        assert!(!f.on_24bit_grid);
        assert_eq!(f.peak, 1.0);
        let mut g = FloatStats::default();
        g.add([f64::NAN].into_iter(), 1);
        assert!(!g.on_16bit_grid, "NaN no está en la rejilla");
    }
}
