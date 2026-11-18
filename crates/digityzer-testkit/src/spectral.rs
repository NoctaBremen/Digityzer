//! Procesado espectral para generar y comprobar fixtures.
//!
//! El filtrado y el sobremuestreo se hacen **en el dominio de la frecuencia sobre la señal entera**
//! (FFT → anular bins → IFFT). Comparado con el FIR de Kaiser que proponía el plan, da un corte
//! mucho más profundo (limitado solo por la precisión de `f64`) y un coste independiente de la
//! frecuencia de muestreo: un FIR con 500 Hz de transición a 192 kHz necesitaría miles de
//! coeficientes. Para fixtures de test, el resultado es el que importa: no hay contenido por
//! encima del corte.
#![allow(clippy::expect_used)] // Longitudes de FFT controladas aquí: realfft no puede fallar.

use std::f64::consts::TAU;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// Anchura del flanco de coseno alzado por debajo del corte, en Hz.
///
/// Suaviza el corte para limitar el *ringing* (fenómeno de Gibbs); por encima de `cutoff_hz`
/// la atenuación es total.
pub const BRICKWALL_TRANSITION_HZ: f64 = 100.0;

/// Paso bajo "de acantilado", como el de un codificador MP3/AAC.
///
/// Ganancia 1 hasta `cutoff_hz − 100 Hz`, flanco de coseno alzado hasta `cutoff_hz` y **cero**
/// por encima. Devuelve una señal de la misma longitud.
pub fn lowpass_brickwall(signal: &[f64], rate: u32, cutoff_hz: f64) -> Vec<f64> {
    if signal.is_empty() {
        return Vec::new();
    }
    let n = signal.len().next_power_of_two().max(2);
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);

    let mut input = fwd.make_input_vec();
    input[..signal.len()].copy_from_slice(signal);
    let mut spectrum = fwd.make_output_vec();
    fwd.process(&mut input, &mut spectrum)
        .expect("FFT directa con longitudes correctas");

    let hz_per_bin = f64::from(rate) / n as f64;
    let pass_end = (cutoff_hz - BRICKWALL_TRANSITION_HZ).max(0.0);
    for (k, bin) in spectrum.iter_mut().enumerate() {
        let f = k as f64 * hz_per_bin;
        let gain = if f <= pass_end {
            1.0
        } else if f >= cutoff_hz {
            0.0
        } else {
            let t = (f - pass_end) / (cutoff_hz - pass_end);
            0.5 * (1.0 + (std::f64::consts::PI * t).cos())
        };
        *bin *= gain / n as f64;
    }
    spectrum[0].im = 0.0;
    if let Some(last) = spectrum.last_mut() {
        last.im = 0.0;
    }

    let mut out = inv.make_output_vec();
    inv.process(&mut spectrum, &mut out)
        .expect("FFT inversa con longitudes correctas");
    out.truncate(signal.len());
    out
}

/// Sobremuestreo de banda limitada de `from_rate` a `to_rate` (≥ `from_rate`), con cualquier
/// relación (p. ej. 44.100 → 96.000).
///
/// Interpolación ideal por relleno de ceros en el espectro: el resultado **no tiene contenido**
/// por encima de `from_rate / 2`. Longitud de salida: `round(len · to_rate / from_rate)`.
///
/// # Panics
/// Si `to_rate < from_rate` o alguna frecuencia es 0.
pub fn upsample_from(signal: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
    assert!(
        from_rate > 0 && to_rate > 0,
        "frecuencias de muestreo nulas"
    );
    assert!(to_rate >= from_rate, "upsample_from solo sobremuestrea");
    if signal.is_empty() {
        return Vec::new();
    }
    let g = gcd(from_rate, to_rate);
    let (p, q) = ((from_rate / g) as usize, (to_rate / g) as usize);
    // Longitudes N2 = p·k y M2 = q·k, con k potencia de 2 (≥ 2, para que sean pares).
    let k = signal.len().div_ceil(p).next_power_of_two().max(2);
    let (n2, m2) = (p * k, q * k);

    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n2);
    let inv = planner.plan_fft_inverse(m2);

    let mut input = fwd.make_input_vec();
    input[..signal.len()].copy_from_slice(signal);
    let mut spectrum = fwd.make_output_vec();
    fwd.process(&mut input, &mut spectrum)
        .expect("FFT directa con longitudes correctas");

    let mut up = inv.make_input_vec();
    // Se copian los bins por debajo del Nyquist original; el propio bin de Nyquist se anula
    // (no tiene una representación simétrica única tras el sobremuestreo).
    let copy = n2 / 2;
    for (dst, src) in up.iter_mut().zip(spectrum.iter()).take(copy) {
        *dst = *src / n2 as f64;
    }
    up[0].im = 0.0;
    if let Some(last) = up.last_mut() {
        *last = Complex::new(0.0, 0.0);
    }

    let mut out = inv.make_output_vec();
    inv.process(&mut up, &mut out)
        .expect("FFT inversa con longitudes correctas");
    let out_len = (signal.len() as f64 * q as f64 / p as f64).round() as usize;
    out.truncate(out_len);
    out
}

/// Espectro de potencia medio (método de Welch: ventana Hann periódica, solapamiento del 50 %),
/// en dB, con `fft_size / 2 + 1` bins.
///
/// Escala: un seno de amplitud 1 da 0 dB en su bin. Bin `k` ↔ `k · rate / fft_size` Hz.
///
/// # Panics
/// Si `fft_size` < 2.
pub fn welch_db(signal: &[f64], fft_size: usize) -> Vec<f64> {
    assert!(fft_size >= 2, "fft_size debe ser ≥ 2");
    let window: Vec<f64> = (0..fft_size)
        .map(|i| 0.5 - 0.5 * (TAU * i as f64 / fft_size as f64).cos())
        .collect();
    let wsum: f64 = window.iter().sum();
    let scale = (2.0 / wsum).powi(2);

    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(fft_size);
    let mut input = fwd.make_input_vec();
    let mut spectrum = fwd.make_output_vec();
    let mut acc = vec![0.0f64; fft_size / 2 + 1];
    let hop = fft_size / 2;

    let mut frames = 0usize;
    let mut start = 0usize;
    loop {
        input.iter_mut().for_each(|v| *v = 0.0);
        let end = (start + fft_size).min(signal.len());
        if start < end {
            for (i, &x) in signal[start..end].iter().enumerate() {
                input[i] = x * window[i];
            }
        }
        fwd.process(&mut input, &mut spectrum)
            .expect("FFT directa con longitudes correctas");
        for (a, c) in acc.iter_mut().zip(&spectrum) {
            *a += c.norm_sqr() * scale;
        }
        frames += 1;
        start += hop;
        if start + fft_size > signal.len() {
            break;
        }
    }
    acc.iter()
        .map(|&p| 10.0 * (p / frames as f64 + 1e-30).log10())
        .collect()
}

/// Potencia media (en dB) de los bins de `spectrum_db` entre `lo_hz` y `hi_hz`.
///
/// La media se hace en potencia lineal. Devuelve −300 dB si el intervalo no contiene bins.
pub fn band_mean_db(
    spectrum_db: &[f64],
    rate: u32,
    fft_size: usize,
    lo_hz: f64,
    hi_hz: f64,
) -> f64 {
    let hz_per_bin = f64::from(rate) / fft_size as f64;
    let (mut sum, mut count) = (0.0, 0usize);
    for (k, &db) in spectrum_db.iter().enumerate() {
        let f = k as f64 * hz_per_bin;
        if f >= lo_hz && f <= hi_hz {
            sum += 10f64.powf(db / 10.0);
            count += 1;
        }
    }
    if count == 0 {
        -300.0
    } else {
        10.0 * (sum / count as f64 + 1e-30).log10()
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{sine, white_noise};

    const FFT: usize = 8192;

    #[test]
    fn welch_escala_dbfs() {
        let rate = 48_000;
        // 1.500 Hz cae justo en un bin con FFT de 8.192 a 48 kHz (bin 256).
        let s = sine(1500.0, rate, 1.0, 1.0);
        let spec = welch_db(&s, FFT);
        let k = (1500.0 * FFT as f64 / f64::from(rate)).round() as usize;
        assert!(
            spec[k].abs() < 0.1,
            "seno a escala completa = {} dB",
            spec[k]
        );
        let s = sine(1500.0, rate, 1.0, 0.1);
        let spec = welch_db(&s, FFT);
        assert!((spec[k] + 20.0).abs() < 0.1, "seno de 0,1 = {} dB", spec[k]);
    }

    #[test]
    fn brickwall_atenua_80_db_a_500_hz() {
        for (rate, cutoff) in [
            (44_100u32, 16_000.0),
            (48_000, 19_000.0),
            (96_000, 16_000.0),
        ] {
            let noise = white_noise(rate, 4.0, 0.5, 11);
            let filtered = lowpass_brickwall(&noise, rate, cutoff);
            assert_eq!(filtered.len(), noise.len());
            let spec = welch_db(&filtered, FFT);
            let pass = band_mean_db(&spec, rate, FFT, 1_000.0, cutoff - 1_000.0);
            let stop = band_mean_db(&spec, rate, FFT, cutoff + 500.0, f64::from(rate) / 2.0);
            assert!(
                pass - stop >= 80.0,
                "{rate} Hz, corte {cutoff}: banda pasante {pass:.1} dB, banda eliminada {stop:.1} dB"
            );
            // La banda pasante conserva el nivel del ruido original (±1 dB).
            let orig = band_mean_db(&welch_db(&noise, FFT), rate, FFT, 1_000.0, cutoff - 1_000.0);
            assert!(
                (pass - orig).abs() < 1.0,
                "pasante {pass:.2} vs original {orig:.2}"
            );
        }
    }

    #[test]
    fn sobremuestreo_sin_contenido_por_encima_del_nyquist_original() {
        for (from, to) in [
            (44_100u32, 96_000u32),
            (44_100, 88_200),
            (48_000, 96_000),
            (48_000, 192_000),
        ] {
            let noise = white_noise(from, 3.0, 0.5, 5);
            let up = upsample_from(&noise, from, to);
            let expected = (noise.len() as f64 * f64::from(to) / f64::from(from)).round() as usize;
            assert_eq!(up.len(), expected);
            let spec = welch_db(&up, FFT);
            let nyq = f64::from(from) / 2.0;
            let pass = band_mean_db(&spec, to, FFT, 1_000.0, nyq - 1_000.0);
            let stop = band_mean_db(&spec, to, FFT, nyq + 500.0, f64::from(to) / 2.0 - 500.0);
            assert!(
                pass - stop >= 90.0,
                "{from}→{to}: {pass:.1} vs {stop:.1} dB"
            );
        }
    }

    #[test]
    fn sobremuestreo_conserva_un_seno() {
        let s = sine(1000.0, 44_100, 1.0, 0.5);
        let up = upsample_from(&s, 44_100, 96_000);
        // Comparación con el seno ideal a 96 kHz, lejos de los bordes.
        let ideal = sine(1000.0, 96_000, 1.0, 0.5);
        let err = up[20_000..70_000]
            .iter()
            .zip(&ideal[20_000..70_000])
            .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
        assert!(err < 1e-3, "error máximo {err}");
    }

    #[test]
    fn vacios() {
        assert!(lowpass_brickwall(&[], 44_100, 16_000.0).is_empty());
        assert!(upsample_from(&[], 44_100, 96_000).is_empty());
        assert_eq!(welch_db(&[], 1024).len(), 513);
        assert_eq!(gcd(44_100, 96_000), 300);
    }
}
