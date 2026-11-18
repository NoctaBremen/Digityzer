//! Escritor AIFF PCM entero en streaming.
//!
//! Estructura: `FORM`/`AIFF` → `COMM` → `SSND` (datos) → chunks adicionales. La cabecera se escribe
//! provisional al crear el escritor y [`AiffWriter::finalize`] parchea los tamaños.

use std::io::{Seek, SeekFrom, Write};

use crate::aiff::error::AiffError;
use crate::aiff::extended::f64_to_ext80;

/// Parámetros del audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiffSpec {
    /// Canales (≥ 1).
    pub channels: u16,
    /// Frecuencia de muestreo en Hz (> 0).
    pub sample_rate: u32,
    /// Profundidad (`sampleSize`) en bits, de 1 a 32.
    pub bits: u16,
}

impl AiffSpec {
    /// Bytes por muestra: `ceil(bits / 8)`.
    pub fn bytes_per_sample(&self) -> usize {
        usize::from(self.bits).div_ceil(8)
    }

    /// Comprueba que los parámetros son válidos para escribir.
    pub fn validate(&self) -> Result<(), AiffError> {
        if !(1..=32).contains(&self.bits) {
            return Err(AiffError::InvalidSpec(format!(
                "profundidad de {} bits (admitida: 1–32)",
                self.bits
            )));
        }
        if self.channels == 0 || self.channels > i16::MAX as u16 {
            return Err(AiffError::InvalidSpec(format!(
                "{} canales (admitidos: 1–32.767)",
                self.channels
            )));
        }
        if self.sample_rate == 0 {
            return Err(AiffError::InvalidSpec("frecuencia de 0 Hz".into()));
        }
        Ok(())
    }
}

/// Posición del tamaño de `FORM`.
const FORM_SIZE_POS: u64 = 4;
/// Posición de `numSampleFrames` en `COMM`.
const COMM_FRAMES_POS: u64 = 22;
/// Posición del tamaño de `SSND`.
const SSND_SIZE_POS: u64 = 42;
/// Bytes de cabecera antes de los datos de `SSND`.
pub const HEADER_LEN: u64 = 54;

/// Escritor AIFF.
pub struct AiffWriter<W: Write + Seek> {
    w: W,
    spec: AiffSpec,
    data_bytes: u64,
    extra: Vec<([u8; 4], Vec<u8>)>,
    buf: Vec<u8>,
    min: i64,
    max: i64,
    shift: u32,
}

impl<W: Write + Seek> AiffWriter<W> {
    /// Valida `spec` y escribe la cabecera provisional en la posición actual de `w` (que debe
    /// ser el principio del archivo).
    pub fn new(mut w: W, spec: AiffSpec) -> Result<Self, AiffError> {
        spec.validate()?;
        let bytes = spec.bytes_per_sample() as u32;
        let mut h = Vec::with_capacity(HEADER_LEN as usize);
        h.extend_from_slice(b"FORM");
        h.extend_from_slice(&0u32.to_be_bytes());
        h.extend_from_slice(b"AIFF");
        h.extend_from_slice(b"COMM");
        h.extend_from_slice(&18u32.to_be_bytes());
        h.extend_from_slice(&(spec.channels as i16).to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes());
        h.extend_from_slice(&(spec.bits as i16).to_be_bytes());
        h.extend_from_slice(&f64_to_ext80(f64::from(spec.sample_rate)));
        h.extend_from_slice(b"SSND");
        h.extend_from_slice(&8u32.to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes()); // offset
        h.extend_from_slice(&0u32.to_be_bytes()); // blockSize
        debug_assert_eq!(h.len() as u64, HEADER_LEN);
        w.write_all(&h)?;
        let half = 1i64 << (spec.bits - 1);
        Ok(Self {
            w,
            spec,
            data_bytes: 0,
            extra: Vec::new(),
            buf: Vec::new(),
            min: -half,
            max: half - 1,
            shift: 8 * bytes - u32::from(spec.bits),
        })
    }

    /// Parámetros del audio.
    pub fn spec(&self) -> AiffSpec {
        self.spec
    }

    /// Frames escritos hasta ahora.
    pub fn frames_written(&self) -> u64 {
        self.data_bytes / (self.spec.bytes_per_sample() as u64 * u64::from(self.spec.channels))
    }

    /// Escribe muestras entrelazadas, justificadas a la derecha. La longitud debe ser múltiplo
    /// de los canales y cada valor debe caber en `bits` (si no, error y no se escribe nada de
    /// esta llamada).
    pub fn write_frames_i32(&mut self, samples: &[i32]) -> Result<(), AiffError> {
        let ch = usize::from(self.spec.channels);
        if !samples.len().is_multiple_of(ch) {
            return Err(AiffError::BadLength {
                samples: samples.len(),
                channels: self.spec.channels,
            });
        }
        if let Some(&bad) = samples
            .iter()
            .find(|&&v| i64::from(v) < self.min || i64::from(v) > self.max)
        {
            return Err(AiffError::SampleOutOfRange {
                value: bad,
                bits: self.spec.bits,
            });
        }
        let bytes = self.spec.bytes_per_sample();
        let add = (samples.len() * bytes) as u64;
        // SSND = 8 (offset + blockSize) + datos; FORM = 4 + COMM (26) + SSND (8 + tamaño) + resto.
        if self.data_bytes + add + HEADER_LEN > u64::from(u32::MAX) {
            return Err(AiffError::TooLarge);
        }
        self.buf.clear();
        self.buf.reserve(samples.len() * bytes);
        for &v in samples {
            // Alineación a la izquierda: los bits bajos de relleno quedan a 0.
            let word = ((i64::from(v) << self.shift) as u32).to_be_bytes();
            self.buf.extend_from_slice(&word[4 - bytes..]);
        }
        self.w.write_all(&self.buf)?;
        self.data_bytes += add;
        Ok(())
    }

    /// Añade un chunk que se escribirá después de `SSND` al finalizar (p. ej. `ID3 `).
    pub fn append_chunk(&mut self, id: [u8; 4], data: Vec<u8>) {
        self.extra.push((id, data));
    }

    /// Termina el archivo: relleno de `SSND`, chunks adicionales y tamaños definitivos.
    /// Devuelve el escritor interno (con los datos ya enviados con `flush`).
    pub fn finalize(mut self) -> Result<W, AiffError> {
        let ssnd_size = 8 + self.data_bytes;
        let mut total = HEADER_LEN + self.data_bytes;
        if self.data_bytes % 2 == 1 {
            self.w.write_all(&[0])?;
            total += 1;
        }
        for (id, data) in std::mem::take(&mut self.extra) {
            let len = data.len() as u64;
            total += 8 + len + (len % 2);
            if total > u64::from(u32::MAX) || len > u64::from(u32::MAX) {
                return Err(AiffError::TooLarge);
            }
            self.w.write_all(&id)?;
            self.w.write_all(&(len as u32).to_be_bytes())?;
            self.w.write_all(&data)?;
            if len % 2 == 1 {
                self.w.write_all(&[0])?;
            }
        }
        let form_size = total - 8;
        let frames = self.frames_written();
        if form_size > u64::from(u32::MAX) || frames > u64::from(u32::MAX) {
            return Err(AiffError::TooLarge);
        }
        let end = self.w.stream_position()?;
        let base = end - total;
        self.w.seek(SeekFrom::Start(base + FORM_SIZE_POS))?;
        self.w.write_all(&(form_size as u32).to_be_bytes())?;
        self.w.seek(SeekFrom::Start(base + COMM_FRAMES_POS))?;
        self.w.write_all(&(frames as u32).to_be_bytes())?;
        self.w.seek(SeekFrom::Start(base + SSND_SIZE_POS))?;
        self.w.write_all(&(ssnd_size as u32).to_be_bytes())?;
        self.w.seek(SeekFrom::Start(end))?;
        self.w.flush()?;
        Ok(self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn spec(bits: u16, channels: u16) -> AiffSpec {
        AiffSpec {
            channels,
            sample_rate: 44_100,
            bits,
        }
    }

    #[test]
    fn valida_parametros_y_rangos() {
        for (b, c, r) in [(0, 2, 44_100), (33, 2, 44_100), (16, 0, 44_100), (16, 2, 0)] {
            let s = AiffSpec {
                channels: c,
                sample_rate: r,
                bits: b,
            };
            assert!(matches!(
                AiffWriter::new(Cursor::new(Vec::new()), s),
                Err(AiffError::InvalidSpec(_))
            ));
        }
        let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec(12, 2)).unwrap();
        assert!(matches!(
            w.write_frames_i32(&[1, 2, 3]),
            Err(AiffError::BadLength { .. })
        ));
        assert!(matches!(
            w.write_frames_i32(&[2047, 2048]),
            Err(AiffError::SampleOutOfRange {
                value: 2048,
                bits: 12
            })
        ));
        assert!(w.write_frames_i32(&[-2048, 2047]).is_ok());
        assert_eq!(w.frames_written(), 1);
    }

    #[test]
    fn alineacion_a_la_izquierda() {
        let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec(12, 1)).unwrap();
        w.write_frames_i32(&[-1, 1, -2048]).unwrap();
        let out = w.finalize().unwrap().into_inner();
        let data = &out[HEADER_LEN as usize..HEADER_LEN as usize + 6];
        assert_eq!(data, [0xFF, 0xF0, 0x00, 0x10, 0x80, 0x00]);
        // 8 bits: con signo, un byte; número impar de bytes → relleno.
        let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec(8, 1)).unwrap();
        w.write_frames_i32(&[-128, 127, -1]).unwrap();
        let out = w.finalize().unwrap().into_inner();
        assert_eq!(&out[54..], [0x80, 0x7F, 0xFF, 0x00]);
        assert_eq!(u32::from_be_bytes([out[42], out[43], out[44], out[45]]), 11);
        assert_eq!(
            u32::from_be_bytes([out[4], out[5], out[6], out[7]]) as usize,
            out.len() - 8
        );
    }
}
