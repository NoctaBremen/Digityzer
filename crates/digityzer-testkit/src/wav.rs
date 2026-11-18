//! Escritores de WAV para fixtures.
//!
//! - PCM entero de 8/16/24/32 bits y float de 32 bits: con `hound`.
//! - Float de 64 bits y `WAVE_FORMAT_EXTENSIBLE` (p. ej. 20 bits válidos en un contenedor de 24):
//!   con un escritor mínimo propio, porque hound no los cubre.
//!
//! Las muestras enteras se pasan **con signo y justificadas a la derecha** en todos los casos.
//! En 8 bits, el formato WAV guarda las muestras **sin signo** (valor + 128): hound hace esa
//! conversión al escribir.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::signals::int_range;

fn to_io(e: hound::Error) -> io::Error {
    match e {
        hound::Error::IoError(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

fn check_layout(channels: u16, len: usize) -> io::Result<()> {
    if channels == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "channels debe ser ≥ 1",
        ));
    }
    if !len.is_multiple_of(usize::from(channels)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "el número de muestras no es múltiplo del número de canales",
        ));
    }
    Ok(())
}

fn check_range(bits: u32, data: &[i32]) -> io::Result<()> {
    let (min, max) = int_range(bits);
    if let Some(v) = data
        .iter()
        .find(|&&v| i64::from(v) < min || i64::from(v) > max)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("muestra {v} fuera del rango de {bits} bits"),
        ));
    }
    Ok(())
}

/// Escribe un WAV PCM entero (`bits` ∈ {8, 16, 24, 32}) con muestras entrelazadas.
pub fn write_wav_int(
    path: &Path,
    rate: u32,
    bits: u16,
    channels: u16,
    interleaved: &[i32],
) -> io::Result<()> {
    if ![8, 16, 24, 32].contains(&bits) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bits debe ser 8, 16, 24 o 32",
        ));
    }
    check_layout(channels, interleaved.len())?;
    check_range(u32::from(bits), interleaved)?;
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: bits,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(to_io)?;
    for &s in interleaved {
        if bits == 8 {
            // hound espera i8 para 8 bits y lo convierte a sin signo al escribir.
            w.write_sample(s as i8).map_err(to_io)?;
        } else {
            w.write_sample(s).map_err(to_io)?;
        }
    }
    w.finalize().map_err(to_io)
}

/// Escribe un WAV float de 32 bits (IEEE) con muestras entrelazadas.
pub fn write_wav_f32(path: &Path, rate: u32, channels: u16, interleaved: &[f32]) -> io::Result<()> {
    check_layout(channels, interleaved.len())?;
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(to_io)?;
    for &s in interleaved {
        w.write_sample(s).map_err(to_io)?;
    }
    w.finalize().map_err(to_io)
}

/// Escribe un WAV float de 64 bits (IEEE, `WAVE_FORMAT_IEEE_FLOAT` con chunk `fact`).
pub fn write_wav_f64(path: &Path, rate: u32, channels: u16, interleaved: &[f64]) -> io::Result<()> {
    check_layout(channels, interleaved.len())?;
    let data: Vec<u8> = interleaved.iter().flat_map(|v| v.to_le_bytes()).collect();
    let frames = (interleaved.len() / usize::from(channels)) as u32;
    let fmt = FmtChunk {
        format_tag: WAVE_FORMAT_IEEE_FLOAT,
        channels,
        rate,
        container_bits: 64,
        extensible: None,
    };
    write_riff(path, &fmt, Some(frames), &data)
}

/// Escribe un WAV `WAVE_FORMAT_EXTENSIBLE` PCM entero con `valid_bits` bits válidos dentro de
/// un contenedor de `container_bits` (p. ej. 20 en 24).
///
/// Las muestras se pasan justificadas a la derecha (rango de `valid_bits`); el archivo las guarda
/// alineadas a la izquierda dentro del contenedor, como exige la especificación.
pub fn write_wav_extensible(
    path: &Path,
    rate: u32,
    container_bits: u16,
    valid_bits: u16,
    channels: u16,
    interleaved: &[i32],
) -> io::Result<()> {
    if ![16, 24, 32].contains(&container_bits) || valid_bits == 0 || valid_bits > container_bits {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "contenedor de 16/24/32 bits y 1 ≤ valid_bits ≤ container_bits",
        ));
    }
    check_layout(channels, interleaved.len())?;
    check_range(u32::from(valid_bits), interleaved)?;
    let shift = container_bits - valid_bits;
    let bytes = usize::from(container_bits / 8);
    let mut data = Vec::with_capacity(interleaved.len() * bytes);
    for &s in interleaved {
        let v = (s << shift).to_le_bytes();
        data.extend_from_slice(&v[..bytes]);
    }
    let channel_mask = match channels {
        1 => 0x4, // SPEAKER_FRONT_CENTER
        2 => 0x3, // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
        _ => 0,
    };
    let fmt = FmtChunk {
        format_tag: WAVE_FORMAT_EXTENSIBLE,
        channels,
        rate,
        container_bits,
        extensible: Some((valid_bits, channel_mask)),
    };
    write_riff(path, &fmt, None, &data)
}

const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// GUID de `KSDATAFORMAT_SUBTYPE_PCM` en el orden de bytes del archivo.
const SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

struct FmtChunk {
    format_tag: u16,
    channels: u16,
    rate: u32,
    container_bits: u16,
    /// `(valid_bits, channel_mask)` para `WAVE_FORMAT_EXTENSIBLE`.
    extensible: Option<(u16, u32)>,
}

fn write_riff(
    path: &Path,
    fmt: &FmtChunk,
    fact_frames: Option<u32>,
    data: &[u8],
) -> io::Result<()> {
    let block_align = fmt.channels * (fmt.container_bits / 8);
    let mut fmt_body = Vec::new();
    fmt_body.extend_from_slice(&fmt.format_tag.to_le_bytes());
    fmt_body.extend_from_slice(&fmt.channels.to_le_bytes());
    fmt_body.extend_from_slice(&fmt.rate.to_le_bytes());
    fmt_body.extend_from_slice(&(fmt.rate * u32::from(block_align)).to_le_bytes());
    fmt_body.extend_from_slice(&block_align.to_le_bytes());
    fmt_body.extend_from_slice(&fmt.container_bits.to_le_bytes());
    match fmt.extensible {
        Some((valid_bits, mask)) => {
            fmt_body.extend_from_slice(&22u16.to_le_bytes()); // cbSize
            fmt_body.extend_from_slice(&valid_bits.to_le_bytes());
            fmt_body.extend_from_slice(&mask.to_le_bytes());
            fmt_body.extend_from_slice(&SUBTYPE_PCM);
        }
        None => fmt_body.extend_from_slice(&0u16.to_le_bytes()), // cbSize = 0 (no PCM)
    }

    let mut chunks: Vec<([u8; 4], Vec<u8>)> = vec![(*b"fmt ", fmt_body)];
    if let Some(frames) = fact_frames {
        chunks.push((*b"fact", frames.to_le_bytes().to_vec()));
    }
    let padded = |len: usize| len + (len & 1);
    let riff_size = 4
        + chunks
            .iter()
            .map(|(_, b)| 8 + padded(b.len()))
            .sum::<usize>()
        + 8
        + padded(data.len());
    let riff_size = u32::try_from(riff_size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "WAV de más de 4 GiB"))?;

    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(b"RIFF")?;
    w.write_all(&riff_size.to_le_bytes())?;
    w.write_all(b"WAVE")?;
    for (id, body) in &chunks {
        w.write_all(id)?;
        w.write_all(&(body.len() as u32).to_le_bytes())?;
        w.write_all(body)?;
        if body.len() % 2 == 1 {
            w.write_all(&[0])?;
        }
    }
    w.write_all(b"data")?;
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    if data.len() % 2 == 1 {
        w.write_all(&[0])?;
    }
    w.flush()
}
