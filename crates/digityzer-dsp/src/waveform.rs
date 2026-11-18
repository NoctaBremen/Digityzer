//! Forma de onda de la grabación en curso (T2.5): mínimo y máximo de cada columna de tiempo.

use std::collections::VecDeque;

/// Columnas `(mín, máx)` de la forma de onda, la más reciente al final.
#[derive(Debug, Clone)]
pub struct Waveform {
    per_column: usize,
    count: usize,
    min: f32,
    max: f32,
    columns: VecDeque<(f32, f32)>,
    capacity: usize,
}

impl Waveform {
    /// Una columna cada `per_column` muestras; se guardan las últimas `capacity`.
    pub fn new(per_column: usize, capacity: usize) -> Self {
        Self {
            per_column: per_column.max(1),
            count: 0,
            min: f32::MAX,
            max: f32::MIN,
            columns: VecDeque::with_capacity(capacity),
            capacity: capacity.max(1),
        }
    }

    /// Añade una muestra normalizada.
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.min = self.min.min(x);
        self.max = self.max.max(x);
        self.count += 1;
        if self.count == self.per_column {
            if self.columns.len() == self.capacity {
                self.columns.pop_front();
            }
            self.columns.push_back((self.min, self.max));
            self.count = 0;
            self.min = f32::MAX;
            self.max = f32::MIN;
        }
    }

    /// Columnas completas.
    pub fn columns(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.columns.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columnas_y_desplazamiento() {
        let mut w = Waveform::new(4, 3);
        for x in [
            0.1, -0.5, 0.3, 0.0, 0.9, 0.2, -0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.7, 0.7,
        ] {
            w.push(x);
        }
        let c: Vec<_> = w.columns().collect();
        assert_eq!(c, vec![(-0.5, 0.3), (-0.1, 0.9), (0.0, 0.0)]);
        w.push(0.7);
        w.push(0.7);
        assert_eq!(w.columns().next(), Some((-0.1, 0.9)));
    }
}
