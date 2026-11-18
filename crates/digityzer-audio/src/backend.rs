//! Capa de audio por la que se graba (1.1).
//!
//! - [`AudioBackend::Alsa`]: dispositivo `hw:` en exclusiva, formato nativo y frecuencia exacta.
//!   Es la única capa **bit-perfect** y la de por defecto.
//! - [`AudioBackend::PipeWire`], [`AudioBackend::Pulse`] y [`AudioBackend::Default`]: se abren
//!   con la misma API de ALSA a través de los plugins del sistema (`pipewire` de pipewire-alsa,
//!   `pulse` de alsa-plugins y el `default` de la configuración de ALSA). Graban la fuente
//!   predeterminada del servidor en estéreo; el servidor puede remuestrear o mezclar, así que el
//!   master se marca como **no bit-perfect** en su JSON.
//!
//! No se enlazan bibliotecas de PipeWire ni de PulseAudio: si el plugin no está instalado, la
//! capa sale como no disponible con lo que hay que instalar.

use std::fmt;

use alsa::Direction;
use alsa::pcm::PCM;

/// Capa de audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioBackend {
    /// ALSA directo (`hw:`), bit-perfect.
    Alsa,
    /// PipeWire (plugin `pipewire` de pipewire-alsa).
    PipeWire,
    /// PulseAudio, o pipewire-pulse (plugin `pulse` de alsa-plugins).
    Pulse,
    /// Dispositivo `default` de ALSA (lo que diga la configuración del sistema).
    Default,
}

impl AudioBackend {
    /// Todas, en el orden del selector.
    pub const ALL: [AudioBackend; 4] = [
        AudioBackend::Alsa,
        AudioBackend::PipeWire,
        AudioBackend::Pulse,
        AudioBackend::Default,
    ];

    /// Identificador estable (configuración, JSON y `--backend`).
    pub fn id(self) -> &'static str {
        match self {
            AudioBackend::Alsa => "alsa",
            AudioBackend::PipeWire => "pipewire",
            AudioBackend::Pulse => "pulse",
            AudioBackend::Default => "default",
        }
    }

    /// Desde el identificador (sin distinguir mayúsculas).
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.id().eq_ignore_ascii_case(s.trim()))
    }

    /// Nombre visible.
    pub fn label(self) -> &'static str {
        match self {
            AudioBackend::Alsa => "ALSA directo (bit-perfect)",
            AudioBackend::PipeWire => "PipeWire",
            AudioBackend::Pulse => "PulseAudio",
            AudioBackend::Default => "Predeterminado de ALSA",
        }
    }

    /// Nombre del dispositivo PCM de ALSA de las capas por plugin (`None` en ALSA directo, que
    /// usa el `hw:` elegido).
    pub fn pcm_name(self) -> Option<&'static str> {
        match self {
            AudioBackend::Alsa => None,
            AudioBackend::PipeWire => Some("pipewire"),
            AudioBackend::Pulse => Some("pulse"),
            AudioBackend::Default => Some("default"),
        }
    }

    /// `true` solo en ALSA directo: las demás pasan por un servidor o un plugin que puede
    /// convertir.
    pub fn bit_perfect(self) -> bool {
        self == AudioBackend::Alsa
    }

    /// Qué instalar si la capa no está disponible (Arch Linux).
    pub fn install_hint(self) -> &'static str {
        match self {
            AudioBackend::Alsa => "alsa-lib",
            AudioBackend::PipeWire => "pipewire y pipewire-alsa, con PipeWire en marcha",
            AudioBackend::Pulse => {
                "pulseaudio-alsa (alsa-plugins) y PulseAudio o pipewire-pulse en marcha"
            }
            AudioBackend::Default => "una configuración de ALSA con dispositivo «default»",
        }
    }

    /// Nombre de la fuente que se graba, para la interfaz y el JSON.
    pub fn source_name(self) -> &'static str {
        match self {
            AudioBackend::Alsa => "ALSA",
            AudioBackend::PipeWire => "PipeWire (fuente predeterminada)",
            AudioBackend::Pulse => "PulseAudio (fuente predeterminada)",
            AudioBackend::Default => "ALSA «default»",
        }
    }
}

impl fmt::Display for AudioBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// `true` si la capa por plugin se puede abrir para grabar ahora mismo (ALSA directo: siempre).
pub fn available(backend: AudioBackend) -> bool {
    match backend.pcm_name() {
        None => true,
        Some(name) => PCM::new(name, Direction::Capture, true).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identificadores_y_propiedades() {
        for b in AudioBackend::ALL {
            assert_eq!(AudioBackend::from_id(b.id()), Some(b));
        }
        assert_eq!(
            AudioBackend::from_id(" PipeWire "),
            Some(AudioBackend::PipeWire)
        );
        assert_eq!(AudioBackend::from_id("jack"), None);
        assert!(AudioBackend::Alsa.bit_perfect());
        assert!(!AudioBackend::PipeWire.bit_perfect());
        assert_eq!(AudioBackend::Pulse.pcm_name(), Some("pulse"));
        assert_eq!(AudioBackend::Alsa.pcm_name(), None);
        assert!(available(AudioBackend::Alsa));
    }
}
