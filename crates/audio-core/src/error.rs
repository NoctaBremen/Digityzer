//! Errores del núcleo.

use std::io;

/// Error de las operaciones del núcleo. Los mensajes están en castellano porque llegan al usuario.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Error de entrada/salida (abrir o leer el archivo).
    #[error("error de E/S: {0}")]
    Io(#[from] io::Error),
    /// Formato, códec o parámetro no soportado.
    #[error("no soportado: {0}")]
    Unsupported(String),
    /// Datos de audio dañados o incoherentes.
    #[error("error de decodificación: {0}")]
    Decode(String),
    /// La operación se canceló.
    #[error("cancelado")]
    Cancelled,
    /// El archivo está vacío.
    #[error("archivo vacío")]
    Empty,
}

/// Errores de las etiquetas ID3 del AIFF (en Spector eran la variante `Tags` de `ConvertError`).
#[derive(Debug, thiserror::Error)]
pub enum TagError {
    /// Error de E/S.
    #[error("error de E/S: {0}")]
    Io(#[from] io::Error),
    /// Etiquetas que no se pueden escribir.
    #[error("etiquetas: {0}")]
    Tags(String),
}
