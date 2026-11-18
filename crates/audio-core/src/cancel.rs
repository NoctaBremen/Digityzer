//! Cancelación cooperativa e informe de progreso.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Señal de cancelación compartida entre la interfaz y los trabajos.
///
/// Los trabajos la comprueban en cada bloque decodificado y antes de cualquier operación
/// irreversible. Clonarla comparte la misma señal.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Crea una señal sin cancelar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pide la cancelación (idempotente).
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// `true` si se ha pedido la cancelación.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Receptor del progreso de un trabajo.
pub trait Progress: Sync {
    /// Informa del progreso, con `fraction` en [0, 1].
    fn report(&self, fraction: f32);
}

/// Implementación que descarta el progreso.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _fraction: f32) {}
}

impl<F: Fn(f32) + Sync> Progress for F {
    fn report(&self, fraction: f32) {
        self(fraction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelacion_compartida() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn progreso_con_closure() {
        let last = std::sync::Mutex::new(0.0f32);
        let p = |f: f32| *last.lock().unwrap() = f;
        p.report(0.5);
        NoProgress.report(1.0);
        assert_eq!(*last.lock().unwrap(), 0.5);
    }
}
