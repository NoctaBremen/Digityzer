//! Bloques PCM en memoria y vistas de canal para el DSP (extraído de `spector-core::decode`).

use crate::error::CoreError;

/// Muestras de un bloque decodificado, **entrelazadas**.
#[derive(Debug, Clone, PartialEq)]
pub enum PcmData {
    /// Enteros justificados a la derecha: un 24 bits va de −8.388.608 a 8.388.607.
    Int {
        /// Profundidad en bits (1–32).
        bits: u32,
        /// Muestras entrelazadas.
        samples: Vec<i32>,
    },
    /// Coma flotante de 32 bits (PCM float o formatos con pérdida).
    F32(Vec<f32>),
    /// Coma flotante de 64 bits.
    F64(Vec<f64>),
}

/// Vista de canales para el análisis espectral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelView {
    /// Media de todos los canales.
    Mix,
    /// Un único canal (índice desde 0).
    Channel(u16),
    /// `(L − R) / 2` (requiere ≥ 2 canales).
    Side,
}

/// Bloque de audio decodificado.
#[derive(Debug, Clone, PartialEq)]
pub struct PcmBlock {
    /// Número de canales.
    pub channels: u16,
    /// Número de frames (muestras por canal).
    pub frames: usize,
    /// Muestras entrelazadas.
    pub data: PcmData,
}

impl PcmBlock {
    /// Valor de la muestra `i` (entrelazada) en coma flotante, con 1,0 = escala completa.
    fn sample_f32(&self, i: usize) -> f32 {
        match &self.data {
            PcmData::Int { bits, samples } => {
                samples[i] as f32 / (1u64 << (bits.saturating_sub(1))) as f32
            }
            PcmData::F32(s) => s[i],
            PcmData::F64(s) => s[i] as f32,
        }
    }

    /// Canal o mezcla en `f32` (1,0 = escala completa), **solo para el DSP**.
    ///
    /// Nunca se usa en la ruta bit-perfect.
    pub fn channel_f32(&self, view: ChannelView) -> Result<Vec<f32>, CoreError> {
        let ch = usize::from(self.channels);
        if ch == 0 {
            return Ok(Vec::new());
        }
        match view {
            ChannelView::Mix => Ok((0..self.frames)
                .map(|f| (0..ch).map(|c| self.sample_f32(f * ch + c)).sum::<f32>() / ch as f32)
                .collect()),
            ChannelView::Channel(c) => {
                let c = usize::from(c);
                if c >= ch {
                    return Err(CoreError::Unsupported(format!(
                        "el canal {} no existe (el archivo tiene {ch})",
                        c + 1
                    )));
                }
                Ok((0..self.frames)
                    .map(|f| self.sample_f32(f * ch + c))
                    .collect())
            }
            ChannelView::Side => {
                if ch < 2 {
                    return Err(CoreError::Unsupported(
                        "la vista Side necesita al menos dos canales".into(),
                    ));
                }
                Ok((0..self.frames)
                    .map(|f| (self.sample_f32(f * ch) - self.sample_f32(f * ch + 1)) / 2.0)
                    .collect())
            }
        }
    }
}

/// Formato de una secuencia de bloques PCM (nuevo en Digityzer: sustituye a la información que
/// en Spector salía del archivo decodificado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    /// Frecuencia de muestreo (Hz).
    pub sample_rate: u32,
    /// Número de canales.
    pub channels: u16,
    /// Frames totales, si se conocen (solo para el progreso).
    pub frames: Option<u64>,
}
