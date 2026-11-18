//! Proyecto no destructivo: JSON del master (dispositivo, formato, xruns, notas del disco),
//! marcadores de pistas y metadatos (fases 1, 4 y 5 del plan). El master nunca se reescribe.

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
