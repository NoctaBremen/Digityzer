//! Espectrograma de toda la pista, calculado **una vez** a resolución fija.
//!
//! - Columnas: se acumulan frames (potencia media lineal, nunca media de dB). Cuando se llega a
//!   `max_columns`, las columnas se **fusionan por pares** y se duplica el número de frames por
//!   columna. El resultado queda siempre entre `max_columns / 2` y `max_columns` columnas sin
//!   conocer la duración de antemano.
//! - Filas: si la FFT tiene más bins que `max_rows`, se agrupan bins contiguos por **máximo** de
//!   potencia, para que un corte abrupto (un MP3) no se difumine.
//! - La interfaz escala la imagen: redimensionar la ventana no recalcula nada.

use crate::cancel::{CancelToken, Progress};
use crate::dsp::frame::{FrameAnalyzer, power_to_db};
use crate::dsp::window::WindowKind;
use crate::error::CoreError;
use crate::pcm::{ChannelView, PcmBlock, PcmFormat};

/// Tamaños de FFT admitidos en la interfaz.
pub const FFT_SIZES: [usize; 6] = [1024, 2048, 4096, 8192, 16_384, 32_768];

/// dB que corresponde al valor 0 de la vista previa cuantizada.
pub const PREVIEW_DB_MIN: f32 = -140.0;
/// dB que corresponde al valor 255 de la vista previa cuantizada.
pub const PREVIEW_DB_MAX: f32 = 0.0;

/// Parámetros del espectrograma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectrogramParams {
    /// Tamaño de la FFT (potencia de 2, de 1.024 a 32.768).
    pub fft_size: usize,
    /// Ventana.
    pub window: WindowKind,
    /// Canales analizados.
    pub channel_view: ChannelView,
    /// Máximo de columnas (≥ 2). El resultado tiene entre `max/2` y `max`.
    pub max_columns: usize,
    /// Máximo de filas (≥ 1).
    pub max_rows: usize,
}

impl Default for SpectrogramParams {
    fn default() -> Self {
        Self {
            fft_size: 8192,
            window: WindowKind::Hann,
            channel_view: ChannelView::Mix,
            max_columns: 4096,
            max_rows: 2048,
        }
    }
}

impl SpectrogramParams {
    /// Comprueba los parámetros.
    pub fn validate(&self) -> Result<(), CoreError> {
        if !FFT_SIZES.contains(&self.fft_size) {
            return Err(CoreError::Unsupported(format!(
                "tamaño de FFT {}: debe ser potencia de 2 entre 1.024 y 32.768",
                self.fft_size
            )));
        }
        if self.max_columns < 2 {
            return Err(CoreError::Unsupported("max_columns debe ser ≥ 2".into()));
        }
        if self.max_rows < 1 {
            return Err(CoreError::Unsupported("max_rows debe ser ≥ 1".into()));
        }
        Ok(())
    }
}

/// Acceso común a una rejilla de dB (espectrograma completo o vista previa).
pub trait DbGrid {
    /// Número de columnas (tiempo).
    fn columns(&self) -> usize;
    /// Número de filas (frecuencia; fila 0 = 0 Hz).
    fn rows(&self) -> usize;
    /// Valor en dB de la celda.
    fn db_at(&self, col: usize, row: usize) -> f32;
}

/// Espectrograma calculado.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrogramData {
    /// Columnas (tiempo).
    pub columns: usize,
    /// Filas (frecuencia).
    pub rows: usize,
    /// Frecuencia de muestreo del audio.
    pub sample_rate: u32,
    /// Duración del audio analizado, en segundos.
    pub duration_s: f64,
    /// Hz que cubre cada fila (la fila `r` empieza en `r · hz_per_row`).
    pub hz_per_row: f64,
    /// Segundos que cubre cada columna (salvo, quizá, la última).
    pub seconds_per_column: f64,
    /// dBFS: `db[col * rows + row]`, fila 0 = 0 Hz.
    pub db: Vec<f32>,
}

impl SpectrogramData {
    /// Frecuencia de Nyquist.
    pub fn nyquist_hz(&self) -> f64 {
        f64::from(self.sample_rate) / 2.0
    }

    /// Fila que contiene la frecuencia `hz` (recortada al rango).
    pub fn row_for_hz(&self, hz: f64) -> usize {
        ((hz / self.hz_per_row).floor().max(0.0) as usize).min(self.rows.saturating_sub(1))
    }
}

impl DbGrid for SpectrogramData {
    fn columns(&self) -> usize {
        self.columns
    }
    fn rows(&self) -> usize {
        self.rows
    }
    fn db_at(&self, col: usize, row: usize) -> f32 {
        self.db[col * self.rows + row]
    }
}

/// Vista previa cuantizada a u8 (0 = −140 dB, 255 = 0 dB). Unas 4 veces más pequeña que
/// [`SpectrogramData`]; la usan los lotes para no guardar el espectrograma completo de cada pista.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrogramPreview {
    /// Columnas (tiempo).
    pub columns: usize,
    /// Filas (frecuencia).
    pub rows: usize,
    /// Frecuencia de muestreo del audio.
    pub sample_rate: u32,
    /// Duración del audio analizado, en segundos.
    pub duration_s: f64,
    /// Hz por fila.
    pub hz_per_row: f64,
    /// Valores cuantizados: `data[col * rows + row]`.
    pub data: Vec<u8>,
}

impl From<&SpectrogramData> for SpectrogramPreview {
    fn from(s: &SpectrogramData) -> Self {
        let range = PREVIEW_DB_MAX - PREVIEW_DB_MIN;
        Self {
            columns: s.columns,
            rows: s.rows,
            sample_rate: s.sample_rate,
            duration_s: s.duration_s,
            hz_per_row: s.hz_per_row,
            data: s
                .db
                .iter()
                .map(|&db| {
                    ((db - PREVIEW_DB_MIN) / range * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                })
                .collect(),
        }
    }
}

impl From<&SpectrogramPreview> for SpectrogramData {
    /// Descuantiza la vista previa (resolución de ~0,55 dB) para mostrarla como un espectrograma.
    fn from(p: &SpectrogramPreview) -> Self {
        Self {
            columns: p.columns,
            rows: p.rows,
            sample_rate: p.sample_rate,
            duration_s: p.duration_s,
            hz_per_row: p.hz_per_row,
            seconds_per_column: if p.columns > 0 {
                p.duration_s / p.columns as f64
            } else {
                0.0
            },
            db: (0..p.columns)
                .flat_map(|c| (0..p.rows).map(move |r| (c, r)))
                .map(|(c, r)| p.db_at(c, r))
                .collect(),
        }
    }
}

impl DbGrid for SpectrogramPreview {
    fn columns(&self) -> usize {
        self.columns
    }
    fn rows(&self) -> usize {
        self.rows
    }
    fn db_at(&self, col: usize, row: usize) -> f32 {
        let q = f32::from(self.data[col * self.rows + row]);
        PREVIEW_DB_MIN + q / 255.0 * (PREVIEW_DB_MAX - PREVIEW_DB_MIN)
    }
}

/// Acumulador incremental de columnas (ver la doc del módulo).
#[derive(Debug, Clone)]
pub struct SpectrogramBuilder {
    max_columns: usize,
    rows: usize,
    bins_per_row: usize,
    hop: usize,
    sample_rate: u32,
    hz_per_row: f64,
    frames_per_column: u64,
    /// Columnas completas: potencia media por fila.
    done: Vec<Vec<f32>>,
    /// Columna en curso: suma de potencias por fila y número de frames.
    acc: Vec<f64>,
    acc_frames: u64,
}

impl SpectrogramBuilder {
    /// Crea un acumulador para frames de `bins` bins, generados cada `hop` muestras.
    pub fn new(
        max_columns: usize,
        max_rows: usize,
        bins: usize,
        hop: usize,
        sample_rate: u32,
    ) -> Self {
        let max_rows = max_rows.max(1);
        let bins = bins.max(1);
        let bins_per_row = bins.div_ceil(max_rows);
        let rows = bins.div_ceil(bins_per_row);
        // bins = fft/2 + 1 → Hz por bin = rate / fft = rate / (2 · (bins − 1)).
        let hz_per_bin = if bins > 1 {
            f64::from(sample_rate) / (2.0 * (bins - 1) as f64)
        } else {
            f64::from(sample_rate) / 2.0
        };
        Self {
            max_columns: max_columns.max(2),
            rows,
            bins_per_row,
            hop,
            sample_rate,
            hz_per_row: hz_per_bin * bins_per_row as f64,
            frames_per_column: 1,
            done: Vec::new(),
            acc: vec![0.0; rows],
            acc_frames: 0,
        }
    }

    /// Filas del resultado.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Añade un frame de potencias lineales (un valor por bin).
    pub fn push_frame(&mut self, power: &[f32]) {
        for (r, chunk) in power.chunks(self.bins_per_row).take(self.rows).enumerate() {
            let max = chunk.iter().copied().fold(0.0f32, f32::max);
            self.acc[r] += f64::from(max);
        }
        self.acc_frames += 1;
        if self.acc_frames == self.frames_per_column {
            if self.done.len() < self.max_columns {
                self.close_column();
            } else {
                self.merge_pairs();
            }
        }
    }

    fn close_column(&mut self) {
        let n = self.acc_frames as f64;
        self.done
            .push(self.acc.iter().map(|&s| (s / n) as f32).collect());
        self.acc.iter_mut().for_each(|s| *s = 0.0);
        self.acc_frames = 0;
    }

    /// Se llama cuando hay `max_columns` columnas completas y la columna en curso también está
    /// completa: fusiona las columnas por pares y duplica `frames_per_column`.
    ///
    /// La columna en curso (con `frames_per_column` frames) pasa a ser la mitad de una columna
    /// nueva; si `max_columns` es impar, se completa con la columna sobrante.
    fn merge_pairs(&mut self) {
        let old = std::mem::take(&mut self.done);
        let mut iter = old.chunks_exact(2);
        for pair in &mut iter {
            self.done.push(
                pair[0]
                    .iter()
                    .zip(&pair[1])
                    .map(|(a, b)| (a + b) * 0.5)
                    .collect(),
            );
        }
        let n = self.frames_per_column as f64;
        self.frames_per_column *= 2;
        if let [odd] = iter.remainder() {
            for (a, &p) in self.acc.iter_mut().zip(odd) {
                *a += f64::from(p) * n;
            }
            self.acc_frames += self.frames_per_column / 2;
            self.close_column();
        }
    }

    /// Termina y devuelve el espectrograma. `total_samples` = frames de audio analizados.
    pub fn finish(mut self, total_samples: u64) -> SpectrogramData {
        if self.acc_frames > 0 {
            if self.done.len() < self.max_columns {
                self.close_column();
            } else if let Some(last) = self.done.last_mut() {
                // Sin sitio para otra columna: los frames sobrantes se suman a la última.
                let n = self.frames_per_column as f64;
                let total = n + self.acc_frames as f64;
                for (l, &a) in last.iter_mut().zip(&self.acc) {
                    *l = ((f64::from(*l) * n + a) / total) as f32;
                }
            }
        }
        let columns = self.done.len();
        let db = self
            .done
            .iter()
            .flat_map(|col| col.iter().map(|&p| power_to_db(p)))
            .collect();
        let rate = f64::from(self.sample_rate);
        SpectrogramData {
            columns,
            rows: self.rows,
            sample_rate: self.sample_rate,
            duration_s: total_samples as f64 / rate,
            hz_per_row: self.hz_per_row,
            seconds_per_column: self.frames_per_column as f64 * self.hop as f64 / rate,
            db,
        }
    }
}

/// Calcula el espectrograma de una secuencia de bloques PCM con el formato `format`.
///
/// Adaptado de `spector-core::compute_spectrogram`: en Spector la entrada era un archivo
/// decodificado con symphonia; aquí es cualquier iterador de bloques (el master que se acaba de
/// grabar, un tramo en memoria o señales de test).
///
/// Comprueba `cancel` en cada bloque (→ [`CoreError::Cancelled`]) e informa del progreso
/// (frames procesados / frames totales) si `format.frames` es conocido.
pub fn compute_spectrogram<I>(
    blocks: I,
    format: PcmFormat,
    params: &SpectrogramParams,
    cancel: &CancelToken,
    progress: &dyn Progress,
) -> Result<SpectrogramData, CoreError>
where
    I: IntoIterator<Item = PcmBlock>,
{
    params.validate()?;
    let mut analyzer = FrameAnalyzer::new(
        params.fft_size,
        params.window,
        params.channel_view,
        format.channels,
    )?;
    let mut builder = SpectrogramBuilder::new(
        params.max_columns,
        params.max_rows,
        analyzer.bins(),
        analyzer.hop(),
        format.sample_rate,
    );
    let total = format.frames.filter(|&f| f > 0);
    let mut last_reported = 0.0f32;
    for block in blocks {
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        analyzer.push(&block, |p| builder.push_frame(p))?;
        if let Some(total) = total {
            let f = (analyzer.samples_seen() as f64 / total as f64).min(1.0) as f32;
            if f - last_reported >= 0.005 {
                progress.report(f);
                last_reported = f;
            }
        }
    }
    analyzer.finish(|p| builder.push_frame(p));
    progress.report(1.0);
    Ok(builder.finish(analyzer.samples_seen()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(bins: usize, value: f32) -> Vec<f32> {
        vec![value; bins]
    }

    #[test]
    fn diezmado_por_pares() {
        for max in [8usize, 7, 2] {
            for frames in [1u64, 5, 8, 9, 100, 1000] {
                let mut b = SpectrogramBuilder::new(max, 4, 4, 512, 48_000);
                for _ in 0..frames {
                    b.push_frame(&frame(4, 1.0));
                }
                let s = b.finish(frames * 512);
                assert!(
                    s.columns <= max,
                    "max {max}, frames {frames}: {} columnas",
                    s.columns
                );
                if frames as usize >= max {
                    assert!(
                        s.columns >= max / 2,
                        "max {max}, frames {frames}: {}",
                        s.columns
                    );
                } else {
                    assert_eq!(s.columns, frames as usize);
                }
            }
        }
    }

    #[test]
    fn la_fusion_conserva_la_media_de_potencia() {
        // Frames con potencia creciente 1..=16; con 4 columnas cada una es la media de 4.
        let mut b = SpectrogramBuilder::new(4, 1, 1, 1, 48_000);
        for i in 1..=16 {
            b.push_frame(&[i as f32]);
        }
        let s = b.finish(16);
        let expected = [2.5f32, 6.5, 10.5, 14.5].map(|p| 10.0 * p.log10());
        assert_eq!(s.columns, 4);
        for (got, want) in s.db.iter().zip(expected) {
            assert!((got - want).abs() < 1e-4, "{got} vs {want}");
        }
    }

    #[test]
    fn columna_impar_conserva_su_peso() {
        // max 3: con el frame 4 se fusionan (1, 2) → 1,5 y la columna sobrante (3) se une al
        // frame 4 → (3 + 4) / 2.
        let mut b = SpectrogramBuilder::new(3, 1, 1, 1, 48_000);
        for i in 1..=4 {
            b.push_frame(&[i as f32]);
        }
        let s = b.finish(4);
        let lin: Vec<f32> = s.db.iter().map(|d| 10f32.powf(d / 10.0)).collect();
        assert_eq!(lin.len(), 2);
        assert!(
            (lin[0] - 1.5).abs() < 1e-4 && (lin[1] - 3.5).abs() < 1e-4,
            "{lin:?}"
        );
    }

    #[test]
    fn agrupacion_de_filas_por_maximo() {
        let mut b = SpectrogramBuilder::new(4, 2, 5, 4, 8_000);
        assert_eq!(b.rows(), 2); // 5 bins → 3 por fila → 2 filas
        b.push_frame(&[0.1, 1.0, 0.1, 0.01, 0.5]);
        let s = b.finish(8);
        assert!((s.db[0] - 0.0).abs() < 1e-5, "máximo de la fila 0 = 1,0");
        assert!((s.db[1] - 10.0 * 0.5f32.log10()).abs() < 1e-5);
        // 5 bins → FFT de 8 a 8 kHz → 1.000 Hz por bin, 3 bins por fila.
        assert!((s.hz_per_row - 3000.0).abs() < 1e-9);
    }

    #[test]
    fn vista_previa_cuantizada() {
        let s = SpectrogramData {
            columns: 1,
            rows: 4,
            sample_rate: 48_000,
            duration_s: 1.0,
            hz_per_row: 1.0,
            seconds_per_column: 1.0,
            db: vec![-200.0, -140.0, -70.0, 0.0],
        };
        let p = SpectrogramPreview::from(&s);
        assert_eq!(p.data, vec![0, 0, 128, 255]);
        assert!((p.db_at(0, 2) - -69.8).abs() < 0.3);
        assert_eq!(p.db_at(0, 3), 0.0);
    }

    #[test]
    fn vista_previa_a_datos() {
        let s = SpectrogramData {
            columns: 2,
            rows: 2,
            sample_rate: 48_000,
            duration_s: 4.0,
            hz_per_row: 12_000.0,
            seconds_per_column: 2.0,
            db: vec![-140.0, 0.0, -70.0, -10.0],
        };
        let back = SpectrogramData::from(&SpectrogramPreview::from(&s));
        assert_eq!(
            (back.columns, back.rows, back.seconds_per_column),
            (2, 2, 2.0)
        );
        for (a, b) in back.db.iter().zip(&s.db) {
            assert!((a - b).abs() < 0.3, "{a} vs {b}");
        }
    }

    #[test]
    fn validacion() {
        assert!(SpectrogramParams::default().validate().is_ok());
        let bad = |f: fn(&mut SpectrogramParams)| {
            let mut p = SpectrogramParams::default();
            f(&mut p);
            p.validate().is_err()
        };
        assert!(bad(|p| p.fft_size = 512));
        assert!(bad(|p| p.fft_size = 65_536));
        assert!(bad(|p| p.fft_size = 3000));
        assert!(bad(|p| p.max_columns = 1));
        assert!(bad(|p| p.max_rows = 0));
    }
}
