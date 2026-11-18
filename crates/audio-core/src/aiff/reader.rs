//! Lector AIFF/AIFF-C PCM entero.
//!
//! Sirve para **verificar** lo que escribe [`crate::aiff::AiffWriter`], así que no comparte con él la
//! codificación de muestras: aquí se decodifica con funciones propias (montaje de bytes en la
//! parte alta de un `u32` y desplazamiento aritmético a la derecha).

use std::io::{Read, Seek, SeekFrom};

use crate::aiff::error::AiffError;
use crate::aiff::extended::ext80_to_f64;
use crate::aiff::writer::AiffSpec;

/// Un chunk del archivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Identificador de 4 bytes (`COMM`, `SSND`, `ID3 `…).
    pub id: [u8; 4],
    /// Posición de la cabecera del chunk en el archivo.
    pub offset: u64,
    /// Tamaño declarado de los datos (sin la cabecera de 8 bytes ni el relleno).
    pub size: u32,
}

impl ChunkInfo {
    /// Identificador como texto (con `?` en los bytes no imprimibles).
    pub fn id_str(&self) -> String {
        self.id
            .iter()
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '?'
                }
            })
            .collect()
    }
}

/// Orden de los bytes de las muestras.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// AIFF o AIFF-C `NONE`/`twos`: big-endian.
    BigEndian,
    /// AIFF-C `sowt`: little-endian.
    LittleEndian,
    /// AIFF-C `fl32`: float IEEE de 32 bits big-endian.
    Float32,
    /// AIFF-C `fl64`: float IEEE de 64 bits big-endian.
    Float64,
}

/// Lector AIFF.
pub struct AiffReader<R: Read + Seek> {
    r: R,
    spec: AiffSpec,
    sample_rate: f64,
    frames: u32,
    compression: Compression,
    is_aifc: bool,
    chunks: Vec<ChunkInfo>,
    remaining: u64,
    raw: Vec<u8>,
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Lee hasta llenar `buf` o llegar al final. Devuelve los bytes leídos.
fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, AiffError> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(AiffError::Io(e)),
        }
    }
    Ok(n)
}

impl<R: Read + Seek> AiffReader<R> {
    /// Abre el archivo: recorre los chunks (tolera los desconocidos y un tamaño de `FORM`
    /// incoherente) y deja el lector al principio de las muestras.
    pub fn open(mut r: R) -> Result<Self, AiffError> {
        let mut head = [0u8; 12];
        if read_up_to(&mut r, &mut head)? < 12 || &head[..4] != b"FORM" {
            return Err(AiffError::Malformed("no empieza por FORM".into()));
        }
        let is_aifc = match &head[8..12] {
            b"AIFF" => false,
            b"AIFC" => true,
            _ => {
                return Err(AiffError::Malformed(
                    "el tipo de FORM no es AIFF ni AIFC".into(),
                ));
            }
        };
        let mut chunks = Vec::new();
        let mut comm: Option<(u16, u32, u16, f64, Compression)> = None;
        let mut ssnd: Option<(u64, u64)> = None; // (inicio de las muestras, bytes disponibles)
        let mut pos = 12u64;
        loop {
            let mut ch = [0u8; 8];
            // Fin de los chunks: final del archivo (o una cola de menos de 8 bytes).
            if read_up_to(&mut r, &mut ch)? < 8 {
                break;
            }
            let id = [ch[0], ch[1], ch[2], ch[3]];
            let size = be_u32(&ch[4..]);
            chunks.push(ChunkInfo {
                id,
                offset: pos,
                size,
            });
            let body = pos + 8;
            match &id {
                b"COMM" => {
                    let need = if is_aifc { 22 } else { 18 };
                    if (size as usize) < need {
                        return Err(AiffError::Malformed(format!("COMM de {size} bytes")));
                    }
                    let mut c = vec![0u8; need];
                    r.read_exact(&mut c)?;
                    let channels = be_u16(&c[0..2]);
                    let frames = be_u32(&c[2..6]);
                    let bits = be_u16(&c[6..8]);
                    let mut ext = [0u8; 10];
                    ext.copy_from_slice(&c[8..18]);
                    let rate = ext80_to_f64(&ext);
                    let compression = if is_aifc {
                        match &c[18..22] {
                            b"NONE" | b"twos" => Compression::BigEndian,
                            b"sowt" => Compression::LittleEndian,
                            b"fl32" | b"FL32" => Compression::Float32,
                            b"fl64" | b"FL64" => Compression::Float64,
                            other => {
                                return Err(AiffError::Unsupported(format!(
                                    "compresión AIFF-C «{}»",
                                    String::from_utf8_lossy(other)
                                )));
                            }
                        }
                    } else {
                        Compression::BigEndian
                    };
                    comm = Some((channels, frames, bits, rate, compression));
                }
                b"SSND" => {
                    if size < 8 {
                        return Err(AiffError::Malformed(format!("SSND de {size} bytes")));
                    }
                    let mut s = [0u8; 8];
                    r.read_exact(&mut s)?;
                    let offset = u64::from(be_u32(&s[0..4]));
                    if offset > u64::from(size) - 8 {
                        return Err(AiffError::Malformed(
                            "offset de SSND fuera del chunk".into(),
                        ));
                    }
                    ssnd = Some((body + 8 + offset, u64::from(size) - 8 - offset));
                }
                _ => {}
            }
            pos = body + u64::from(size) + u64::from(size % 2);
            r.seek(SeekFrom::Start(pos))?;
        }
        let (channels, frames, bits, rate, compression) =
            comm.ok_or_else(|| AiffError::Malformed("falta el chunk COMM".into()))?;
        let (data_start, data_len) =
            ssnd.ok_or_else(|| AiffError::Malformed("falta el chunk SSND".into()))?;
        if channels == 0 || channels > i16::MAX as u16 {
            return Err(AiffError::Malformed(format!("{channels} canales")));
        }
        match compression {
            Compression::Float32 if bits != 32 => {
                return Err(AiffError::Malformed(format!("fl32 con {bits} bits")));
            }
            Compression::Float64 if bits != 64 => {
                return Err(AiffError::Malformed(format!("fl64 con {bits} bits")));
            }
            Compression::Float32 | Compression::Float64 => {}
            _ if !(1..=32).contains(&bits) => {
                return Err(AiffError::Unsupported(format!("{bits} bits por muestra")));
            }
            _ => {}
        }
        if !(rate.is_finite() && rate > 0.0 && rate <= f64::from(u32::MAX)) {
            return Err(AiffError::Malformed(format!("frecuencia de {rate} Hz")));
        }
        let spec = AiffSpec {
            channels,
            sample_rate: rate.round() as u32,
            bits,
        };
        let need = u64::from(frames) * u64::from(channels) * spec.bytes_per_sample() as u64;
        if need > data_len {
            return Err(AiffError::Malformed(format!(
                "SSND declara {data_len} bytes y COMM pide {need}"
            )));
        }
        r.seek(SeekFrom::Start(data_start))?;
        Ok(Self {
            r,
            spec,
            sample_rate: rate,
            frames,
            compression,
            is_aifc,
            chunks,
            remaining: u64::from(frames),
            raw: Vec::new(),
        })
    }

    /// Canales, frecuencia (redondeada a Hz) y bits.
    pub fn spec(&self) -> AiffSpec {
        self.spec
    }

    /// Frecuencia exacta de `COMM`.
    pub fn sample_rate_f64(&self) -> f64 {
        self.sample_rate
    }

    /// Frames declarados en `COMM`.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Frames que quedan por leer.
    pub fn remaining_frames(&self) -> u64 {
        self.remaining
    }

    /// Orden de los bytes de las muestras.
    pub fn compression(&self) -> Compression {
        self.compression
    }

    /// `true` si las muestras son float (`fl32`/`fl64`).
    pub fn is_float(&self) -> bool {
        matches!(
            self.compression,
            Compression::Float32 | Compression::Float64
        )
    }

    /// `true` si el archivo es AIFF-C.
    pub fn is_aifc(&self) -> bool {
        self.is_aifc
    }

    /// Chunks del archivo, en orden.
    pub fn chunks(&self) -> &[ChunkInfo] {
        &self.chunks
    }

    /// Lee frames entrelazados en `buf` (tantos como quepan enteros), justificados a la
    /// derecha y con extensión de signo. Devuelve los frames leídos (0 al terminar). Si el
    /// archivo se acaba antes de lo declarado → [`AiffError::Truncated`].
    pub fn read_frames_i32(&mut self, buf: &mut [i32]) -> Result<usize, AiffError> {
        if self.is_float() {
            return Err(AiffError::Unsupported(
                "muestras float: usa read_frames_f32/f64".into(),
            ));
        }
        let ch = usize::from(self.spec.channels);
        let frames = ((buf.len() / ch) as u64).min(self.remaining) as usize;
        if frames == 0 {
            return Ok(0);
        }
        let bps = self.spec.bytes_per_sample();
        let n = frames * ch;
        self.raw.resize(n * bps, 0);
        let got = read_up_to(&mut self.r, &mut self.raw)?;
        if got < self.raw.len() {
            return Err(AiffError::Truncated);
        }
        let bits = u32::from(self.spec.bits);
        match self.compression {
            Compression::BigEndian => decode_be(&self.raw, bps, bits, &mut buf[..n]),
            Compression::LittleEndian => decode_le(&self.raw, bps, bits, &mut buf[..n]),
            // Rechazados al principio de la función.
            Compression::Float32 | Compression::Float64 => {}
        }
        self.remaining -= frames as u64;
        Ok(frames)
    }

    /// Lee los bytes crudos de hasta `buf_frames` frames (para los formatos float).
    fn read_raw(&mut self, buf_frames: usize) -> Result<usize, AiffError> {
        let frames = (buf_frames as u64).min(self.remaining) as usize;
        if frames == 0 {
            return Ok(0);
        }
        let n = frames * usize::from(self.spec.channels) * self.spec.bytes_per_sample();
        self.raw.resize(n, 0);
        if read_up_to(&mut self.r, &mut self.raw)? < n {
            return Err(AiffError::Truncated);
        }
        self.remaining -= frames as u64;
        Ok(frames)
    }

    /// Lee frames `fl32` con sus bits exactos. Devuelve los frames leídos (0 al terminar).
    pub fn read_frames_f32(&mut self, buf: &mut [f32]) -> Result<usize, AiffError> {
        if self.compression != Compression::Float32 {
            return Err(AiffError::Unsupported("el archivo no es fl32".into()));
        }
        let ch = usize::from(self.spec.channels);
        let frames = self.read_raw(buf.len() / ch)?;
        for (o, b) in buf.iter_mut().zip(self.raw.chunks_exact(4)) {
            *o = f32::from_bits(u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        }
        Ok(frames)
    }

    /// Lee frames `fl64` con sus bits exactos. Devuelve los frames leídos (0 al terminar).
    pub fn read_frames_f64(&mut self, buf: &mut [f64]) -> Result<usize, AiffError> {
        if self.compression != Compression::Float64 {
            return Err(AiffError::Unsupported("el archivo no es fl64".into()));
        }
        let ch = usize::from(self.spec.channels);
        let frames = self.read_raw(buf.len() / ch)?;
        for (o, b) in buf.iter_mut().zip(self.raw.chunks_exact(8)) {
            let mut x = [0u8; 8];
            x.copy_from_slice(b);
            *o = f64::from_bits(u64::from_be_bytes(x));
        }
        Ok(frames)
    }

    /// Lee todas las muestras que quedan.
    pub fn read_all_i32(&mut self) -> Result<Vec<i32>, AiffError> {
        let ch = usize::from(self.spec.channels);
        // Las reservas no se fían de los tamaños del archivo: se crece al leer.
        let mut out = Vec::with_capacity((self.remaining as usize).saturating_mul(ch).min(1 << 20));
        let mut buf = vec![0i32; frames_per_buffer(ch) * ch];
        loop {
            let f = self.read_frames_i32(&mut buf)?;
            if f == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..f * ch]);
        }
    }
}

/// Frames por búfer de lectura: ~1 M de muestras en total, sea cual sea el número de canales.
pub fn frames_per_buffer(channels: usize) -> usize {
    ((1 << 20) / channels.max(1)).clamp(1, 8192)
}

/// Big-endian: el primer byte va a los bits 31..24 de una palabra; después, desplazamiento
/// aritmético de `32 − bits` (descarta el relleno bajo y extiende el signo).
fn decode_be(raw: &[u8], bps: usize, bits: u32, out: &mut [i32]) {
    let drop = 32 - bits;
    for (o, s) in out.iter_mut().zip(raw.chunks_exact(bps)) {
        let mut word = 0u32;
        for (k, &b) in s.iter().enumerate() {
            word |= u32::from(b) << (24 - 8 * k as u32);
        }
        *o = (word as i32) >> drop;
    }
}

/// Little-endian (`sowt`): el último byte es el más significativo.
fn decode_le(raw: &[u8], bps: usize, bits: u32, out: &mut [i32]) {
    let drop = 32 - bits;
    for (o, s) in out.iter_mut().zip(raw.chunks_exact(bps)) {
        let mut word = 0u32;
        for (k, &b) in s.iter().rev().enumerate() {
            word |= u32::from(b) << (24 - 8 * k as u32);
        }
        *o = (word as i32) >> drop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodificacion_independiente() {
        let mut out = [0i32; 3];
        decode_be(&[0xFF, 0xF0, 0x00, 0x10, 0x80, 0x00], 2, 12, &mut out);
        assert_eq!(out, [-1, 1, -2048]);
        decode_le(&[0xF0, 0xFF, 0x10, 0x00, 0x00, 0x80], 2, 12, &mut out);
        assert_eq!(out, [-1, 1, -2048]);
        let mut o24 = [0i32; 2];
        decode_be(&[0x80, 0, 0, 0x7F, 0xFF, 0xFF], 3, 24, &mut o24);
        assert_eq!(o24, [-8_388_608, 8_388_607]);
        let mut o32 = [0i32; 1];
        decode_be(&[0x80, 0, 0, 0], 4, 32, &mut o32);
        assert_eq!(o32, [i32::MIN]);
    }
}
