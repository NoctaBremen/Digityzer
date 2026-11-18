//! Medidores (pico, RMS, correlación) y medidas de la cadena mecánica: velocidad y wow & flutter,
//! diafonía, balance, zumbido de masa y comparación de capturas (fases 2 y 3 del plan).
//!
//! El espectro y la detección de cortes vienen de `audio-core`.

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
