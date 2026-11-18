//! Análisis espectral por frames: ventana + FFT real + potencias normalizadas a escala completa.
//!
//! # Escala
//!
//! La potencia del bin `k` es `|X_k|² · (2 / Σw)²` (para `k = 0` y `k = N/2`, `(1 / Σw)²`).
//! Con esta escala, un seno de amplitud `A` centrado en un bin da potencia `A²`: un seno a escala
//! completa marca **0 dBFS** y uno de amplitud 0,1, −20 dBFS. Es la convención que usan el
//! espectrograma y el análisis.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::dsp::window::{WindowKind, window};
use crate::error::CoreError;
use crate::pcm::{ChannelView, PcmBlock, PcmData};

/// Genera un frame de potencias cada `fft_size / 2` muestras (solapamiento del 50 %).
pub struct FrameAnalyzer {
    fft_size: usize,
    hop: usize,
    view: ChannelView,
    channels: u16,
    window: Vec<f32>,
    scale_mid: f32,
    scale_edge: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Muestras pendientes de cada flujo analizado (un flujo por canal en `Mix`; uno en otro caso).
    pending: Vec<Vec<f32>>,
    /// Inicio de la próxima ventana dentro de `pending`.
    offset: usize,
    power: Vec<f32>,
    /// Vista Side calculada a partir de las FFT de L y R de la mezcla (ver [`Self::enable_side`]).
    side: Option<SideState>,
    frames_emitted: u64,
    samples_seen: u64,
}

/// Estado de la vista Side derivada: espectro de L (canal 0) y potencias de Side.
struct SideState {
    left: Vec<Complex<f32>>,
    power: Vec<f32>,
}

impl std::fmt::Debug for FrameAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameAnalyzer")
            .field("fft_size", &self.fft_size)
            .field("view", &self.view)
            .field("channels", &self.channels)
            .field("frames_emitted", &self.frames_emitted)
            .finish_non_exhaustive()
    }
}

impl FrameAnalyzer {
    /// Crea un analizador.
    ///
    /// Errores: `fft_size` no es potencia de 2 o es menor que 16; la vista no es compatible con
    /// el número de canales (Side en mono, canal inexistente).
    pub fn new(
        fft_size: usize,
        kind: WindowKind,
        view: ChannelView,
        channels: u16,
    ) -> Result<Self, CoreError> {
        if fft_size < 16 || !fft_size.is_power_of_two() {
            return Err(CoreError::Unsupported(format!(
                "tamaño de FFT {fft_size}: debe ser potencia de 2"
            )));
        }
        if channels == 0 {
            return Err(CoreError::Unsupported("audio sin canales".into()));
        }
        let streams = match view {
            ChannelView::Mix => usize::from(channels),
            ChannelView::Channel(c) if c < channels => 1,
            ChannelView::Channel(c) => {
                return Err(CoreError::Unsupported(format!(
                    "el canal {} no existe (el archivo tiene {channels})",
                    c + 1
                )));
            }
            ChannelView::Side if channels >= 2 => 1,
            ChannelView::Side => {
                return Err(CoreError::Unsupported(
                    "la vista Side necesita al menos dos canales".into(),
                ));
            }
        };
        let window = window(kind, fft_size);
        let wsum: f64 = window.iter().map(|&w| f64::from(w)).sum();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        Ok(Self {
            fft_size,
            hop: fft_size / 2,
            view,
            channels,
            scale_mid: (2.0 / wsum).powi(2) as f32,
            scale_edge: (1.0 / wsum).powi(2) as f32,
            fft_in: fft.make_input_vec(),
            fft_out: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            pending: vec![Vec::with_capacity(fft_size * 4); streams],
            offset: 0,
            power: vec![0.0; fft_size / 2 + 1],
            side: None,
            frames_emitted: 0,
            samples_seen: 0,
        })
    }

    /// Número de bins por frame (`fft_size / 2 + 1`).
    pub fn bins(&self) -> usize {
        self.fft_size / 2 + 1
    }

    /// Muestras entre frames consecutivos.
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Frames emitidos hasta ahora.
    pub fn frames_emitted(&self) -> u64 {
        self.frames_emitted
    }

    /// Frames de audio (muestras por canal) recibidos hasta ahora.
    pub fn samples_seen(&self) -> u64 {
        self.samples_seen
    }

    /// Calcula también la vista Side (`(L − R) / 2`) **sin otra FFT**: como la FFT es lineal,
    /// `FFT((L − R)/2) = (X_L − X_R)/2`, y la mezcla ya calcula `X_L` y `X_R`. Solo para la vista
    /// Mezcla con ≥ 2 canales. Las potencias Side llegan como segundo argumento de
    /// [`Self::push_with_side`].
    pub fn enable_side(&mut self) -> Result<(), CoreError> {
        if self.view != ChannelView::Mix || self.channels < 2 {
            return Err(CoreError::Unsupported(
                "la vista Side derivada requiere la mezcla de ≥ 2 canales".into(),
            ));
        }
        self.side = Some(SideState {
            left: self.fft.make_output_vec(),
            power: vec![0.0; self.fft_size / 2 + 1],
        });
        Ok(())
    }

    /// Añade un bloque y llama a `on_frame` con las potencias (lineales) de cada frame completo.
    pub fn push(
        &mut self,
        block: &PcmBlock,
        mut on_frame: impl FnMut(&[f32]),
    ) -> Result<(), CoreError> {
        self.push_with_side(block, |p, _| on_frame(p))
    }

    /// Como [`Self::push`], pero `on_frame` recibe también las potencias de la vista Side si se
    /// activó con [`Self::enable_side`] (`None` en otro caso).
    pub fn push_with_side(
        &mut self,
        block: &PcmBlock,
        mut on_frame: impl FnMut(&[f32], Option<&[f32]>),
    ) -> Result<(), CoreError> {
        if block.channels != self.channels {
            return Err(CoreError::Decode(format!(
                "bloque con {} canales en un análisis de {}",
                block.channels, self.channels
            )));
        }
        self.append(block);
        self.samples_seen += block.frames as u64;
        while self.pending[0].len() - self.offset >= self.fft_size {
            self.emit_frame(self.offset, &mut on_frame);
            self.offset += self.hop;
        }
        // Compacta cuando lo ya consumido domina el búfer.
        if self.offset > self.fft_size * 2 {
            for p in &mut self.pending {
                p.drain(..self.offset);
            }
            self.offset = 0;
        }
        Ok(())
    }

    /// Termina el análisis. Si no se ha emitido ningún frame (audio más corto que la FFT) y hay
    /// muestras, emite uno rellenado con ceros. El resto final menor que una FFT se descarta.
    pub fn finish(&mut self, mut on_frame: impl FnMut(&[f32])) {
        self.finish_with_side(|p, _| on_frame(p));
    }

    /// Como [`Self::finish`], con las potencias de la vista Side (ver [`Self::push_with_side`]).
    pub fn finish_with_side(&mut self, mut on_frame: impl FnMut(&[f32], Option<&[f32]>)) {
        let available = self.pending[0].len() - self.offset;
        if self.frames_emitted == 0 && available > 0 {
            for p in &mut self.pending {
                p.resize(self.offset + self.fft_size, 0.0);
            }
            self.emit_frame(self.offset, &mut on_frame);
        }
        for p in &mut self.pending {
            p.clear();
        }
        self.offset = 0;
    }

    fn append(&mut self, block: &PcmBlock) {
        let ch = usize::from(self.channels);
        let get = |i: usize| -> f32 {
            match &block.data {
                PcmData::Int { bits, samples } => {
                    samples[i] as f32 / (1u64 << bits.saturating_sub(1)) as f32
                }
                PcmData::F32(s) => s[i],
                PcmData::F64(s) => s[i] as f32,
            }
        };
        match self.view {
            ChannelView::Mix => {
                for (c, p) in self.pending.iter_mut().enumerate() {
                    p.extend((0..block.frames).map(|f| get(f * ch + c)));
                }
            }
            ChannelView::Channel(c) => {
                let c = usize::from(c);
                self.pending[0].extend((0..block.frames).map(|f| get(f * ch + c)));
            }
            ChannelView::Side => {
                self.pending[0]
                    .extend((0..block.frames).map(|f| (get(f * ch) - get(f * ch + 1)) * 0.5));
            }
        }
    }

    fn emit_frame(&mut self, start: usize, on_frame: &mut impl FnMut(&[f32], Option<&[f32]>)) {
        self.power.iter_mut().for_each(|p| *p = 0.0);
        let streams = self.pending.len();
        let last = self.power.len() - 1;
        for s in 0..streams {
            let src = &self.pending[s][start..start + self.fft_size];
            for ((dst, &x), &w) in self.fft_in.iter_mut().zip(src).zip(&self.window) {
                *dst = x * w;
            }
            // Longitudes creadas por el propio planificador: process no puede fallar.
            if self
                .fft
                .process_with_scratch(&mut self.fft_in, &mut self.fft_out, &mut self.scratch)
                .is_err()
            {
                continue;
            }
            for (k, (p, c)) in self.power.iter_mut().zip(&self.fft_out).enumerate() {
                let scale = if k == 0 || k == last {
                    self.scale_edge
                } else {
                    self.scale_mid
                };
                *p += c.norm_sqr() * scale;
            }
            if let Some(side) = self.side.as_mut() {
                if s == 0 {
                    side.left.copy_from_slice(&self.fft_out);
                } else if s == 1 {
                    for (k, ((p, l), r)) in side
                        .power
                        .iter_mut()
                        .zip(&side.left)
                        .zip(&self.fft_out)
                        .enumerate()
                    {
                        let scale = if k == 0 || k == last {
                            self.scale_edge
                        } else {
                            self.scale_mid
                        };
                        *p = ((l - r) * 0.5).norm_sqr() * scale;
                    }
                }
            }
        }
        if streams > 1 {
            let inv = 1.0 / streams as f32;
            self.power.iter_mut().for_each(|p| *p *= inv);
        }
        self.frames_emitted += 1;
        on_frame(&self.power, self.side.as_ref().map(|s| s.power.as_slice()));
    }
}

/// Potencia lineal → dB, con suelo en −200 dB.
pub fn power_to_db(p: f32) -> f32 {
    (10.0 * p.max(1e-20).log10()).clamp(-200.0, 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_f32(channels: u16, data: Vec<f32>) -> PcmBlock {
        PcmBlock {
            channels,
            frames: data.len() / usize::from(channels),
            data: PcmData::F32(data),
        }
    }

    fn sine(freq: f64, rate: f64, n: usize, amp: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / rate).sin()) as f32)
            .collect()
    }

    #[test]
    fn seno_a_escala_completa_es_0_dbfs() {
        for kind in [WindowKind::Hann, WindowKind::BlackmanHarris] {
            // 1.500 Hz a 48 kHz con FFT de 8.192 cae justo en el bin 256.
            let mut a = FrameAnalyzer::new(8192, kind, ChannelView::Mix, 1).unwrap();
            let mut peak = Vec::new();
            a.push(&block_f32(1, sine(1500.0, 48_000.0, 48_000, 1.0)), |p| {
                peak.push(p[256])
            })
            .unwrap();
            assert!(!peak.is_empty());
            for &p in &peak {
                assert!(
                    power_to_db(p).abs() < 0.05,
                    "{kind:?}: {} dB",
                    power_to_db(p)
                );
            }
            let mut a = FrameAnalyzer::new(8192, kind, ChannelView::Mix, 1).unwrap();
            let mut db = 0.0;
            a.push(&block_f32(1, sine(1500.0, 48_000.0, 48_000, 0.1)), |p| {
                db = power_to_db(p[256])
            })
            .unwrap();
            assert!((db + 20.0).abs() < 0.05, "{kind:?}: {db} dB");
        }
    }

    #[test]
    fn numero_de_frames_y_salto() {
        let mut a = FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Mix, 1).unwrap();
        let mut n = 0;
        // Entrada en bloques de tamaños irregulares: 10.000 muestras → 1 + (10000−1024)/512 = 18.
        for chunk in vec![0.1f32; 10_000].chunks(777) {
            a.push(&block_f32(1, chunk.to_vec()), |_| n += 1).unwrap();
        }
        a.finish(|_| n += 1);
        assert_eq!(n, 18);
        assert_eq!(a.samples_seen(), 10_000);
    }

    #[test]
    fn audio_corto_produce_un_frame() {
        let mut a = FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Mix, 1).unwrap();
        let mut n = 0;
        a.push(&block_f32(1, vec![0.5; 100]), |_| n += 1).unwrap();
        a.finish(|_| n += 1);
        assert_eq!(n, 1);
    }

    #[test]
    fn mezcla_es_media_de_potencias() {
        // L = seno a escala completa, R = silencio → Mix = la mitad de potencia (−3,01 dB).
        let l = sine(1500.0, 48_000.0, 16_384, 1.0);
        let inter: Vec<f32> = l.iter().flat_map(|&x| [x, 0.0]).collect();
        let mut a = FrameAnalyzer::new(8192, WindowKind::Hann, ChannelView::Mix, 2).unwrap();
        let mut db = 0.0;
        a.push(&block_f32(2, inter.clone()), |p| db = power_to_db(p[256]))
            .unwrap();
        assert!((db + 3.01).abs() < 0.05, "Mix: {db}");

        let mut a = FrameAnalyzer::new(8192, WindowKind::Hann, ChannelView::Channel(1), 2).unwrap();
        a.push(&block_f32(2, inter.clone()), |p| db = power_to_db(p[256]))
            .unwrap();
        assert!(db < -150.0, "canal derecho en silencio: {db}");

        // Side = (L − R) / 2 → amplitud 0,5 → −6,02 dB.
        let mut a = FrameAnalyzer::new(8192, WindowKind::Hann, ChannelView::Side, 2).unwrap();
        a.push(&block_f32(2, inter), |p| db = power_to_db(p[256]))
            .unwrap();
        assert!((db + 6.02).abs() < 0.05, "Side: {db}");
    }

    #[test]
    fn side_derivada_igual_a_la_directa() {
        let l = sine(1500.0, 48_000.0, 20_000, 0.7);
        let r = sine(3000.0, 48_000.0, 20_000, 0.3);
        let inter: Vec<f32> = l.iter().zip(&r).flat_map(|(&a, &b)| [a, b]).collect();
        let mut direct = Vec::new();
        let mut a = FrameAnalyzer::new(2048, WindowKind::Hann, ChannelView::Side, 2).unwrap();
        a.push(&block_f32(2, inter.clone()), |p| direct.push(p.to_vec()))
            .unwrap();
        let mut derived = Vec::new();
        let mut m = FrameAnalyzer::new(2048, WindowKind::Hann, ChannelView::Mix, 2).unwrap();
        m.enable_side().unwrap();
        m.push_with_side(&block_f32(2, inter), |_, s| {
            derived.push(s.unwrap().to_vec())
        })
        .unwrap();
        assert_eq!(direct.len(), derived.len());
        for (d, e) in direct.iter().zip(&derived) {
            for (x, y) in d.iter().zip(e) {
                assert!((x - y).abs() <= 1e-6 * x.abs().max(1e-6), "{x} vs {y}");
            }
        }
        let mut mono = FrameAnalyzer::new(2048, WindowKind::Hann, ChannelView::Mix, 1).unwrap();
        assert!(mono.enable_side().is_err());
    }

    #[test]
    fn enteros_escalados() {
        let s: Vec<i32> = sine(1500.0, 48_000.0, 16_384, 0.5)
            .iter()
            .map(|&x| (f64::from(x) * 8_388_608.0).round() as i32)
            .collect();
        let b = PcmBlock {
            channels: 1,
            frames: s.len(),
            data: PcmData::Int {
                bits: 24,
                samples: s,
            },
        };
        let mut a = FrameAnalyzer::new(8192, WindowKind::Hann, ChannelView::Mix, 1).unwrap();
        let mut db = 0.0;
        a.push(&b, |p| db = power_to_db(p[256])).unwrap();
        assert!((db + 6.02).abs() < 0.05, "{db}");
    }

    #[test]
    fn parametros_invalidos() {
        assert!(FrameAnalyzer::new(1000, WindowKind::Hann, ChannelView::Mix, 1).is_err());
        assert!(FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Side, 1).is_err());
        assert!(FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Channel(2), 2).is_err());
        assert!(FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Mix, 0).is_err());
        let mut a = FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Mix, 2).unwrap();
        assert!(a.push(&block_f32(1, vec![0.0; 10]), |_| ()).is_err());
    }

    #[test]
    fn silencio_es_suelo() {
        let mut a = FrameAnalyzer::new(1024, WindowKind::Hann, ChannelView::Mix, 1).unwrap();
        let mut min = 0.0f32;
        a.push(&block_f32(1, vec![0.0; 4096]), |p| {
            min = p.iter().map(|&x| power_to_db(x)).fold(0.0, f32::min)
        })
        .unwrap();
        assert_eq!(min, -200.0);
    }
}
