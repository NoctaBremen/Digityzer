//! Números de coma flotante extendidos IEEE 754 de 80 bits (el `sampleRate` de `COMM`).
//!
//! Formato: 1 bit de signo + 15 de exponente (sesgo 16.383) + 64 de mantisa con el 1 entero
//! **explícito**, en big-endian. Todo `f64` finito se representa exactamente.

const BIAS: i32 = 16_383;

/// `f64` → extended de 80 bits. Exacto para cualquier `f64` finito (en particular, para todos los
/// enteros hasta 2^53). Infinito y NaN usan el exponente máximo.
pub fn f64_to_ext80(v: f64) -> [u8; 10] {
    let bits = v.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (e, mantissa): (u16, u64) = if exp == 0x7FF {
        // Infinito (fracción 0) o NaN (fracción ≠ 0, se conserva el bit de quiet NaN).
        (0x7FFF, (1u64 << 63) | (frac << 11))
    } else if exp == 0 && frac == 0 {
        (0, 0)
    } else if exp == 0 {
        // Subnormal de f64: valor = frac · 2^−1074. Se normaliza (en extended sí es normal):
        // m = frac · 2^shift con el bit 63 a 1, y m · 2^(e − sesgo − 63) = frac · 2^−1074.
        let shift = frac.leading_zeros() as i32; // frac < 2^52 → shift ≥ 12
        let m = frac << shift;
        let e = BIAS + 63 - 1074 - shift;
        (e as u16, m)
    } else {
        ((exp - 1023 + BIAS) as u16, (1u64 << 63) | (frac << 11))
    };
    let se = sign | e;
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&se.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Extended de 80 bits → `f64` (redondeando la mantisa de 64 bits a 53 si hace falta).
pub fn ext80_to_f64(b: &[u8; 10]) -> f64 {
    let se = u16::from_be_bytes([b[0], b[1]]);
    let negative = se & 0x8000 != 0;
    let exp = i32::from(se & 0x7FFF);
    let mut m = [0u8; 8];
    m.copy_from_slice(&b[2..]);
    let mantissa = u64::from_be_bytes(m);
    let magnitude = if exp == 0x7FFF {
        if mantissa << 1 == 0 {
            f64::INFINITY
        } else {
            f64::NAN
        }
    } else if mantissa == 0 {
        0.0
    } else {
        // valor = mantisa · 2^(exp − sesgo − 63)
        ldexp(mantissa as f64, exp - BIAS - 63)
    };
    if negative { -magnitude } else { magnitude }
}

/// `x · 2^n` sin desbordar en los pasos intermedios.
fn ldexp(mut x: f64, mut n: i32) -> f64 {
    const STEP: i32 = 1000;
    let up = f64::from_bits(((1023 + STEP) as u64) << 52); // 2^1000
    let down = f64::from_bits(((1023 - STEP) as u64) << 52); // 2^−1000
    while n > STEP {
        x *= up;
        n -= STEP;
        if x.is_infinite() {
            return x;
        }
    }
    while n < -STEP {
        x *= down;
        n += STEP;
        if x == 0.0 {
            return x;
        }
    }
    x * f64::from_bits(((1023 + n) as u64) << 52)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_conocido_44100() {
        assert_eq!(
            f64_to_ext80(44_100.0),
            [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            ext80_to_f64(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]),
            44_100.0
        );
        // 48.000 = 0xBB80 · 2^0 → exponente 16.383 + 15.
        assert_eq!(
            f64_to_ext80(48_000.0),
            [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(f64_to_ext80(1.0), [0x3F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(f64_to_ext80(0.0), [0; 10]);
    }

    #[test]
    fn enteros_y_frecuencias_exactos() {
        let rates = [
            8_000, 11_025, 22_050, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800,
            384_000, 705_600, 768_000,
        ];
        for r in rates {
            assert_eq!(ext80_to_f64(&f64_to_ext80(f64::from(r))), f64::from(r));
        }
        let mut x = 1u64;
        while x <= 1u64 << 53 {
            for v in [x - 1, x, x + 1] {
                let f = v as f64;
                assert_eq!(ext80_to_f64(&f64_to_ext80(f)), f, "{v}");
            }
            x <<= 1;
        }
    }

    #[test]
    fn valores_especiales_y_aleatorios() {
        for v in [
            -1.5,
            0.1,
            1e-300,
            1e300,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE / 8.0, // subnormal
            5e-324,                  // el menor subnormal
            -0.0,
        ] {
            let back = ext80_to_f64(&f64_to_ext80(v));
            assert_eq!(back.to_bits(), v.to_bits(), "{v:e}");
        }
        assert_eq!(ext80_to_f64(&f64_to_ext80(f64::INFINITY)), f64::INFINITY);
        assert!(ext80_to_f64(&f64_to_ext80(f64::NAN)).is_nan());
        // Generador congruencial simple: 10.000 bits aleatorios de f64 finitos.
        let mut s = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..10_000 {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let v = f64::from_bits(s);
            if v.is_finite() {
                assert_eq!(ext80_to_f64(&f64_to_ext80(v)).to_bits(), v.to_bits());
            }
        }
    }
}
