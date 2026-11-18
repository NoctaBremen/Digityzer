//! JSON que acompaña a cada master (T1.3): con qué se grabó, qué pasó durante la grabación y la
//! huella del audio para comprobar más tarde que el archivo no ha cambiado.
//!
//! Se escribe **una sola vez**, al cerrar la grabación, con `create_new` y se deja en solo lectura.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Versión del formato del JSON (2: capa de audio y `bit_perfect`, versión 1.1; 3: grabación
/// interrumpida o recuperada, T1.6).
pub const MASTER_INFO_VERSION: u32 = 3;

fn default_backend() -> String {
    "alsa".into()
}

fn default_true() -> bool {
    true
}

/// Errores al guardar o leer el JSON del master.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// Error de E/S.
    #[error("error de E/S en {path}: {source}")]
    Io {
        /// Archivo afectado.
        path: PathBuf,
        /// Error original.
        source: std::io::Error,
    },
    /// JSON inválido.
    #[error("JSON inválido en {path}: {source}")]
    Json {
        /// Archivo afectado.
        path: PathBuf,
        /// Error original.
        source: serde_json::Error,
    },
}

/// Un xrun (desbordamiento del búfer de captura de ALSA): se han perdido muestras ahí.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct XrunRecord {
    /// Frames del master escritos antes del xrun.
    pub frame: u64,
    /// Segundos de grabación en ese punto.
    pub seconds: f64,
}

/// Huella del audio del master (solo el chunk `data`), por bloques.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioHash {
    /// Algoritmo (`blake3`).
    pub algorithm: String,
    /// Tamaño de bloque en bytes (el último puede ser menor).
    pub block_bytes: u64,
    /// Huella de cada bloque, en hexadecimal.
    pub blocks: Vec<String>,
    /// Huella de todo el audio, en hexadecimal.
    pub total: String,
}

/// Información de un master.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MasterInfo {
    /// Versión del formato ([`MASTER_INFO_VERSION`]).
    pub version: u32,
    /// Versión de Digityzer que grabó.
    pub digityzer_version: String,
    /// Capa de audio (`alsa`, `pipewire`, `pulse`, `default` o `sintetico`). Los JSON de la
    /// versión 1 no la tienen: eran siempre ALSA directo.
    #[serde(default = "default_backend")]
    pub audio_backend: String,
    /// `true` si la captura fue bit-perfect (solo ALSA directo `hw:`).
    #[serde(default = "default_true")]
    pub bit_perfect: bool,
    /// Dispositivo ALSA (`hw:…`, o el PCM del plugin).
    pub device: String,
    /// Nombre de la tarjeta según ALSA.
    pub card_name: String,
    /// Frecuencia de muestreo (Hz).
    pub sample_rate: u32,
    /// Formato nativo de la captura (`S32_LE`, `S24_3LE`, `S16_LE`).
    pub sample_format: String,
    /// Bits por muestra en el archivo (contenedor).
    pub container_bits: u16,
    /// Bits bajos que han sido cero en todas las muestras (p. ej. 8 en un 24 bits dentro de 32).
    /// Es una observación: el archivo guarda los bits tal cual.
    pub lsb_zero_bits: u32,
    /// Canales abiertos en la tarjeta.
    pub channels_opened: u16,
    /// Par grabado, numerado desde 1 como en la mesa (p. ej. `[3, 4]`).
    pub channel_pair: [u16; 2],
    /// Contenedor (`WAV` o `RF64`).
    pub container: String,
    /// Frames escritos.
    pub frames: u64,
    /// Duración en segundos.
    pub duration_s: f64,
    /// Bytes de audio (chunk `data`).
    pub data_bytes: u64,
    /// Inicio de la grabación (UTC, ISO 8601).
    pub started_at: String,
    /// Fin de la grabación (UTC, ISO 8601).
    pub finished_at: String,
    /// `true` si el hilo de captura consiguió prioridad de tiempo real (SCHED_FIFO).
    pub realtime: bool,
    /// Xruns de ALSA, en orden.
    pub xruns: Vec<XrunRecord>,
    /// Frames perdidos porque el escritor no daba abasto (debería ser siempre 0).
    pub frames_lost_ring: u64,
    /// Huella del audio.
    pub hash: AudioHash,
    /// Notas del disco (sello, catálogo, estado…).
    pub notes: String,
    /// `true` si la grabación no se paró con «Parar» (la tarjeta se apagó o desconectó, o el
    /// programa se cerró de golpe y se recuperó).
    #[serde(default)]
    pub interrupted: bool,
    /// Por qué se interrumpió.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption: Option<String>,
    /// `true` si el master se recuperó de un `.partial` tras un cierre inesperado.
    #[serde(default)]
    pub recovered: bool,
    /// Estimación de los segundos perdidos al recuperar (lo capturado tras la última
    /// confirmación).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_lost: Option<f64>,
}

/// Escribe `bytes` en `path` de forma atómica (temporal en la misma carpeta, `fsync`, rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let res = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_data()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Ficha de una grabación en curso (`<master>.partial.json`), reescrita en cada punto de
/// confirmación. Sirve para recuperar el `.partial` si el programa muere (T1.6). No es la ficha
/// del master: esa se escribe una sola vez al cerrar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialInfo {
    /// La ficha del master tal como quedaría, con `frames` y la huella aún sin calcular.
    pub master: MasterInfo,
    /// Bytes de cada frame del par en el archivo (para cortar).
    pub block_align: u16,
    /// Frames confirmados en el disco (`fdatasync` hecho).
    pub confirmed_frames: u64,
    /// Cuándo se confirmaron (segundos Unix, con decimales).
    pub confirmed_unix: f64,
}

impl PartialInfo {
    /// Ruta de la ficha de un archivo en curso: `<partial>.json`.
    pub fn path_for(partial: &Path) -> PathBuf {
        let mut s = partial.as_os_str().to_owned();
        s.push(".json");
        PathBuf::from(s)
    }

    /// Guarda la ficha de forma atómica.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let json = serde_json::to_vec_pretty(self).map_err(|source| ProjectError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        write_atomic(path, &json).map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Lee la ficha.
    pub fn read(path: &Path) -> Result<Self, ProjectError> {
        let bytes = fs::read(path).map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| ProjectError::Json {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Segundos Unix actuales, con decimales.
pub fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl MasterInfo {
    /// Ruta del JSON de un master: `<master>.json` (`cara-a.wav` → `cara-a.wav.json`).
    pub fn path_for(master: &Path) -> PathBuf {
        let mut s = master.as_os_str().to_owned();
        s.push(".json");
        PathBuf::from(s)
    }

    /// Escribe el JSON en `path` sin sobrescribir nunca y lo deja en solo lectura.
    pub fn write_new(&self, path: &Path) -> Result<(), ProjectError> {
        let io = |source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        };
        let json = serde_json::to_vec_pretty(self).map_err(|source| ProjectError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?;
        f.write_all(&json).map_err(io)?;
        f.write_all(b"\n").map_err(io)?;
        f.sync_all().map_err(io)?;
        set_read_only(path).map_err(io)
    }

    /// Lee un JSON de master.
    pub fn read(path: &Path) -> Result<Self, ProjectError> {
        let bytes = fs::read(path).map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| ProjectError::Json {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Quita los permisos de escritura de un archivo (master o JSON ya cerrados).
pub fn set_read_only(path: &Path) -> std::io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> MasterInfo {
        MasterInfo {
            version: MASTER_INFO_VERSION,
            digityzer_version: "0.1.0".into(),
            audio_backend: "alsa".into(),
            bit_perfect: true,
            device: "hw:CARD=USB1,DEV=0".into(),
            card_name: "XONE 96 USB 1".into(),
            sample_rate: 96_000,
            sample_format: "S32_LE".into(),
            container_bits: 32,
            lsb_zero_bits: 8,
            channels_opened: 12,
            channel_pair: [3, 4],
            container: "WAV".into(),
            frames: 96_000,
            duration_s: 1.0,
            data_bytes: 768_000,
            started_at: "2026-11-18T10:00:00Z".into(),
            finished_at: "2026-11-18T10:00:01Z".into(),
            realtime: true,
            xruns: vec![XrunRecord {
                frame: 4800,
                seconds: 0.05,
            }],
            frames_lost_ring: 0,
            hash: AudioHash {
                algorithm: "blake3".into(),
                block_bytes: 1 << 20,
                blocks: vec!["ab".into()],
                total: "cd".into(),
            },
            notes: "Sello Records CAT-001, cara A".into(),
            interrupted: false,
            interruption: None,
            recovered: false,
            seconds_lost: None,
        }
    }

    #[test]
    fn ida_y_vuelta_sin_sobrescribir_y_en_solo_lectura() {
        let dir = tempfile::tempdir().unwrap();
        let master = dir.path().join("cara-a.wav");
        let p = MasterInfo::path_for(&master);
        assert_eq!(p.file_name().unwrap(), "cara-a.wav.json");
        let info = sample();
        info.write_new(&p).unwrap();
        assert_eq!(MasterInfo::read(&p).unwrap(), info);
        assert!(fs::metadata(&p).unwrap().permissions().readonly());
        assert!(matches!(info.write_new(&p), Err(ProjectError::Io { .. })));
    }

    #[test]
    fn ficha_de_progreso_atomica() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("cara-a.wav.partial");
        let p = PartialInfo::path_for(&partial);
        assert_eq!(p.file_name().unwrap(), "cara-a.wav.partial.json");
        let mut info = PartialInfo {
            master: sample(),
            block_align: 8,
            confirmed_frames: 0,
            confirmed_unix: unix_now(),
        };
        info.save(&p).unwrap();
        info.confirmed_frames = 192_000;
        info.save(&p).unwrap();
        assert_eq!(PartialInfo::read(&p).unwrap(), info);
        assert!(!dir.path().join("cara-a.wav.partial.json.tmp").exists());
    }

    #[test]
    fn json_de_la_version_1_se_lee_como_alsa_bit_perfect() {
        let mut v = serde_json::to_value(sample()).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("audio_backend");
        o.remove("bit_perfect");
        o.remove("interrupted");
        o.remove("recovered");
        let back: MasterInfo = serde_json::from_value(v).unwrap();
        assert_eq!(back.audio_backend, "alsa");
        assert!(back.bit_perfect);
        assert!(!back.interrupted && !back.recovered);
    }
}
