//! Señales sintéticas deterministas y su cuantización a enteros.
//!
//! Convenciones:
//! - Las señales son `Vec<f64>` mono con amplitud en [−1, 1] (1,0 = escala completa).
//! - El número de muestras de una duración `secs` es `round(secs · rate)`.
//! - Los enteros están **justificados a la derecha**: un 24 bits va de −8.388.608 a 8.388.607.
//! - Toda aleatoriedad usa [`Rng`] con semilla explícita: los tests son reproducibles.

use std::f64::consts::TAU;

/// Generador pseudoaleatorio xorshift64* (determinista, sin dependencias).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Crea un generador. La semilla 0 se sustituye por una constante (xorshift no admite 0).
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Siguiente valor de 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Valor uniforme en [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        // 53 bits de mantisa.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Valor uniforme en [−1, 1).
    pub fn next_signed(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }

    /// Entero uniforme en [0, 2^bits). `bits` ≤ 32.
    pub fn next_bits(&mut self, bits: u32) -> u32 {
        assert!(bits <= 32, "bits debe ser ≤ 32");
        if bits == 0 {
            return 0;
        }
        (self.next_u64() >> (64 - bits)) as u32
    }
}

/// Número de muestras de `secs` segundos a `rate` Hz.
pub fn num_samples(rate: u32, secs: f64) -> usize {
    (secs * f64::from(rate)).round().max(0.0) as usize
}

/// Seno de frecuencia `freq` Hz y amplitud `amp`, con fase inicial 0.
pub fn sine(freq: f64, rate: u32, secs: f64, amp: f64) -> Vec<f64> {
    let n = num_samples(rate, secs);
    let w = TAU * freq / f64::from(rate);
    (0..n).map(|i| amp * (w * i as f64).sin()).collect()
}

/// Suma de senos de igual amplitud. La amplitud **total** de pico no supera `amp`.
pub fn multitone(freqs: &[f64], rate: u32, secs: f64, amp: f64) -> Vec<f64> {
    let n = num_samples(rate, secs);
    if freqs.is_empty() {
        return vec![0.0; n];
    }
    let each = amp / freqs.len() as f64;
    let mut out = vec![0.0; n];
    for &f in freqs {
        let w = TAU * f / f64::from(rate);
        for (i, v) in out.iter_mut().enumerate() {
            *v += each * (w * i as f64).sin();
        }
    }
    out
}

/// Ruido blanco uniforme en [−amp, amp).
pub fn white_noise(rate: u32, secs: f64, amp: f64, seed: u64) -> Vec<f64> {
    let n = num_samples(rate, secs);
    let mut rng = Rng::new(seed);
    (0..n).map(|_| amp * rng.next_signed()).collect()
}

/// Silencio digital (ceros).
pub fn silence(rate: u32, secs: f64) -> Vec<f64> {
    vec![0.0; num_samples(rate, secs)]
}

/// Multiplica por `gain` y recorta a [−1, 1] (clipping duro).
pub fn clip(signal: &[f64], gain: f64) -> Vec<f64> {
    signal
        .iter()
        .map(|&x| (x * gain).clamp(-1.0, 1.0))
        .collect()
}

/// Valor mínimo y máximo de un entero con signo de `bits` bits (justificado a la derecha).
pub fn int_range(bits: u32) -> (i64, i64) {
    assert!((1..=32).contains(&bits), "bits debe estar entre 1 y 32");
    let half = 1i64 << (bits - 1);
    (-half, half - 1)
}

/// Cuantiza a enteros de `bits` bits: `round(x · 2^(bits−1))`, saturado al rango.
///
/// Solo existe en el testkit: el producto **nunca** cuantiza (reglas bit-perfect).
pub fn quantize(signal: &[f64], bits: u32) -> Vec<i32> {
    let (min, max) = int_range(bits);
    let scale = (1i64 << (bits - 1)) as f64;
    signal
        .iter()
        .map(|&x| {
            let v = (x * scale).round();
            // `as i64` satura con NaN → 0; luego se recorta al rango de `bits`.
            (v as i64).clamp(min, max) as i32
        })
        .collect()
}

/// Falso 24 bits: muestras de 16 bits desplazadas 8 bits a la izquierda (bits bajos a cero).
pub fn pad_to_24(samples16: &[i32]) -> Vec<i32> {
    samples16.iter().map(|&s| s << 8).collect()
}

/// Falso 24 bits "disimulado": 16 bits desplazados 8 bits y `lsb_bits` (≤ 8) bits bajos de ruido.
///
/// La máscara de bits bajos no lo detecta; sirve para probar el indicio de suelo de ruido.
pub fn pad_to_24_with_noise(samples16: &[i32], lsb_bits: u32, seed: u64) -> Vec<i32> {
    assert!(lsb_bits <= 8, "lsb_bits debe ser ≤ 8");
    let mut rng = Rng::new(seed);
    samples16
        .iter()
        .map(|&s| (s << 8) | rng.next_bits(lsb_bits) as i32)
        .collect()
}

/// Entrelaza canales de igual longitud: `[L0, R0, L1, R1, …]`.
///
/// # Panics
/// Si no hay canales o si sus longitudes difieren (error del propio test).
pub fn interleave<T: Copy>(channels: &[Vec<T>]) -> Vec<T> {
    assert!(
        !channels.is_empty(),
        "interleave: se necesita al menos un canal"
    );
    let len = channels[0].len();
    assert!(
        channels.iter().all(|c| c.len() == len),
        "interleave: los canales tienen longitudes distintas"
    );
    let mut out = Vec::with_capacity(len * channels.len());
    for i in 0..len {
        for c in channels {
            out.push(c[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_determinista() {
        let a: Vec<u64> = {
            let mut r = Rng::new(42);
            (0..5).map(|_| r.next_u64()).collect()
        };
        let b: Vec<u64> = {
            let mut r = Rng::new(42);
            (0..5).map(|_| r.next_u64()).collect()
        };
        assert_eq!(a, b);
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
        let mut r = Rng::new(7);
        assert!((0..10_000).all(|_| (0.0..1.0).contains(&r.next_f64())));
        assert!((0..10_000).all(|_| r.next_bits(8) < 256));
    }

    #[test]
    fn longitudes() {
        assert_eq!(sine(1000.0, 48_000, 1.0, 1.0).len(), 48_000);
        assert_eq!(white_noise(44_100, 0.5, 1.0, 1).len(), 22_050);
        assert!(silence(44_100, 0.0).is_empty());
        assert!(sine(1000.0, 48_000, 0.0, 1.0).is_empty());
    }

    #[test]
    fn seno_amplitud() {
        let s = sine(1000.0, 48_000, 0.1, 0.5);
        let peak = s.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!((peak - 0.5).abs() < 1e-3, "pico {peak}");
    }

    #[test]
    fn multitono_no_supera_amp() {
        let s = multitone(&[100.0, 1000.0, 5000.0], 48_000, 0.2, 0.9);
        assert!(s.iter().all(|x| x.abs() <= 0.9 + 1e-12));
    }

    #[test]
    fn cuantizacion_y_saturacion() {
        assert_eq!(
            quantize(&[0.0, 0.5, -0.5, 1.0, -1.0, 2.0, -2.0], 16),
            vec![0, 16384, -16384, 32767, -32768, 32767, -32768]
        );
        assert_eq!(quantize(&[1.0, -1.0], 24), vec![8_388_607, -8_388_608]);
        assert_eq!(quantize(&[1.0, -1.0], 32), vec![i32::MAX, i32::MIN]);
        assert_eq!(quantize(&[1.0, -1.0], 8), vec![127, -128]);
        assert_eq!(quantize(&[f64::NAN], 16), vec![0]);
    }

    #[test]
    fn falsos_24_bits() {
        let s16 = vec![1, -1, 32767, -32768];
        assert_eq!(pad_to_24(&s16), vec![256, -256, 8_388_352, -8_388_608]);
        let noisy = pad_to_24_with_noise(&s16, 8, 3);
        for (n, s) in noisy.iter().zip(&s16) {
            assert_eq!(n >> 8, *s, "los 16 bits altos se conservan");
        }
        let (min, max) = int_range(24);
        assert!(
            noisy
                .iter()
                .all(|&v| i64::from(v) >= min && i64::from(v) <= max)
        );
        assert!(pad_to_24_with_noise(&s16, 0, 3) == pad_to_24(&s16));
    }

    #[test]
    fn clipping() {
        assert_eq!(clip(&[0.3, 0.6, -0.9], 2.0), vec![0.6, 1.0, -1.0]);
    }

    #[test]
    fn entrelazado() {
        assert_eq!(
            interleave(&[vec![1, 2, 3], vec![10, 20, 30]]),
            vec![1, 10, 2, 20, 3, 30]
        );
        assert_eq!(interleave(&[vec![5, 6]]), vec![5, 6]);
    }

    #[test]
    #[should_panic(expected = "longitudes distintas")]
    fn entrelazado_longitudes_distintas() {
        let _ = interleave(&[vec![1, 2], vec![1]]);
    }
}
