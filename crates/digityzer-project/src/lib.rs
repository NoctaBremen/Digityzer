//! Proyecto no destructivo: JSON del master (dispositivo, formato, xruns, huella, notas del
//! disco), y más adelante marcadores de pistas y metadatos (fases 4 y 5). El master nunca se
//! reescribe.

pub mod master;
pub mod time;

pub use master::{
    AudioHash, MASTER_INFO_VERSION, MasterInfo, PartialInfo, ProjectError, XrunRecord,
    set_read_only, unix_now, write_atomic,
};
pub use time::utc_iso8601;

/// Versión de la crate (la del workspace).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
