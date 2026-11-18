//! Round trip exacto del escritor y el lector AIFF, cabecera byte a byte, errores e
//! interoperabilidad con ffmpeg.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use audio_core::aiff::{AiffError, AiffReader, AiffSpec, AiffWriter, Compression};
use digityzer_testkit::{Rng, ffmpeg_encode, require_ffmpeg, temp_dir};
use proptest::prelude::*;

const RATES: [u32; 11] = [
    8_000, 11_025, 22_050, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000,
];

fn write(spec: AiffSpec, samples: &[i32]) -> Vec<u8> {
    let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec).unwrap();
    // En dos llamadas, para probar el streaming.
    let ch = usize::from(spec.channels);
    let half = samples.len() / ch / 2 * ch;
    w.write_frames_i32(&samples[..half]).unwrap();
    w.write_frames_i32(&samples[half..]).unwrap();
    w.finalize().unwrap().into_inner()
}

fn read(bytes: Vec<u8>) -> (AiffSpec, Vec<i32>) {
    let mut r = AiffReader::open(Cursor::new(bytes)).unwrap();
    let spec = r.spec();
    (spec, r.read_all_i32().unwrap())
}

/// Valores aleatorios en el rango de `bits` más los extremos.
fn samples(bits: u16, channels: u16, frames: usize, seed: u64) -> Vec<i32> {
    let half = 1i64 << (bits - 1);
    let (min, max) = (-half, half - 1);
    let mut rng = Rng::new(seed);
    let mut v: Vec<i32> = [min, max, 0, -1]
        .iter()
        .flat_map(|&x| std::iter::repeat_n(x as i32, usize::from(channels)))
        .collect();
    let span = (max - min + 1) as u64;
    while v.len() < frames * usize::from(channels) {
        v.push((min + (rng.next_u64() % span) as i64) as i32);
    }
    v
}

#[test]
fn matriz_bits_canales_frecuencias() {
    let mut seed = 1;
    for bits in [8u16, 12, 16, 20, 24, 32] {
        for channels in [1u16, 2, 6] {
            for rate in RATES {
                let spec = AiffSpec {
                    channels,
                    sample_rate: rate,
                    bits,
                };
                let s = samples(bits, channels, 301, seed);
                seed += 1;
                let (back_spec, back) = read(write(spec, &s));
                assert_eq!(back_spec, spec);
                assert_eq!(back, s, "{bits} bits, {channels} canales, {rate} Hz");
            }
        }
    }
}

#[test]
fn todas_las_profundidades_de_1_a_32() {
    for bits in 1u16..=32 {
        let spec = AiffSpec {
            channels: 2,
            sample_rate: 44_100,
            bits,
        };
        let s = samples(bits, 2, 64, u64::from(bits));
        assert_eq!(read(write(spec, &s)).1, s, "{bits} bits");
    }
}

#[test]
fn cabecera_byte_a_byte() {
    let spec = AiffSpec {
        channels: 2,
        sample_rate: 44_100,
        bits: 16,
    };
    let bytes = write(spec, &[1, -2]);
    #[rustfmt::skip]
    let expected: Vec<u8> = [
        b"FORM".as_slice(), &[0x00, 0x00, 0x00, 0x32], b"AIFF",
        b"COMM", &[0x00, 0x00, 0x00, 0x12],
        &[0x00, 0x02],                    // canales
        &[0x00, 0x00, 0x00, 0x01],        // frames
        &[0x00, 0x10],                    // bits
        &[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0], // 44.100 Hz
        b"SSND", &[0x00, 0x00, 0x00, 0x0C],
        &[0, 0, 0, 0], &[0, 0, 0, 0],     // offset, blockSize
        &[0x00, 0x01, 0xFF, 0xFE],        // 1, −2
    ]
    .concat();
    assert_eq!(bytes, expected);
}

#[test]
fn cero_frames_y_chunks_adicionales() {
    let spec = AiffSpec {
        channels: 2,
        sample_rate: 48_000,
        bits: 24,
    };
    let w = AiffWriter::new(Cursor::new(Vec::new()), spec).unwrap();
    let bytes = w.finalize().unwrap().into_inner();
    let mut r = AiffReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(r.frames(), 0);
    assert!(r.read_all_i32().unwrap().is_empty());

    let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec).unwrap();
    w.write_frames_i32(&[5, -5]).unwrap();
    w.append_chunk(*b"ID3 ", b"abc".to_vec()); // impar → relleno
    w.append_chunk(*b"COMT", vec![0; 4]);
    let bytes = w.finalize().unwrap().into_inner();
    let form = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    assert_eq!(form, bytes.len() - 8);
    let mut r = AiffReader::open(Cursor::new(bytes)).unwrap();
    let ids: Vec<String> = r.chunks().iter().map(|c| c.id_str()).collect();
    assert_eq!(ids, ["COMM", "SSND", "ID3 ", "COMT"]);
    assert_eq!(r.chunks()[2].size, 3);
    assert_eq!(r.read_all_i32().unwrap(), [5, -5]);
}

/// AIFF-C a mano: compresión, chunks desconocidos antes de los datos y offset en SSND.
fn aifc(compression: &[u8; 4], data: &[u8], ssnd_offset: u32) -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&1u16.to_be_bytes());
    comm.extend_from_slice(&((data.len() / 2) as u32).to_be_bytes());
    comm.extend_from_slice(&16u16.to_be_bytes());
    comm.extend_from_slice(&audio_core::aiff::f64_to_ext80(44_100.0));
    comm.extend_from_slice(compression);
    comm.extend_from_slice(&[0, 0]); // pstring vacía + relleno
    let mut body = b"AIFC".to_vec();
    let mut chunk = |id: &[u8; 4], d: &[u8]| {
        body.extend_from_slice(id);
        body.extend_from_slice(&(d.len() as u32).to_be_bytes());
        body.extend_from_slice(d);
        if d.len() % 2 == 1 {
            body.push(0);
        }
    };
    chunk(b"FVER", &[0xA2, 0x80, 0x51, 0x40]);
    chunk(b"COMM", &comm);
    chunk(b"APPL", b"xyz");
    let mut ssnd = ssnd_offset.to_be_bytes().to_vec();
    ssnd.extend_from_slice(&0u32.to_be_bytes());
    ssnd.extend(std::iter::repeat_n(0xEE, ssnd_offset as usize));
    ssnd.extend_from_slice(data);
    chunk(b"SSND", &ssnd);
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn aifc_sowt_none_y_offset() {
    let mut r = AiffReader::open(Cursor::new(aifc(b"sowt", &[0x01, 0x00, 0xFE, 0xFF], 0))).unwrap();
    assert!(r.is_aifc());
    assert_eq!(r.compression(), Compression::LittleEndian);
    assert_eq!(r.read_all_i32().unwrap(), [1, -2]);
    let mut r = AiffReader::open(Cursor::new(aifc(b"NONE", &[0x00, 0x01, 0xFF, 0xFE], 6))).unwrap();
    assert_eq!(r.compression(), Compression::BigEndian);
    assert_eq!(r.read_all_i32().unwrap(), [1, -2]);
    for codec in [b"ulaw", b"raw ", b"ima4"] {
        assert!(matches!(
            AiffReader::open(Cursor::new(aifc(codec, &[0, 0], 0))),
            Err(AiffError::Unsupported(_))
        ));
    }
    // fl32 se admite (F5-T3), pero no con 16 bits.
    assert!(matches!(
        AiffReader::open(Cursor::new(aifc(b"fl32", &[0, 0], 0))),
        Err(AiffError::Malformed(_))
    ));
}

#[test]
fn errores_sin_panico() {
    let spec = AiffSpec {
        channels: 2,
        sample_rate: 44_100,
        bits: 16,
    };
    let good = write(spec, &samples(16, 2, 100, 7));

    // Truncado en medio de los datos: las cabeceras son coherentes, pero el archivo se acaba
    // antes → error al leer (no al abrir).
    let cut = good[..good.len() - 10].to_vec();
    let mut r = AiffReader::open(Cursor::new(cut)).unwrap();
    assert!(matches!(r.read_all_i32(), Err(AiffError::Truncated)));

    // SSND declarado más largo que el archivo y COMM coherente con él.
    let mut long = good.clone();
    long[42..46].copy_from_slice(&10_000u32.to_be_bytes());
    long[22..26].copy_from_slice(&2_000u32.to_be_bytes());
    let mut r = AiffReader::open(Cursor::new(long)).unwrap();
    assert!(matches!(r.read_all_i32(), Err(AiffError::Truncated)));

    // COMM pide más frames de los que caben en SSND.
    let mut incoherent = good.clone();
    incoherent[22..26].copy_from_slice(&101u32.to_be_bytes());
    assert!(matches!(
        AiffReader::open(Cursor::new(incoherent)),
        Err(AiffError::Malformed(_))
    ));

    // FORM con un tamaño absurdo: se ignora y se lee igual.
    let mut form = good.clone();
    form[4..8].copy_from_slice(&7u32.to_be_bytes());
    assert_eq!(read(form).1, samples(16, 2, 100, 7));

    // Sin COMM / sin SSND / basura.
    assert!(AiffReader::open(Cursor::new(good[..38].to_vec())).is_err());
    assert!(AiffReader::open(Cursor::new(b"RIFF....WAVE".to_vec())).is_err());
    assert!(AiffReader::open(Cursor::new(Vec::new())).is_err());
    for n in 0..good.len().min(60) {
        let _ = AiffReader::open(Cursor::new(good[..n].to_vec())).map(|mut r| r.read_all_i32());
    }
}

#[test]
fn interoperabilidad_con_ffmpeg() {
    require_ffmpeg!();
    let dir = temp_dir();
    for (bits, rate) in [(16u16, 44_100u32), (24, 96_000), (32, 48_000), (8, 22_050)] {
        let spec = AiffSpec {
            channels: 2,
            sample_rate: rate,
            bits,
        };
        let s = samples(bits, 2, 4000, u64::from(bits));
        let src = dir.path().join(format!("x{bits}.aiff"));
        std::fs::write(&src, write(spec, &s)).unwrap();
        let raw = dir.path().join(format!("x{bits}.raw"));
        ffmpeg_encode(&src, &raw, &["-f", "s32le", "-c:a", "pcm_s32le"]).unwrap();
        let bytes = std::fs::read(&raw).unwrap();
        let decoded: Vec<i32> = bytes
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> (32 - bits))
            .collect();
        assert_eq!(decoded, s, "{bits} bits");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn round_trip_aleatorio(
        bits in 1u16..=32,
        channels in 1u16..=8,
        frames in 0usize..300,
        seed in any::<u64>(),
        rate in 1u32..=800_000,
    ) {
        let spec = AiffSpec { channels, sample_rate: rate, bits };
        let s: Vec<i32> = samples(bits, channels, frames, seed)
            .into_iter()
            .take(frames * usize::from(channels))
            .collect();
        let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec).unwrap();
        w.write_frames_i32(&s).unwrap();
        let bytes = w.finalize().unwrap().into_inner();
        let (back_spec, back) = read(bytes);
        prop_assert_eq!(back_spec, spec);
        prop_assert_eq!(back, s);
    }
}
