//! Escritor y lector AIFF independientes (copiados de la crate `spector-aiff`), más la conversión
//! a extended de 80 bits. Solo usa `std`.
//!
//! - [`AiffWriter`]: PCM entero de 1 a 32 bits, en streaming, sin truncar nunca valores fuera de
//!   rango.
//! - [`AiffReader`]: AIFF y AIFF-C (`NONE`/`twos`/`sowt`), para verificar la salida con funciones
//!   distintas de las del escritor.

pub mod error;
pub mod extended;
pub mod reader;
pub mod writer;

pub use error::AiffError;
pub use extended::{ext80_to_f64, f64_to_ext80};
pub use reader::{AiffReader, ChunkInfo, Compression, frames_per_buffer};
pub use writer::{AiffSpec, AiffWriter, HEADER_LEN};
