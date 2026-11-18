//! Señales sintéticas propias de Digityzer para los tests de DSP: tonos modulados (velocidad,
//! wow & flutter, nivel), clics, zumbido de masa, diafonía y ruido rosa como ruido de surco.
//!
//! Mismas convenciones que [`crate::signals`]: `Vec<f64>` mono, 1,0 = escala completa, número de
//! muestras `round(secs · rate)` y aleatoriedad con semilla explícita.

use std::f64::consts::TAU;

use crate::signals::{Rng, num_samples, white_noise};

/// Convierte dB a factor de amplitud (`10^(dB/20)`).
pub fn db_to_amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Tono de `carrier_hz` reproducido con un error de velocidad constante `speed_error`
/// (fracción: `0.001` = +0,1 %). Imita un plato que gira algo rápido o lento.
pub fn speed_tone(carrier_hz: f64, rate: u32, secs: f64, amp: f64, speed_error: f64) -> Vec<f64> {
    crate::signals::sine(carrier_hz * (1.0 + speed_error), rate, secs, amp)
}

/// Tono con modulación de frecuencia sinusoidal: imita el wow & flutter de un plato.
///
/// La frecuencia instantánea es `f(t) = carrier_hz · (1 + deviation · sin(2π · mod_hz · t))`, con
/// `deviation` como fracción de pico (`0.001` = 0,1 %). La fase se integra de forma exacta, así
/// que no acumula error numérico en grabaciones largas.
///
/// # Panics
/// Si `mod_hz` no es positivo.
pub fn fm_tone(
    carrier_hz: f64,
    rate: u32,
    secs: f64,
    amp: f64,
    mod_hz: f64,
    deviation: f64,
) -> Vec<f64> {
    assert!(mod_hz > 0.0, "mod_hz debe ser positivo");
    let r = f64::from(rate);
    let beta = carrier_hz * deviation / mod_hz;
    (0..num_samples(rate, secs))
        .map(|i| {
            let t = i as f64 / r;
            // φ(t) = 2π·∫f = 2π·fc·t + (fc·d / fm)·(1 − cos(2π·fm·t)), con φ(0) = 0.
            let phase = TAU * carrier_hz * t - beta * ((TAU * mod_hz * t).cos() - 1.0);
            amp * phase.sin()
        })
        .collect()
}

/// Tono con modulación de amplitud: `amp · (1 + depth · sin(2π · mod_hz · t)) · sin(2π · fc · t)`.
/// Imita una variación de nivel (excentricidad, VTA mal ajustado). `depth` en [0, 1].
pub fn am_tone(
    carrier_hz: f64,
    rate: u32,
    secs: f64,
    amp: f64,
    mod_hz: f64,
    depth: f64,
) -> Vec<f64> {
    let r = f64::from(rate);
    (0..num_samples(rate, secs))
        .map(|i| {
            let t = i as f64 / r;
            amp * (1.0 + depth * (TAU * mod_hz * t).sin()) * (TAU * carrier_hz * t).sin()
        })
        .collect()
}

/// Posición de un clic añadido por [`add_clicks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClickSpan {
    /// Primera muestra afectada.
    pub start: usize,
    /// Número de muestras afectadas.
    pub len: usize,
}

impl ClickSpan {
    /// `true` si la muestra `i` está dentro del clic.
    pub fn contains(&self, i: usize) -> bool {
        (self.start..self.start + self.len).contains(&i)
    }
}

/// Duración de un clic sintético en muestras: 0,5 ms (mínimo 4 muestras).
pub fn click_len(rate: u32) -> usize {
    (rate as usize / 2000).max(4)
}

/// Suma a `signal` `count` clics deterministas y devuelve dónde están, en orden.
///
/// Cada clic es un impulso de [`click_len`] muestras con signo aleatorio, pico `amp` y caída
/// exponencial, como el de una mota de polvo. La señal se divide en `count` tramos iguales y cada
/// clic cae en la mitad central de su tramo, así que nunca se solapan. Fuera de los clics la
/// señal no cambia ni una muestra. No se recorta: elige `amp` para no pasar de la escala completa.
pub fn add_clicks(
    signal: &mut [f64],
    rate: u32,
    count: usize,
    amp: f64,
    seed: u64,
) -> Vec<ClickSpan> {
    let len = click_len(rate);
    if count == 0 || signal.len() < count * len * 4 {
        return Vec::new();
    }
    let mut rng = Rng::new(seed);
    let slot = signal.len() / count;
    let tau = len as f64 / 4.0;
    (0..count)
        .map(|k| {
            let room = slot / 2 - len;
            let start = k * slot + slot / 4 + (rng.next_f64() * room as f64) as usize;
            let sign = if rng.next_u64() & 1 == 0 { 1.0 } else { -1.0 };
            for (j, s) in signal[start..start + len].iter_mut().enumerate() {
                *s += sign * amp * (-(j as f64) / tau).exp();
            }
            ClickSpan { start, len }
        })
        .collect()
}

/// Zumbido de masa: fundamental `fundamental_hz` (50 Hz en Europa) y sus armónicos.
/// `levels_dbfs[k]` es el pico del armónico `k + 1` en dBFS; `None` lo omite.
pub fn hum(rate: u32, secs: f64, fundamental_hz: f64, levels_dbfs: &[Option<f64>]) -> Vec<f64> {
    let r = f64::from(rate);
    let parts: Vec<(f64, f64)> = levels_dbfs
        .iter()
        .enumerate()
        .filter_map(|(k, db)| db.map(|db| ((k + 1) as f64 * fundamental_hz, db_to_amp(db))))
        .collect();
    (0..num_samples(rate, secs))
        .map(|i| {
            let t = i as f64 / r;
            parts.iter().map(|&(f, a)| a * (TAU * f * t).sin()).sum()
        })
        .collect()
}

/// Lo que se filtra de un canal al otro con una diafonía de `leak_db` (negativo, p. ej. −30 dB).
/// Para un par L/R con diafonía L→R: `L = signal`, `R = leak(&signal, −30.0)`.
pub fn leak(signal: &[f64], leak_db: f64) -> Vec<f64> {
    let g = db_to_amp(leak_db);
    signal.iter().map(|&s| s * g).collect()
}

/// Suma muestra a muestra dos señales (la más corta se rellena con ceros).
pub fn mix(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).copied().unwrap_or(0.0) + b.get(i).copied().unwrap_or(0.0))
        .collect()
}

/// Ruido rosa (−3 dB/octava) con valor eficaz `rms`: imita el ruido de surco. Filtro de Paul
/// Kellet sobre ruido blanco determinista.
pub fn pink_noise(rate: u32, secs: f64, rms: f64, seed: u64) -> Vec<f64> {
    let white = white_noise(rate, secs, 1.0, seed);
    let mut b = [0.0f64; 7];
    let mut out: Vec<f64> = white
        .iter()
        .map(|&w| {
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let y = b.iter().sum::<f64>() + w * 0.5362;
            b[6] = w * 0.115926;
            y
        })
        .collect();
    let current = (out.iter().map(|v| v * v).sum::<f64>() / out.len().max(1) as f64).sqrt();
    if current > 0.0 {
        let g = rms / current;
        out.iter_mut().for_each(|v| *v *= g);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::welch_db;

    /// Frecuencia instantánea por cruces por cero ascendentes con interpolación lineal:
    /// (instante, frecuencia) de cada período.
    fn periods(signal: &[f64], rate: u32) -> Vec<(f64, f64)> {
        let r = f64::from(rate);
        let mut crossings = Vec::new();
        for i in 1..signal.len() {
            let (a, b) = (signal[i - 1], signal[i]);
            if a < 0.0 && b >= 0.0 {
                crossings.push((i - 1) as f64 + a / (a - b));
            }
        }
        crossings
            .windows(2)
            .map(|w| (w[0] / r, r / (w[1] - w[0])))
            .collect()
    }

    #[test]
    fn velocidad_constante() {
        let s = speed_tone(3150.0, 96_000, 1.0, 0.5, 0.005);
        let p = periods(&s, 96_000);
        let mean = p.iter().map(|x| x.1).sum::<f64>() / p.len() as f64;
        assert!((mean / 3150.0 - 1.005).abs() < 1e-5, "{mean}");
    }

    #[test]
    fn fm_desviacion_y_frecuencia_media() {
        // 3150 Hz, wow de 0,55 Hz (33⅓ rpm) al 0,10 % de pico.
        let s = fm_tone(3150.0, 96_000, 4.0, 0.5, 0.55, 0.001);
        let p = periods(&s, 96_000);
        let mean = p.iter().map(|x| x.1).sum::<f64>() / p.len() as f64;
        let max = p.iter().map(|x| x.1).fold(f64::MIN, f64::max);
        let min = p.iter().map(|x| x.1).fold(f64::MAX, f64::min);
        assert!((mean - 3150.0).abs() < 0.5, "media {mean}");
        let dev = (max - min) / 2.0 / 3150.0;
        assert!((dev - 0.001).abs() < 0.00005, "desviación {dev}");
        // Continuidad de fase: ningún salto entre muestras mayor que el de la frecuencia máxima.
        let step = TAU * 3150.0 * 1.001 / 96_000.0;
        assert!(
            s.windows(2)
                .all(|w| (w[1] - w[0]).abs() <= 0.5 * step * 1.01)
        );
    }

    #[test]
    fn am_envolvente() {
        let s = am_tone(1000.0, 48_000, 2.0, 0.5, 2.0, 0.2);
        let peak = s.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!((peak - 0.6).abs() < 1e-3, "{peak}");
    }

    #[test]
    fn clics_deterministas_y_sin_tocar_el_resto() {
        let clean = crate::signals::sine(1000.0, 96_000, 2.0, 0.3);
        let mut a = clean.clone();
        let spans = add_clicks(&mut a, 96_000, 10, 0.5, 7);
        assert_eq!(spans.len(), 10);
        assert!(spans.windows(2).all(|w| w[0].start + w[0].len < w[1].start));
        for (i, (x, y)) in a.iter().zip(&clean).enumerate() {
            let inside = spans.iter().any(|c| c.contains(i));
            if inside {
                continue;
            }
            assert_eq!(x, y, "muestra {i} fuera de un clic");
        }
        for c in &spans {
            assert!((a[c.start] - clean[c.start]).abs() > 0.49);
        }
        let mut b = clean.clone();
        assert_eq!(add_clicks(&mut b, 96_000, 10, 0.5, 7), spans);
        assert_eq!(a, b);
    }

    #[test]
    fn zumbido_en_sus_armonicos_y_a_su_nivel() {
        // Bins de 1 Hz: 50, 100 y 150 Hz caen en el centro de su bin.
        let s = hum(
            48_000,
            4.0,
            50.0,
            &[Some(-70.0), Some(-76.0), None, Some(-80.0)],
        );
        let db = welch_db(&s, 48_000);
        assert!((db[50] + 70.0).abs() < 0.5, "{}", db[50]);
        assert!((db[100] + 76.0).abs() < 0.5, "{}", db[100]);
        assert!(db[150] < -120.0, "{}", db[150]);
        assert!((db[200] + 80.0).abs() < 0.5, "{}", db[200]);
    }

    #[test]
    fn diafonia_de_menos_30_db() {
        let l = crate::signals::sine(1000.0, 48_000, 1.0, 0.5);
        let r = leak(&l, -30.0);
        let ratio = 20.0 * (r[100] / l[100]).abs().log10();
        assert!((ratio + 30.0).abs() < 1e-9);
        assert_eq!(mix(&[1.0, 2.0], &[0.5]), vec![1.5, 2.0]);
    }

    #[test]
    fn ruido_rosa_baja_3_db_por_octava() {
        let s = pink_noise(48_000, 8.0, 0.05, 3);
        let rms = (s.iter().map(|v| v * v).sum::<f64>() / s.len() as f64).sqrt();
        assert!((rms - 0.05).abs() < 1e-9);
        let db = welch_db(&s, 4096);
        let band = |lo: f64, hi: f64| crate::spectral::band_mean_db(&db, 48_000, 4096, lo, hi);
        // Cuatro octavas (500 Hz → 8 kHz): unos −12 dB.
        let drop = band(7_000.0, 9_000.0) - band(450.0, 550.0);
        assert!((drop + 12.0).abs() < 1.5, "{drop}");
    }
}
