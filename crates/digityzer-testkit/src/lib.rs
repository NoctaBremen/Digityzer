//! Utilidades de test de Digityzer (solo `dev-dependency`).
//!
//! - [`signals`]: señales sintéticas **deterministas** (semilla fija) en `f64` mono en [−1, 1] y
//!   su cuantización a enteros.
//! - [`spectral`]: filtrado paso bajo "de acantilado" (imita un MP3), sobremuestreo y un espectro
//!   de Welch para comprobar resultados.
//! - [`wav`]: escritores de fixtures WAV en disco.
//! - [`ffmpeg`]: generación opcional de fixtures con pérdida si `ffmpeg` está en el PATH.
//! - [`corrupt`]: corrupciones deterministas de archivos para probar lectores sin pánicos.
//!
//! `signals`, `spectral`, `wav`, `ffmpeg` y `corrupt` están copiados de `spector-testkit` (ver
//! `PROVENANCE.md` de `audio-core`). Nunca se versionan archivos de audio: todo se genera al vuelo
//! en directorios temporales ([`temp_dir`]).

pub mod corrupt;
pub mod ffmpeg;
pub mod signals;
pub mod spectral;
pub mod wav;

pub use ffmpeg::{ffmpeg_available, ffmpeg_encode};
pub use signals::{
    Rng, clip, interleave, multitone, pad_to_24, pad_to_24_with_noise, quantize, silence, sine,
    white_noise,
};
pub use spectral::{lowpass_brickwall, upsample_from, welch_db};
pub use wav::{write_wav_extensible, write_wav_f32, write_wav_f64, write_wav_int};

/// Crea un directorio temporal que se borra al soltarlo.
///
/// # Panics
/// Si el sistema no permite crear directorios temporales (solo se usa en tests).
pub fn temp_dir() -> tempfile::TempDir {
    #[allow(clippy::expect_used)] // Solo tests: sin directorio temporal no hay nada que probar.
    tempfile::tempdir().expect("no se pudo crear un directorio temporal")
}

/// Sale del test actual con un aviso si `ffmpeg` no está disponible.
///
/// ```ignore
/// #[test]
/// fn mp3() {
///     digityzer_testkit::require_ffmpeg!();
///     // … el test que necesita ffmpeg …
/// }
/// ```
#[macro_export]
macro_rules! require_ffmpeg {
    () => {
        if !$crate::ffmpeg_available() {
            eprintln!("SKIP: ffmpeg no disponible");
            return;
        }
    };
}
