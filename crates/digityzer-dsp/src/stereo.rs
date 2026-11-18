//! Correlación de fase y goniómetro (T2.4).
//!
//! Correlación de Pearson sin quitar la media en una ventana deslizante:
//! `Σ L·R / √(Σ L² · Σ R²)`. Mono = +1, un canal invertido = −1, canales independientes ≈ 0.

use std::collections::VecDeque;

use crate::sliding::SlidingSum;

/// Correlación de fase en una ventana deslizante.
#[derive(Debug, Clone)]
pub struct Correlation {
    lr: SlidingSum,
    ll: SlidingSum,
    rr: SlidingSum,
}

impl Correlation {
    /// Ventana de `window` muestras.
    pub fn new(window: usize) -> Self {
        Self {
            lr: SlidingSum::new(window),
            ll: SlidingSum::new(window),
            rr: SlidingSum::new(window),
        }
    }

    /// Añade una pareja de muestras normalizadas.
    #[inline]
    pub fn push(&mut self, l: f64, r: f64) {
        self.lr.push(l * r);
        self.ll.push(l * l);
        self.rr.push(r * r);
    }

    /// Correlación en [−1, 1]; `None` si algún canal está en silencio.
    pub fn value(&self) -> Option<f32> {
        let den = (self.ll.sum() * self.rr.sum()).sqrt();
        if den < 1e-12 {
            None
        } else {
            Some((self.lr_sum() / den).clamp(-1.0, 1.0) as f32)
        }
    }

    fn lr_sum(&self) -> f64 {
        self.lr.signed_sum()
    }
}

/// Puntos del goniómetro: `x = (L − R) / 2` (lado), `y = (L + R) / 2` (medio). Mono es una línea
/// vertical; un canal invertido, una horizontal.
#[derive(Debug, Clone)]
pub struct Goniometer {
    points: VecDeque<(f32, f32)>,
    capacity: usize,
    decimate: usize,
    counter: usize,
}

impl Goniometer {
    /// Guarda los últimos `capacity` puntos, uno de cada `decimate` muestras.
    pub fn new(capacity: usize, decimate: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity),
            capacity: capacity.max(1),
            decimate: decimate.max(1),
            counter: 0,
        }
    }

    /// Añade una pareja de muestras normalizadas.
    #[inline]
    pub fn push(&mut self, l: f64, r: f64) {
        self.counter += 1;
        if self.counter < self.decimate {
            return;
        }
        self.counter = 0;
        if self.points.len() == self.capacity {
            self.points.pop_front();
        }
        self.points
            .push_back((((l - r) / 2.0) as f32, ((l + r) / 2.0) as f32));
    }

    /// Puntos, del más antiguo al más reciente.
    pub fn points(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.points.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_testkit::{sine, white_noise};

    fn corr(l: &[f64], r: &[f64]) -> Option<f32> {
        let mut c = Correlation::new(4800);
        for (&a, &b) in l.iter().zip(r) {
            c.push(a, b);
        }
        c.value()
    }

    #[test]
    fn mono_invertido_e_independiente() {
        let s = sine(1000.0, 48_000, 0.5, 0.5);
        let inv: Vec<f64> = s.iter().map(|v| -v).collect();
        assert!((corr(&s, &s).unwrap() - 1.0).abs() < 1e-6);
        assert!((corr(&s, &inv).unwrap() + 1.0).abs() < 1e-6);
        let a = white_noise(48_000, 0.5, 0.5, 1);
        let b = white_noise(48_000, 0.5, 0.5, 2);
        assert!(corr(&a, &b).unwrap().abs() < 0.05);
        assert_eq!(corr(&s, &vec![0.0; s.len()]), None);
    }

    #[test]
    fn goniometro_mono_vertical() {
        let s = sine(1000.0, 48_000, 0.1, 0.5);
        let mut g = Goniometer::new(100, 4);
        for &v in &s {
            g.push(v, v);
        }
        let pts: Vec<_> = g.points().collect();
        assert_eq!(pts.len(), 100);
        assert!(pts.iter().all(|p| p.0.abs() < 1e-6));
        let mut g = Goniometer::new(100, 1);
        for &v in &s {
            g.push(v, -v);
        }
        assert!(g.points().all(|p| p.1.abs() < 1e-6));
    }
}
