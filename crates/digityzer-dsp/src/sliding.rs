//! Suma deslizante de una ventana fija, con recálculo periódico para que el error de coma
//! flotante no se acumule en grabaciones largas.

/// Suma de los últimos `len` valores.
#[derive(Debug, Clone)]
pub struct SlidingSum {
    ring: Vec<f64>,
    pos: usize,
    filled: usize,
    sum: f64,
    since_recompute: usize,
}

impl SlidingSum {
    /// Ventana de `len` valores (mínimo 1).
    pub fn new(len: usize) -> Self {
        Self {
            ring: vec![0.0; len.max(1)],
            pos: 0,
            filled: 0,
            sum: 0.0,
            since_recompute: 0,
        }
    }

    /// Añade un valor (sale el más antiguo si la ventana está llena).
    pub fn push(&mut self, v: f64) {
        let old = self.ring[self.pos];
        self.ring[self.pos] = v;
        self.pos = (self.pos + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
        self.sum += v - old;
        self.since_recompute += 1;
        if self.since_recompute >= self.ring.len() {
            self.sum = self.ring.iter().sum();
            self.since_recompute = 0;
        }
    }

    /// Suma de la ventana para valores no negativos (cuadrados): nunca negativa por redondeo.
    pub fn sum(&self) -> f64 {
        self.sum.max(0.0)
    }

    /// Suma de la ventana con signo (para productos que pueden ser negativos).
    pub fn signed_sum(&self) -> f64 {
        self.sum
    }

    /// Valores que hay en la ventana (hasta `len`).
    pub fn filled(&self) -> usize {
        self.filled
    }

    /// Vacía la ventana.
    pub fn clear(&mut self) {
        self.ring.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
        self.filled = 0;
        self.sum = 0.0;
        self.since_recompute = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ventana_deslizante() {
        let mut s = SlidingSum::new(3);
        for v in [1.0, 2.0, 3.0, 4.0] {
            s.push(v);
        }
        assert_eq!(s.sum(), 9.0);
        assert_eq!(s.filled(), 3);
        s.clear();
        assert_eq!((s.sum(), s.filled()), (0.0, 0));
    }
}
