//! Marcas de los ejes y de la leyenda (sin dibujar: la interfaz pinta el texto).
//!
//! Como en Spek, se elige el primer "factor bonito" cuyo espaciado en píxeles es al menos
//! `min_spacing_px`. Siempre se incluyen los extremos del eje (salvo que el eje sea demasiado
//! corto) y se descartan las marcas interiores que quedarían demasiado cerca del extremo final.
//! Las etiquetas usan coma decimal; los dB negativos llevan el signo menos tipográfico (U+2212).

/// Una marca de un eje.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// Valor en las unidades del eje (s, Hz o dB).
    pub value: f64,
    /// Etiqueta lista para mostrar.
    pub label: String,
    /// Posición en píxeles desde el origen del eje (izquierda para el tiempo, abajo para
    /// frecuencia y dB).
    pub pos_px: f64,
}

/// Longitud mínima de un eje para tener marcas interiores.
pub const MIN_AXIS_PX: f64 = 20.0;

const TIME_FACTORS: [f64; 13] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0,
];
const FREQ_FACTORS: [f64; 6] = [500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0];
const DB_FACTORS: [f64; 5] = [3.0, 6.0, 10.0, 20.0, 30.0];

/// Marcas del eje de tiempo (de 0 a `duration_s`). Etiquetas `m:ss` o `h:mm:ss`.
///
/// Duración nula, negativa o no finita → sin marcas (la interfaz muestra "?").
pub fn time_ticks(duration_s: f64, length_px: f64, min_spacing_px: f64) -> Vec<Tick> {
    let end_label = |v: f64| {
        if v < 10.0 && v.fract().abs() > 1e-6 {
            fmt_time_tenths(v)
        } else {
            fmt_time(v)
        }
    };
    axis_ticks(
        0.0,
        duration_s,
        length_px,
        min_spacing_px,
        &TIME_FACTORS,
        fmt_time,
        end_label,
    )
}

/// Marcas del eje de frecuencia (de 0 a `nyquist_hz`). Etiquetas "16 kHz", "22,05 kHz".
pub fn freq_ticks(nyquist_hz: f64, length_px: f64, min_spacing_px: f64) -> Vec<Tick> {
    axis_ticks(
        0.0,
        nyquist_hz,
        length_px,
        min_spacing_px,
        &FREQ_FACTORS,
        fmt_khz,
        fmt_khz,
    )
}

/// Marcas de la leyenda de dB (de `db_min` abajo a `db_max` arriba). Etiquetas "−60 dB".
///
/// Si `db_min > db_max`, se intercambian.
pub fn db_ticks(db_min: f64, db_max: f64, length_px: f64, min_spacing_px: f64) -> Vec<Tick> {
    let (lo, hi) = if db_min <= db_max {
        (db_min, db_max)
    } else {
        (db_max, db_min)
    };
    axis_ticks(
        lo,
        hi,
        length_px,
        min_spacing_px,
        &DB_FACTORS,
        fmt_db,
        fmt_db,
    )
}

fn axis_ticks(
    min: f64,
    max: f64,
    length_px: f64,
    min_spacing_px: f64,
    factors: &[f64],
    interior_label: impl Fn(f64) -> String,
    extreme_label: impl Fn(f64) -> String,
) -> Vec<Tick> {
    if !(min.is_finite() && max.is_finite() && length_px.is_finite())
        || max <= min
        || length_px <= 0.0
    {
        return Vec::new();
    }
    let scale = length_px / (max - min);
    let tick = |v: f64, label: String| Tick {
        value: v,
        label,
        pos_px: (v - min) * scale,
    };
    let mut ticks = vec![tick(min, extreme_label(min))];

    if length_px >= MIN_AXIS_PX
        && let Some(&factor) = factors.iter().find(|&&f| f * scale >= min_spacing_px)
    {
        let mut k = (min / factor).floor() + 1.0;
        loop {
            let v = k * factor;
            if v >= max {
                break;
            }
            let pos = (v - min) * scale;
            if pos >= min_spacing_px && length_px - pos >= min_spacing_px {
                ticks.push(tick(v, interior_label(v)));
            }
            k += 1.0;
        }
    }
    // El extremo final, salvo que se solape con el origen en un eje más largo que el mínimo.
    if length_px < MIN_AXIS_PX || length_px >= min_spacing_px {
        ticks.push(tick(max, extreme_label(max)));
    }
    ticks
}

/// Tiempo como `m:ss` (o `h:mm:ss` desde una hora), truncando los segundos.
pub fn fmt_time(secs: f64) -> String {
    let total = secs.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Tiempo con décimas: `1:23,4`.
pub fn fmt_time_tenths(secs: f64) -> String {
    let tenths = (secs.max(0.0) * 10.0).floor() as u64;
    format!("{},{}", fmt_time((tenths / 10) as f64), tenths % 10)
}

/// Frecuencia en kHz con hasta 3 decimales y coma: 16.000 → "16 kHz", 22.050 → "22,05 kHz",
/// 500 → "0,5 kHz", 0 → "0 kHz".
pub fn fmt_khz(hz: f64) -> String {
    let s = format!("{:.3}", hz / 1000.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{} kHz", s.replace('.', ","))
}

/// kHz con un decimal: 16.234 → "16,2 kHz".
pub fn fmt_khz_1(hz: f64) -> String {
    format!("{:.1} kHz", hz / 1000.0).replace('.', ",")
}

/// dB enteros con el signo menos tipográfico: "−60 dB", "0 dB".
pub fn fmt_db(db: f64) -> String {
    let v = db.round() as i64;
    if v < 0 {
        format!("\u{2212}{} dB", -v)
    } else {
        format!("{v} dB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(t: &[Tick]) -> Vec<&str> {
        t.iter().map(|t| t.label.as_str()).collect()
    }

    fn assert_no_overlap(t: &[Tick], min_spacing: f64, what: &str) {
        for w in t.windows(2) {
            assert!(
                w[1].pos_px - w[0].pos_px >= min_spacing - 1e-9,
                "{what}: {:?} y {:?} se solapan",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn frecuencia_44_1_y_96() {
        let t = freq_ticks(22_050.0, 600.0, 40.0);
        assert_eq!(
            labels(&t),
            [
                "0 kHz",
                "2 kHz",
                "4 kHz",
                "6 kHz",
                "8 kHz",
                "10 kHz",
                "12 kHz",
                "14 kHz",
                "16 kHz",
                "18 kHz",
                "20 kHz",
                "22,05 kHz"
            ]
        );
        assert_eq!(t.last().unwrap().pos_px, 600.0);
        let t = freq_ticks(48_000.0, 600.0, 40.0);
        assert_eq!(
            labels(&t),
            [
                "0 kHz", "5 kHz", "10 kHz", "15 kHz", "20 kHz", "25 kHz", "30 kHz", "35 kHz",
                "40 kHz", "48 kHz"
            ]
        );
    }

    #[test]
    fn tiempo() {
        let t = time_ticks(330.0, 800.0, 60.0);
        assert_eq!(
            labels(&t),
            [
                "0:00", "0:30", "1:00", "1:30", "2:00", "2:30", "3:00", "3:30", "4:00", "4:30",
                "5:00", "5:30"
            ]
        );
        let t = time_ticks(7.5, 800.0, 60.0);
        assert_eq!(t.last().unwrap().label, "0:07,5");
        let t = time_ticks(4500.0, 900.0, 80.0);
        assert!(t.iter().any(|t| t.label == "1:00:00"));
        assert_eq!(t.last().unwrap().label, "1:15:00");
        assert_eq!(fmt_time(3725.0), "1:02:05");
    }

    #[test]
    fn decibelios() {
        let t = db_ticks(-120.0, 0.0, 400.0, 30.0);
        assert_eq!(t.first().unwrap().label, "\u{2212}120 dB");
        assert_eq!(t.last().unwrap().label, "0 dB");
        assert!(t.iter().any(|t| t.label == "\u{2212}60 dB"));
        assert_eq!(t.len(), 13);
        assert_eq!(db_ticks(0.0, -120.0, 400.0, 30.0), t, "rango invertido");
    }

    #[test]
    fn nunca_se_solapan() {
        for len in [20.0, 37.0, 80.0, 150.0, 333.0, 600.0, 1234.0, 4000.0] {
            for spacing in [10.0, 25.0, 40.0, 80.0] {
                if len < spacing {
                    continue;
                }
                for nyq in [4_000.0, 11_025.0, 22_050.0, 24_000.0, 48_000.0, 96_000.0] {
                    assert_no_overlap(&freq_ticks(nyq, len, spacing), spacing, "freq");
                }
                for dur in [0.5, 7.0, 61.0, 330.0, 3600.0, 10_000.0] {
                    assert_no_overlap(&time_ticks(dur, len, spacing), spacing, "tiempo");
                }
                for lo in [-160.0, -120.0, -115.0, -40.0] {
                    assert_no_overlap(&db_ticks(lo, 0.0, len, spacing), spacing, "dB");
                }
            }
        }
    }

    #[test]
    fn casos_limite() {
        assert!(time_ticks(0.0, 800.0, 60.0).is_empty());
        assert!(time_ticks(-1.0, 800.0, 60.0).is_empty());
        assert!(time_ticks(f64::NAN, 800.0, 60.0).is_empty());
        assert!(freq_ticks(22_050.0, 0.0, 40.0).is_empty());
        // Eje diminuto: solo los extremos.
        let t = freq_ticks(22_050.0, 10.0, 40.0);
        assert_eq!(labels(&t), ["0 kHz", "22,05 kHz"]);
        // Eje corto pero no diminuto: solo el origen (el extremo final se solaparía).
        let t = freq_ticks(22_050.0, 30.0, 40.0);
        assert_eq!(labels(&t), ["0 kHz"]);
        // Nyquist no entero.
        assert_eq!(fmt_khz(5_512.5), "5,513 kHz");
        assert_eq!(fmt_khz(500.0), "0,5 kHz");
        assert_eq!(fmt_khz(0.0), "0 kHz");
        assert_eq!(fmt_db(-0.4), "0 dB");
        assert_eq!(fmt_time_tenths(83.47), "1:23,4");
    }
}
