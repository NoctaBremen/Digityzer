//! Ayudas compartidas por los tests de integración de `audio-core`.
//!
//! En Spector los tests decodificaban archivos con symphonia; aquí se leen los WAV de test con
//! `hound` y se trocean en [`PcmBlock`] para las funciones que reciben bloques.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use audio_core::{PcmBlock, PcmData, PcmFormat};

/// Frames por bloque al trocear un WAV (como un período de captura grande).
pub const BLOCK_FRAMES: usize = 4096;

/// Lee un WAV entero o float y lo devuelve como bloques más su formato.
pub fn read_wav_blocks(path: &Path) -> (Vec<PcmBlock>, PcmFormat) {
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    let channels = spec.channels;
    let ch = usize::from(channels);
    let data: Vec<PcmData> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let s: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
            s.chunks(BLOCK_FRAMES * ch)
                .map(|c| PcmData::Int {
                    bits: u32::from(spec.bits_per_sample),
                    samples: c.to_vec(),
                })
                .collect()
        }
        hound::SampleFormat::Float => {
            let s: Vec<f32> = r.samples::<f32>().map(Result::unwrap).collect();
            s.chunks(BLOCK_FRAMES * ch)
                .map(|c| PcmData::F32(c.to_vec()))
                .collect()
        }
    };
    let blocks: Vec<PcmBlock> = data
        .into_iter()
        .map(|d| {
            let n = match &d {
                PcmData::Int { samples, .. } => samples.len(),
                PcmData::F32(v) => v.len(),
                PcmData::F64(v) => v.len(),
            };
            PcmBlock {
                channels,
                frames: n / ch,
                data: d,
            }
        })
        .collect();
    let frames = blocks.iter().map(|b| b.frames as u64).sum();
    (
        blocks,
        PcmFormat {
            sample_rate: spec.sample_rate,
            channels,
            frames: Some(frames),
        },
    )
}
