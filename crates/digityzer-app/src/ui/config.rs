//! Configuración de la interfaz en `~/.config/digityzer/config.toml` (T2.9e): tema, vista del
//! espectro y suavizado. Se escribe de forma atómica; si falta o está dañada, valores por
//! defecto.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Configuración guardada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Identificador del tema (`dracula`, `nord`…).
    pub theme: String,
    /// Vista del espectro (`curva`, `barras`, `cascada`).
    pub spectrum_view: String,
    /// Suavizado en fracciones de octava (3, 6 o 12).
    pub smoothing: u32,
    /// Capa de audio (`alsa`, `pipewire`, `pulse` o `default`).
    pub audio_backend: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "dracula".into(),
            spectrum_view: "curva".into(),
            smoothing: 6,
            audio_backend: "alsa".into(),
        }
    }
}

/// Ruta del archivo: `$XDG_CONFIG_HOME/digityzer/config.toml` o
/// `~/.config/digityzer/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("digityzer").join("config.toml"))
}

impl Config {
    /// Lee la configuración de `path`; si no existe o no se entiende, la de por defecto.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("configuración ilegible en {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Lee la configuración del usuario.
    pub fn load() -> Self {
        config_path().map_or_else(Self::default, |p| Self::load_from(&p))
    }

    /// Guarda en `path` de forma atómica.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        audio_core::fsutil::write_atomic(path, text.as_bytes())
    }

    /// Guarda la configuración del usuario; los errores solo se registran.
    pub fn save(&self) {
        if let Some(p) = config_path()
            && let Err(e) = self.save_to(&p)
        {
            tracing::warn!("no se pudo guardar {}: {e}", p.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta_y_valores_por_defecto() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("digityzer").join("config.toml");
        assert_eq!(Config::load_from(&p), Config::default());
        let c = Config {
            theme: "nord".into(),
            spectrum_view: "cascada".into(),
            smoothing: 12,
            audio_backend: "pipewire".into(),
        };
        c.save_to(&p).unwrap();
        assert_eq!(Config::load_from(&p), c);
        std::fs::write(&p, "theme = 3\n").unwrap();
        assert_eq!(Config::load_from(&p), Config::default());
        std::fs::write(&p, "theme = \"gruvbox-dark\"\n").unwrap();
        assert_eq!(Config::load_from(&p).theme, "gruvbox-dark");
        assert_eq!(Config::load_from(&p).smoothing, 6);
    }
}
