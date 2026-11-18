//! Detector de la frecuencia de corte del contenido y estimación del bitrate de origen (F2-T2).
//!
//! Un MP3/AAC aplica un paso bajo abrupto (un "acantilado") cuya frecuencia depende del
//! bitrate. La música real puede caer de forma gradual (roll-off natural, másteres antiguos) o
//! tener ruido de banda ancha (vinilo, cinta): por eso además de la frecuencia se mide la
//! **pendiente** del corte.
//!
//! Algoritmo (ver `docs/ANALYSIS.md`):
//! 1. Perfil `P(f)` = percentil 90 por bin, suavizado (mediana de 5 bins + media móvil).
//! 2. Referencia `R` = mediana de `P` en 1–8 kHz. `R < −90 dB` → sin contenido.
//! 3. Suelo `S` = percentil 10 de `P` en el 4 % superior de la banda.
//! 4. Si `R − S < 20 dB` → banda completa. Si no, el corte `fc` es la frecuencia más alta con
//!    `P > S + 10 dB` en una banda continua de ≥ 300 Hz que termina en `fc`.
//! 5. Pendiente = media de `P` en `[fc − 1200, fc − 200]` − media en `[fc + 200, fc + 1200]`.
//!    Acantilado si ≥ 25 dB.

use super::stats::SpectrumStats;
use super::thresholds::Thresholds;

/// Tipo de resultado del corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CutoffKind {
    /// Hay contenido hasta (casi) Nyquist.
    FullBand,
    /// El contenido termina en `cutoff_hz`.
    Cutoff,
    /// No hay contenido suficiente para analizar (silencio o casi).
    NoContent,
}

/// Origen estimado a partir del corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceEstimate {
    /// Acantilado típico de un codificador con pérdida de ~`kbps`.
    Lossy {
        /// Bitrate aproximado.
        kbps: u32,
        /// `true` si es el primer escalón de la tabla ("≤ 64 kbps").
        or_less: bool,
    },
    /// Sin pérdida o con pérdida de ≥ 320 kbps (corte alto o inexistente).
    LosslessOrHigh,
    /// Contenido limitado sin acantilado (máster o grabación antigua, filtro suave…).
    BandLimited,
    /// Sin contenido: no se puede estimar.
    Unknown,
}

impl SourceEstimate {
    /// Etiqueta para la interfaz: "≈ 128 kbps", "≤ 64 kbps", "sin pérdida o ≥ 320 kbps"…
    pub fn label(&self) -> String {
        match self {
            Self::Lossy {
                kbps,
                or_less: true,
            } => format!("≤ {kbps} kbps"),
            Self::Lossy {
                kbps,
                or_less: false,
            } => format!("≈ {kbps} kbps"),
            Self::LosslessOrHigh => "sin pérdida o ≥ 320 kbps".into(),
            Self::BandLimited => "limitado de banda (posible máster o grabación antigua)".into(),
            Self::Unknown => "desconocido".into(),
        }
    }
}

/// Resultado del detector de corte.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CutoffResult {
    /// Tipo.
    pub kind: CutoffKind,
    /// Frecuencia de corte (Nyquist si es banda completa; 0 sin contenido).
    pub cutoff_hz: f64,
    /// `true` si el corte es un acantilado (pendiente ≥ umbral).
    pub cliff: bool,
    /// Pendiente medida en dB (0 si no aplica).
    pub slope_db: f32,
    /// Nivel de referencia (mediana del perfil en 1–8 kHz).
    pub reference_db: f32,
    /// Suelo estimado en lo alto de la banda.
    pub floor_db: f32,
    /// Origen estimado.
    pub estimated_source: SourceEstimate,
}

/// Detecta la frecuencia de corte del contenido.
pub fn detect_cutoff(spec: &SpectrumStats, t: &Thresholds) -> CutoffResult {
    let nyquist = f64::from(spec.sample_rate) / 2.0;
    let hz = spec.hz_per_bin();
    let profile = smoothed_profile(spec, t);
    let n = profile.len();

    let no_content = |reference_db: f32, floor_db: f32| CutoffResult {
        kind: CutoffKind::NoContent,
        cutoff_hz: 0.0,
        cliff: false,
        slope_db: 0.0,
        reference_db,
        floor_db,
        estimated_source: SourceEstimate::Unknown,
    };
    if spec.active_frames == 0 || n < 8 {
        // Sin frames activos: niveles en el suelo del histograma (valores finitos para JSON).
        let floor = super::stats::HIST_DB_MIN as f32;
        return no_content(floor, floor);
    }

    // 2. Referencia: mediana en 1–8 kHz (recortado a la banda disponible).
    let (r_lo, r_hi) = (
        spec.bin_for_hz(1_000.0),
        spec.bin_for_hz(8_000.0_f64.min(nyquist * 0.8)),
    );
    let reference = median(&profile[r_lo..=r_hi.max(r_lo)]);
    // 3. Suelo: percentil 10 en la franja superior (sin los 3 últimos bins, al menos 20 bins).
    let top_end = n.saturating_sub(3).max(1);
    let band = ((n as f64 * t.floor_band_fraction) as usize)
        .max(20)
        .min(top_end);
    let floor = percentile(&profile[top_end - band..top_end], 10.0);
    if reference < t.no_content_db {
        return no_content(reference, floor);
    }
    let full_band = |slope_db: f32| CutoffResult {
        kind: CutoffKind::FullBand,
        cutoff_hz: nyquist,
        cliff: false,
        slope_db,
        reference_db: reference,
        floor_db: floor,
        estimated_source: SourceEstimate::LosslessOrHigh,
    };
    // 4. ¿Contenido hasta arriba?
    if reference - floor < t.fullband_range_db {
        return full_band(0.0);
    }
    let level = floor + t.content_delta_db;
    let min_run = ((t.min_band_hz / hz).ceil() as usize).max(1);
    let mut run = 0usize;
    let mut cut_bin = None;
    // Recorre de abajo arriba buscando la banda continua más alta.
    for (b, &p) in profile.iter().enumerate() {
        if p > level {
            run += 1;
            if run >= min_run {
                cut_bin = Some(b);
            }
        } else {
            run = 0;
        }
    }
    let Some(cut_bin) = cut_bin else {
        return no_content(reference, floor);
    };
    let cut_bin = refine_cut(&profile, cut_bin, floor, hz, t);
    let cutoff_hz = (cut_bin as f64 + 0.5) * hz;
    let slope = slope_db(&profile, cutoff_hz, hz, t);
    if cutoff_hz >= nyquist - t.fullband_margin_hz {
        return full_band(slope);
    }
    let cliff = slope >= t.cliff_db;
    CutoffResult {
        kind: CutoffKind::Cutoff,
        cutoff_hz,
        cliff,
        slope_db: slope,
        reference_db: reference,
        floor_db: floor,
        estimated_source: estimate_source(cutoff_hz, cliff, t),
    }
}

/// Refina el corte: el primer cruce de "suelo + delta" queda arrastrado hacia arriba por las colas
/// de fuga de la ventana (más con suelos muy bajos, como los de 24 bits). El corte se coloca donde
/// el perfil cruza la **mitad del acantilado**, entre el nivel justo por debajo del corte y el suelo.
fn refine_cut(profile: &[f32], coarse: usize, floor: f32, hz: f64, t: &Thresholds) -> usize {
    let n = profile.len();
    let bin = |f: f64| ((f / hz).round().max(0.0) as usize).min(n - 1);
    let coarse_hz = coarse as f64 * hz;
    let (lo, hi) = (
        bin(coarse_hz - 3.0 * t.slope_outer_hz),
        bin(coarse_hz - t.slope_inner_hz),
    );
    if hi <= lo {
        return coarse;
    }
    // Nivel de la banda pasante: el percentil 75 de una franja amplia bajo el corte grueso, para que
    // la propia transición no lo arrastre hacia abajo.
    let pass = percentile(&profile[lo..=hi], 75.0);
    if pass <= floor {
        return coarse;
    }
    let mid = (pass + floor) / 2.0;
    // El bin más alto ≤ grueso cuyo nivel supera la mitad.
    (lo..=coarse)
        .rev()
        .find(|&b| profile[b] >= mid)
        .unwrap_or(coarse)
}

/// Origen estimado para un corte en `cutoff_hz`.
pub fn estimate_source(cutoff_hz: f64, cliff: bool, t: &Thresholds) -> SourceEstimate {
    if !cliff {
        return SourceEstimate::BandLimited;
    }
    t.bitrate_table
        .iter()
        .enumerate()
        .find(|(_, s)| cutoff_hz < s.max_hz)
        .map_or(SourceEstimate::LosslessOrHigh, |(i, s)| {
            SourceEstimate::Lossy {
                kbps: s.kbps,
                or_less: i == 0,
            }
        })
}

/// Perfil del percentil, suavizado con una mediana de 5 bins y una media móvil de `smoothing_hz`.
pub fn smoothed_profile(spec: &SpectrumStats, t: &Thresholds) -> Vec<f32> {
    let raw = spec.percentile_profile(t.cutoff_percentile);
    let med = median_filter(&raw, 2);
    let half = ((t.smoothing_hz / spec.hz_per_bin() / 2.0).round() as usize).max(1);
    moving_average(&med, half)
}

fn slope_db(profile: &[f32], cutoff_hz: f64, hz: f64, t: &Thresholds) -> f32 {
    let n = profile.len();
    let bin = |f: f64| ((f / hz).round().max(0.0) as usize).min(n - 1);
    let below = (
        bin(cutoff_hz - t.slope_outer_hz),
        bin(cutoff_hz - t.slope_inner_hz),
    );
    let above = (
        bin(cutoff_hz + t.slope_inner_hz),
        bin(cutoff_hz + t.slope_outer_hz),
    );
    // Si no hay sitio por encima del corte (cerca de Nyquist), la pendiente no se puede medir.
    if above.0 >= n - 1 || above.1 <= above.0 || below.1 <= below.0 {
        return 0.0;
    }
    mean(&profile[below.0..=below.1]) - mean(&profile[above.0..=above.1])
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn median(v: &[f32]) -> f32 {
    percentile(v, 50.0)
}

/// Percentil `p` (0–100) de `v` por el método del rango más cercano.
fn percentile(v: &[f32], p: f64) -> f32 {
    if v.is_empty() {
        return f32::NEG_INFINITY;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let idx = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    s[idx.min(s.len() - 1)]
}

/// Mediana móvil de radio `r` (ventana `2r + 1`, recortada en los bordes).
fn median_filter(v: &[f32], r: usize) -> Vec<f32> {
    (0..v.len())
        .map(|i| median(&v[i.saturating_sub(r)..(i + r + 1).min(v.len())]))
        .collect()
}

/// Media móvil de radio `r` (recortada en los bordes), en O(n).
fn moving_average(v: &[f32], r: usize) -> Vec<f32> {
    let mut prefix = Vec::with_capacity(v.len() + 1);
    prefix.push(0.0f64);
    for &x in v {
        let last = prefix.last().copied().unwrap_or(0.0);
        prefix.push(last + f64::from(x));
    }
    (0..v.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(r), (i + r + 1).min(v.len()));
            ((prefix[b] - prefix[a]) / (b - a) as f64) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Estadísticas con un perfil fijo: `level_db(f)` en cada bin, `frames` frames idénticos.
    fn stats(rate: u32, fft: usize, level_db: impl Fn(f64) -> f64) -> SpectrumStats {
        let mut s = SpectrumStats::new(rate, fft);
        let frame: Vec<f32> = (0..s.bins)
            .map(|b| 10f64.powf(level_db(b as f64 * s.hz_per_bin()) / 10.0) as f32)
            .collect();
        for _ in 0..20 {
            s.add_frame(&frame, true);
        }
        s
    }

    #[test]
    fn escalon_a_16_khz_es_un_mp3_de_128() {
        let s = stats(44_100, 8192, |f| if f < 16_000.0 { -50.0 } else { -140.0 });
        let r = detect_cutoff(&s, &Thresholds::default());
        assert_eq!(r.kind, CutoffKind::Cutoff);
        assert!((r.cutoff_hz - 16_000.0).abs() < 200.0, "{}", r.cutoff_hz);
        assert!(r.cliff && r.slope_db > 80.0);
        assert_eq!(
            r.estimated_source,
            SourceEstimate::Lossy {
                kbps: 128,
                or_less: false
            }
        );
        assert_eq!(r.estimated_source.label(), "≈ 128 kbps");
    }

    #[test]
    fn rampa_suave_no_es_acantilado() {
        // −50 dB hasta 8 kHz y luego −6 dB por kHz hasta el suelo.
        let s = stats(44_100, 8192, |f| {
            if f < 8_000.0 {
                -50.0
            } else {
                (-50.0 - 6.0 * (f - 8_000.0) / 1000.0).max(-140.0)
            }
        });
        let r = detect_cutoff(&s, &Thresholds::default());
        assert_eq!(r.kind, CutoffKind::Cutoff);
        assert!(!r.cliff, "pendiente {}", r.slope_db);
        assert_eq!(r.estimated_source, SourceEstimate::BandLimited);
    }

    #[test]
    fn banda_completa_y_sin_contenido() {
        let s = stats(44_100, 8192, |_| -50.0);
        let r = detect_cutoff(&s, &Thresholds::default());
        assert_eq!(r.kind, CutoffKind::FullBand);
        assert_eq!(r.cutoff_hz, 22_050.0);
        assert_eq!(r.estimated_source, SourceEstimate::LosslessOrHigh);

        let s = stats(44_100, 8192, |_| -130.0);
        assert_eq!(
            detect_cutoff(&s, &Thresholds::default()).kind,
            CutoffKind::NoContent
        );
        let empty = SpectrumStats::new(44_100, 8192);
        assert_eq!(
            detect_cutoff(&empty, &Thresholds::default()).kind,
            CutoffKind::NoContent
        );
    }

    #[test]
    fn tabla_de_bitrates() {
        let t = Thresholds::default();
        let kbps = |f: f64| match estimate_source(f, true, &t) {
            SourceEstimate::Lossy { kbps, .. } => kbps,
            _ => 0,
        };
        assert_eq!(kbps(10_000.0), 64);
        assert_eq!(estimate_source(10_000.0, true, &t).label(), "≤ 64 kbps");
        // Cortes medidos de LAME 3.100 (ffmpeg libmp3lame) y FhG.
        assert_eq!(kbps(15_400.0), 96, "LAME 96/112");
        assert_eq!(kbps(16_000.0), 128, "FhG 128");
        assert_eq!(kbps(16_800.0), 128, "LAME 128");
        assert_eq!(kbps(17_500.0), 160);
        assert_eq!(kbps(18_900.0), 192);
        assert_eq!(kbps(19_600.0), 256, "LAME 224/256");
        assert_eq!(kbps(20_000.0), 320);
        assert_eq!(kbps(20_250.0), 320);
        assert_eq!(
            estimate_source(21_000.0, true, &t),
            SourceEstimate::LosslessOrHigh
        );
    }

    #[test]
    fn filtros() {
        assert_eq!(median_filter(&[0.0, 0.0, 9.0, 0.0, 0.0], 1), vec![0.0; 5]);
        assert_eq!(moving_average(&[0.0, 3.0, 6.0], 1), vec![1.5, 3.0, 4.5]);
        assert_eq!(percentile(&[5.0, 1.0, 3.0], 50.0), 3.0);
        assert_eq!(percentile(&[], 50.0), f32::NEG_INFINITY);
    }
}
