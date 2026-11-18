//! Origen de la captura: la tarjeta por ALSA ([`crate::alsa_source::AlsaSource`]) o una señal
//! sintética para los tests y para probar sin la mesa ([`SyntheticSource`]).

use std::time::{Duration, Instant};

use crate::error::AudioError;
use crate::format::SampleFormat;

/// Configuración efectiva del flujo, tal cual la entrega la tarjeta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    /// Frecuencia de muestreo (Hz).
    pub rate: u32,
    /// Formato nativo.
    pub format: SampleFormat,
    /// Canales abiertos (todos los que expone la interfaz).
    pub channels: u16,
    /// Frames por lectura.
    pub period_frames: usize,
}

impl StreamConfig {
    /// Bytes por frame (todos los canales).
    pub fn frame_bytes(&self) -> usize {
        self.format.bytes() * usize::from(self.channels)
    }
}

/// Resultado de una lectura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    /// Se han leído tantos frames al principio del búfer.
    Frames(usize),
    /// Xrun: ALSA ha perdido muestras. El origen ya se ha recuperado y sigue capturando.
    Xrun,
    /// El origen no da más datos (solo orígenes finitos, como los de test).
    End,
}

/// Origen de audio entrelazado. [`CaptureSource::read`] se llama desde el hilo de captura, así
/// que no debe reservar memoria ni bloquearse más allá de esperar al siguiente período.
pub trait CaptureSource: Send {
    /// Configuración del flujo.
    fn config(&self) -> StreamConfig;
    /// Dispositivo (`hw:…` o `sintético`).
    fn device(&self) -> &str;
    /// Nombre legible de la tarjeta.
    fn card_name(&self) -> &str;
    /// Arranca la captura.
    fn start(&mut self) -> Result<(), AudioError>;
    /// Lee hasta `buf.len() / frame_bytes` frames entrelazados en `buf`.
    fn read(&mut self, buf: &mut [u8]) -> Result<ReadOutcome, AudioError>;
}

/// Señal sintética determinista: en cada canal `c`, un seno de `440 · (c + 1)` Hz a −12 dBFS más
/// ruido en los bits bajos, para que cada canal y cada muestra sean distinguibles.
///
/// Por defecto entrega los datos tan rápido como se le piden; con [`Self::realtime`] respeta el
/// ritmo de la frecuencia de muestreo, como una tarjeta.
#[derive(Debug, Clone)]
pub struct SyntheticSource {
    config: StreamConfig,
    total_frames: Option<u64>,
    produced: u64,
    xruns_at: Vec<u64>,
    realtime: bool,
    started: Option<Instant>,
    rng: u64,
}

impl SyntheticSource {
    /// Origen infinito con esta configuración.
    pub fn new(config: StreamConfig) -> Self {
        Self {
            config,
            total_frames: None,
            produced: 0,
            xruns_at: Vec::new(),
            realtime: false,
            started: None,
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Termina tras `frames` frames.
    pub fn with_total_frames(mut self, frames: u64) -> Self {
        self.total_frames = Some(frames);
        self
    }

    /// Simula un xrun al llegar a cada uno de estos frames.
    pub fn with_xruns_at(mut self, mut frames: Vec<u64>) -> Self {
        frames.sort_unstable();
        frames.reverse();
        self.xruns_at = frames;
        self
    }

    /// Entrega los datos al ritmo de la frecuencia de muestreo.
    pub fn realtime(mut self) -> Self {
        self.realtime = true;
        self
    }

    /// Valor de la muestra `frame` del canal `channel`, justificado a la derecha en el formato.
    /// Determinista salvo el ruido de los bits bajos (que sale de un generador con semilla fija
    /// recorrido en orden), así que dos orígenes iguales dan los mismos bytes.
    fn sample(&mut self, frame: u64, channel: u16) -> i32 {
        let rate = f64::from(self.config.rate);
        let f = 440.0 * f64::from(channel + 1);
        let v = 0.25 * (std::f64::consts::TAU * f * frame as f64 / rate).sin();
        let full = match self.config.format {
            SampleFormat::S16Le => f64::from(i16::MAX),
            SampleFormat::S24_3Le => 8_388_607.0,
            SampleFormat::S32Le => 8_388_607.0 * 256.0,
        };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let noise = (self.rng & 0xF) as i32 - 8;
        let s = (v * full) as i32;
        match self.config.format {
            // 24 bits útiles dentro de 32, como muchas tarjetas USB: los 8 bits bajos a cero.
            SampleFormat::S32Le => (s + noise * 256) & !0xFF,
            _ => s + noise,
        }
    }
}

impl CaptureSource for SyntheticSource {
    fn config(&self) -> StreamConfig {
        self.config
    }

    fn device(&self) -> &str {
        "sintético"
    }

    fn card_name(&self) -> &str {
        "Origen sintético"
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.started = Some(Instant::now());
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<ReadOutcome, AudioError> {
        let fb = self.config.frame_bytes();
        let mut frames = (buf.len() / fb).min(self.config.period_frames) as u64;
        if let Some(total) = self.total_frames {
            if self.produced >= total {
                return Ok(ReadOutcome::End);
            }
            frames = frames.min(total - self.produced);
        }
        if let Some(&x) = self.xruns_at.last() {
            if self.produced >= x {
                self.xruns_at.pop();
                return Ok(ReadOutcome::Xrun);
            }
            frames = frames.min(x - self.produced);
        }
        if let (true, Some(t0)) = (self.realtime, self.started) {
            let due = Duration::from_secs_f64(
                (self.produced + frames) as f64 / f64::from(self.config.rate),
            );
            if let Some(wait) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        let bytes = self.config.format.bytes();
        for i in 0..frames as usize {
            for c in 0..self.config.channels {
                let v = self.sample(self.produced + i as u64, c);
                let at = i * fb + usize::from(c) * bytes;
                let le = v.to_le_bytes();
                match self.config.format {
                    SampleFormat::S16Le => buf[at..at + 2].copy_from_slice(&le[..2]),
                    SampleFormat::S24_3Le => buf[at..at + 3].copy_from_slice(&le[..3]),
                    SampleFormat::S32Le => buf[at..at + 4].copy_from_slice(&le),
                }
            }
        }
        self.produced += frames;
        Ok(ReadOutcome::Frames(frames as usize))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn config(format: SampleFormat, channels: u16) -> StreamConfig {
        StreamConfig {
            rate: 96_000,
            format,
            channels,
            period_frames: 480,
        }
    }

    /// Todos los bytes de un origen finito.
    pub(crate) fn drain(mut s: SyntheticSource) -> Vec<u8> {
        let fb = s.config().frame_bytes();
        let mut buf = vec![0u8; fb * s.config().period_frames];
        let mut out = Vec::new();
        s.start().unwrap();
        loop {
            match s.read(&mut buf).unwrap() {
                ReadOutcome::Frames(n) => out.extend_from_slice(&buf[..n * fb]),
                ReadOutcome::Xrun => {}
                ReadOutcome::End => return out,
            }
        }
    }

    #[test]
    fn determinista_finito_y_con_xruns() {
        let c = config(SampleFormat::S32Le, 12);
        let a = drain(SyntheticSource::new(c).with_total_frames(10_000));
        let b = drain(
            SyntheticSource::new(c)
                .with_total_frames(10_000)
                .with_xruns_at(vec![3000]),
        );
        assert_eq!(a.len(), 10_000 * 48);
        assert_eq!(a, b);
        // 24 bits útiles en 32: el byte bajo siempre a cero; canales distintos.
        assert!(a.chunks(4).all(|s| s[0] == 0));
        assert_ne!(a[0..4], a[4..8]);
    }

    #[test]
    fn xrun_en_su_frame() {
        let c = config(SampleFormat::S16Le, 2);
        let mut s = SyntheticSource::new(c)
            .with_total_frames(2000)
            .with_xruns_at(vec![1000]);
        let mut buf = vec![0u8; 4 * 480];
        let mut seen = 0u64;
        s.start().unwrap();
        loop {
            match s.read(&mut buf).unwrap() {
                ReadOutcome::Frames(n) => seen += n as u64,
                ReadOutcome::Xrun => assert_eq!(seen, 1000),
                ReadOutcome::End => break,
            }
        }
        assert_eq!(seen, 2000);
    }
}
