//! Errores de `spector-aiff` (sin dependencias: `Display` y `Error` escritos a mano).

use std::fmt;
use std::io;

/// Error al escribir o leer un AIFF.
#[derive(Debug)]
pub enum AiffError {
    /// Error de entrada/salida.
    Io(io::Error),
    /// Parámetros no válidos para el escritor (bits, canales o frecuencia).
    InvalidSpec(String),
    /// Una muestra no cabe en la profundidad declarada. Nunca se trunca en silencio.
    SampleOutOfRange {
        /// Valor recibido.
        value: i32,
        /// Profundidad declarada.
        bits: u16,
    },
    /// El número de muestras no es múltiplo del número de canales.
    BadLength {
        /// Muestras recibidas.
        samples: usize,
        /// Canales.
        channels: u16,
    },
    /// El archivo superaría los 4 GiB que permiten los tamaños de 32 bits.
    TooLarge,
    /// Estructura no válida (falta un chunk, tamaños incoherentes…).
    Malformed(String),
    /// Variante no admitida (compresión AIFF-C distinta de `NONE`/`twos`/`sowt`, float…).
    Unsupported(String),
    /// Se acabó el archivo antes de los datos declarados.
    Truncated,
}

impl fmt::Display for AiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "error de E/S: {e}"),
            Self::InvalidSpec(m) => write!(f, "parámetros AIFF no válidos: {m}"),
            Self::SampleOutOfRange { value, bits } => {
                write!(f, "la muestra {value} no cabe en {bits} bits")
            }
            Self::BadLength { samples, channels } => {
                write!(f, "{samples} muestras no es múltiplo de {channels} canales")
            }
            Self::TooLarge => write!(f, "el AIFF superaría el límite de 4 GiB"),
            Self::Malformed(m) => write!(f, "AIFF mal formado: {m}"),
            Self::Unsupported(m) => write!(f, "AIFF no admitido: {m}"),
            Self::Truncated => write!(f, "el AIFF está truncado"),
        }
    }
}

impl std::error::Error for AiffError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for AiffError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(e)
        }
    }
}
