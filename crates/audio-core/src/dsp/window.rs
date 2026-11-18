//! Funciones de ventana **periódicas** (las adecuadas para análisis espectral con solapamiento).

use std::f64::consts::TAU;

/// Tipo de ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// Hann: buena resolución en frecuencia, lóbulos laterales a −31 dB.
    #[default]
    Hann,
    /// Blackman-Harris de 4 términos: lóbulos laterales a −92 dB, ideal para ver suelos de ruido.
    BlackmanHarris,
}

impl WindowKind {
    /// Nombre para la interfaz.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hann => "Hann",
            Self::BlackmanHarris => "Blackman-Harris",
        }
    }
}

/// Coeficientes de la ventana de longitud `n` (periódica: `w[i]` con `i/n`, no `i/(n−1)`).
pub fn window(kind: WindowKind, n: usize) -> Vec<f32> {
    let nf = n as f64;
    (0..n)
        .map(|i| {
            let x = TAU * i as f64 / nf;
            let w = match kind {
                WindowKind::Hann => 0.5 - 0.5 * x.cos(),
                WindowKind::BlackmanHarris => {
                    0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                        - 0.01168 * (3.0 * x).cos()
                }
            };
            w as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_periodica() {
        let w = window(WindowKind::Hann, 8);
        assert_eq!(w.len(), 8);
        assert!(w[0].abs() < 1e-7);
        assert!((w[4] - 1.0).abs() < 1e-7);
        // Simetría periódica: w[i] == w[n − i].
        for i in 1..8 {
            assert!((w[i] - w[8 - i]).abs() < 1e-6);
        }
        // Suma = n/2.
        let sum: f32 = window(WindowKind::Hann, 1024).iter().sum();
        assert!((sum - 512.0).abs() < 1e-2);
    }

    #[test]
    fn blackman_harris() {
        let w = window(WindowKind::BlackmanHarris, 1024);
        assert!(w[0].abs() < 1e-4, "extremo ≈ 0 ({})", w[0]);
        assert!((w[512] - 1.0).abs() < 1e-6);
        let sum: f64 = w.iter().map(|&v| f64::from(v)).sum();
        assert!((sum / 1024.0 - 0.35875).abs() < 1e-4, "media = a0");
        for i in 1..1024 {
            assert!((w[i] - w[1024 - i]).abs() < 1e-6);
        }
    }
}
