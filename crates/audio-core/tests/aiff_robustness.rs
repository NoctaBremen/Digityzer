//! Robustez del lector AIFF: ninguna corrupción provoca pánicos, bucles ni reservas enormes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;
use std::time::Duration;

use audio_core::aiff::{AiffReader, AiffSpec, AiffWriter};
use digityzer_testkit::corrupt::{Kind, corruptions, limit_address_space, run_with_timeout};
use proptest::prelude::*;

fn valid(bits: u16, channels: u16) -> Vec<u8> {
    let spec = AiffSpec {
        channels,
        sample_rate: 48_000,
        bits,
    };
    let half = 1i64 << (bits - 1);
    let s: Vec<i32> = (0..3000i64)
        .map(|i| ((i * 7919) % (2 * half) - half) as i32)
        .collect();
    let s = &s[..s.len() / usize::from(channels) * usize::from(channels)];
    let mut w = AiffWriter::new(Cursor::new(Vec::new()), spec).unwrap();
    w.write_frames_i32(s).unwrap();
    w.append_chunk(*b"ID3 ", vec![1, 2, 3]);
    w.finalize().unwrap().into_inner()
}

fn read_everything(bytes: Vec<u8>) -> Result<usize, String> {
    let mut r = AiffReader::open(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let _ = r.chunks().len();
    r.read_all_i32().map(|v| v.len()).map_err(|e| e.to_string())
}

#[test]
fn corrupciones_sin_panicos() {
    limit_address_space(3 << 30);
    let mut failures = Vec::new();
    let mut n = 0;
    for (bits, ch) in [(16u16, 2u16), (24, 1), (8, 6), (20, 2), (32, 2)] {
        for (name, bytes) in corruptions(Kind::Aiff, &valid(bits, ch), u64::from(bits)) {
            n += 1;
            if let Err(e) =
                run_with_timeout(Duration::from_secs(10), move || read_everything(bytes))
            {
                failures.push(format!("{bits} bits/{ch} canales, {name}: {e}"));
            }
        }
    }
    // Muchos canales declarados con un SSND coherente y enorme: no debe reservar de golpe.
    let mut huge = valid(16, 2);
    huge[20..22].copy_from_slice(&32_767u16.to_be_bytes());
    huge[22..26].copy_from_slice(&60_000u32.to_be_bytes());
    huge[42..46].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
    if let Err(e) = run_with_timeout(Duration::from_secs(10), move || read_everything(huge)) {
        failures.push(format!("32.767 canales: {e}"));
    }
    assert!(n > 150);
    assert!(failures.is_empty(), "fallos:\n{}", failures.join("\n"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Fuzzing ligero: cabecera válida + bytes aleatorios (`cargo test -- --ignored`).
    #[test]
    #[ignore = "fuzzing ligero: lento"]
    fn fuzz_cabecera_valida_y_bytes_aleatorios(
        tail in proptest::collection::vec(any::<u8>(), 0..4096),
        cut in 0usize..80,
        flips in proptest::collection::vec((0usize..80, 0u8..8), 0..6),
    ) {
        let mut b = valid(16, 2);
        b.truncate(cut.max(12));
        for (i, bit) in flips {
            if i < b.len() {
                b[i] ^= 1 << bit;
            }
        }
        b.extend_from_slice(&tail);
        let _ = read_everything(b);
    }
}
