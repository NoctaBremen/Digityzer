//! Captura ALSA bit-perfect y escritura del master.
//!
//! Llegan en la fase 1 del plan: listado de dispositivos (T1.1), motor de captura en `hw:` con
//! ring buffer sin bloqueos (T1.2), escritor WAV/RF64 con su JSON (T1.3) y el aviso de
//! dispositivo ocupado (T1.5).

/// Versión de la crate (la del workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_no_vacia() {
        assert!(!VERSION.is_empty());
    }
}
