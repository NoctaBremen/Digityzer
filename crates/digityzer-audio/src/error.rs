//! Errores de captura y escritura del master. Mensajes en castellano para el usuario.

use std::path::PathBuf;

/// `errno` de dispositivo ocupado.
pub const EBUSY: i32 = 16;

/// `errno` positivo de un error de ALSA. La crate `alsa` guarda el código tal cual lo devuelve
/// la biblioteca (negativo, p. ej. −16) en los errores de sus llamadas, y positivo en otros.
pub fn errno_of(e: &alsa::Error) -> i32 {
    e.errno().abs()
}

/// Errores de `digityzer-audio`.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// El dispositivo lo tiene otro programa (T1.5). `hw:` es exclusivo.
    #[error(
        "{name} está en uso por otro programa (¿xwax abierto?). Ciérralo o graba por otro USB \
         ({device})"
    )]
    Busy {
        /// Dispositivo ALSA.
        device: String,
        /// Nombre legible de la tarjeta.
        name: String,
    },
    /// Error de ALSA.
    #[error("ALSA, {context}: {source}")]
    Alsa {
        /// Qué se estaba haciendo.
        context: String,
        /// Error original.
        source: alsa::Error,
    },
    /// La tarjeta no admite la configuración pedida sin convertir.
    #[error("{0}")]
    Unsupported(String),
    /// Parámetros incorrectos.
    #[error("{0}")]
    Invalid(String),
    /// Error de E/S.
    #[error("error de E/S en {path}: {source}")]
    Io {
        /// Archivo afectado.
        path: PathBuf,
        /// Error original.
        source: std::io::Error,
    },
    /// Error al guardar el JSON del master.
    #[error(transparent)]
    Project(#[from] digityzer_project::ProjectError),
    /// El hilo de captura o el escritor terminaron con pánico.
    #[error("el hilo de {0} terminó de forma inesperada")]
    ThreadPanicked(&'static str),
}

impl AudioError {
    /// Envuelve un error de ALSA; si es `EBUSY`, devuelve [`AudioError::Busy`].
    pub fn from_alsa(context: impl Into<String>, device: &str, name: &str, e: alsa::Error) -> Self {
        if errno_of(&e) == EBUSY {
            AudioError::Busy {
                device: device.to_string(),
                name: name.to_string(),
            }
        } else {
            AudioError::Alsa {
                context: context.into(),
                source: e,
            }
        }
    }

    /// Error de E/S con su ruta.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AudioError::Io {
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ebusy_da_un_mensaje_claro() {
        let e = AudioError::from_alsa(
            "abrir",
            "hw:CARD=USB1,DEV=0",
            "XONE 96 USB 1",
            alsa::Error::new("snd_pcm_open", -EBUSY),
        );
        assert!(matches!(e, AudioError::Busy { .. }));
        let msg = e.to_string();
        assert!(msg.starts_with("XONE 96 USB 1 está en uso"), "{msg}");
        assert!(msg.contains("xwax"));
        // Con el signo positivo también (p. ej. `alsa::Error::last`).
        let pos = AudioError::from_alsa("abrir", "hw:0", "x", alsa::Error::new("f", EBUSY));
        assert!(matches!(pos, AudioError::Busy { .. }));
        let other = AudioError::from_alsa("abrir", "hw:0", "x", alsa::Error::new("f", -2));
        assert!(matches!(other, AudioError::Alsa { .. }));
    }
}
