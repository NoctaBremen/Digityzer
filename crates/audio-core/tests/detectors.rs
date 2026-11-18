//! Detectores de corte y sobremuestreo con audio sintético y, si hay ffmpeg, con MP3 reales.
//! Copiado de `spector-core/tests/detectors.rs` (solo la parte de cortes y sobremuestreo).

#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests de integración .

use std::path::Path;

mod common;

use audio_core::CancelToken;
use audio_core::analysis::{
    AnalysisParams, CutoffKind, CutoffResult, SourceEstimate, Thresholds, UpsamplingResult,
    analyze_spectrum, detect_cutoff, detect_upsampling,
};
use digityzer_testkit::*;

/// En Spector, `analyze_raw` decodificaba el archivo; aquí se lee el WAV y se pasan los bloques.
fn detect(path: &Path) -> (CutoffResult, UpsamplingResult) {
    let (blocks, format) = common::read_wav_blocks(path);
    let spectrum = analyze_spectrum(
        blocks,
        format,
        &AnalysisParams::default(),
        &CancelToken::new(),
    )
    .unwrap();
    let t = Thresholds::default();
    let c = detect_cutoff(&spectrum, &t);
    let u = detect_upsampling(&c, format.sample_rate, &t);
    (c, u)
}

/// Ruido blanco estéreo, cuantizado a 24 bits, escrito como WAV.
fn noise_file(
    dir: &Path,
    name: &str,
    rate: u32,
    secs: f64,
    process: impl Fn(Vec<f64>) -> Vec<f64>,
) -> std::path::PathBuf {
    let chans: Vec<Vec<i32>> = (0..2)
        .map(|c| quantize(&process(white_noise(rate, secs, 0.3, 40 + c)), 24))
        .collect();
    let p = dir.join(name);
    write_wav_int(&p, rate, 24, 2, &interleave(&chans)).unwrap();
    p
}

fn kbps(s: SourceEstimate) -> Option<u32> {
    match s {
        SourceEstimate::Lossy { kbps, .. } => Some(kbps),
        _ => None,
    }
}

#[test]
fn ruido_filtrado_a_16_khz_parece_mp3_de_128() {
    let dir = temp_dir();
    let p = noise_file(dir.path(), "16k.wav", 44_100, 6.0, |s| {
        lowpass_brickwall(&s, 44_100, 16_000.0)
    });
    let (c, u) = detect(&p);
    assert_eq!(c.kind, CutoffKind::Cutoff);
    assert!(
        (15_700.0..=16_300.0).contains(&c.cutoff_hz),
        "corte {}",
        c.cutoff_hz
    );
    assert!(c.cliff, "pendiente {}", c.slope_db);
    assert_eq!(kbps(c.estimated_source), Some(128));
    assert_eq!(u, UpsamplingResult::NotApplicable);
}

#[test]
fn ruido_filtrado_a_19_y_19_6_khz() {
    // Cortes medidos de LAME: 192 kbps → 18,9 kHz; 224/256 kbps → 19,6 kHz.
    let dir = temp_dir();
    for (cut, expected) in [(19_000.0, 192), (19_600.0, 256)] {
        let p = noise_file(dir.path(), &format!("{cut}.wav"), 48_000, 6.0, |s| {
            lowpass_brickwall(&s, 48_000, cut)
        });
        let (c, _) = detect(&p);
        assert!(c.cliff);
        assert!(
            (c.cutoff_hz - cut).abs() <= 300.0,
            "corte {} (filtro {cut})",
            c.cutoff_hz
        );
        assert_eq!(kbps(c.estimated_source), Some(expected), "filtro {cut}");
    }
}

#[test]
fn ruido_sin_filtrar_es_banda_completa() {
    let dir = temp_dir();
    for rate in [44_100u32, 48_000, 96_000] {
        let p = noise_file(dir.path(), &format!("full{rate}.wav"), rate, 4.0, |s| s);
        let (c, u) = detect(&p);
        assert_eq!(c.kind, CutoffKind::FullBand, "{rate}: {c:?}");
        assert_eq!(c.estimated_source, SourceEstimate::LosslessOrHigh);
        if rate == 96_000 {
            assert_eq!(u, UpsamplingResult::Genuine);
        }
    }
}

#[test]
fn sobremuestreo_de_44_1_a_96() {
    let dir = temp_dir();
    let p = noise_file(dir.path(), "up.wav", 96_000, 4.0, |s| {
        // La señal "nace" a 44,1 kHz (filtrada a 20 kHz) y se sobremuestrea a 96 kHz.
        let at_44 = lowpass_brickwall(&s[..s.len() * 441 / 960], 44_100, 20_000.0);
        let mut up = upsample_from(&at_44, 44_100, 96_000);
        up.resize(s.len(), 0.0);
        up
    });
    let (c, u) = detect(&p);
    assert_eq!(c.kind, CutoffKind::Cutoff);
    assert!(
        (19_500.0..=20_500.0).contains(&c.cutoff_hz),
        "{}",
        c.cutoff_hz
    );
    assert!(
        matches!(
            u,
            UpsamplingResult::Upsampled {
                from_rate_guess: 44_100,
                ..
            }
        ),
        "{u:?}"
    );
}

#[test]
fn mp3_de_128_reconvertido_a_96_dispara_los_dos() {
    let dir = temp_dir();
    let p = noise_file(dir.path(), "mp3up.wav", 96_000, 4.0, |s| {
        lowpass_brickwall(&s, 96_000, 16_000.0)
    });
    let (c, u) = detect(&p);
    assert_eq!(kbps(c.estimated_source), Some(128));
    assert!(matches!(
        u,
        UpsamplingResult::Upsampled {
            from_rate_guess: 44_100,
            ..
        }
    ));
}

#[test]
fn caida_suave_no_es_acantilado() {
    let dir = temp_dir();
    // Tres polos de paso bajo a 1 kHz en cascada: −18 dB/octava, sin acantilado.
    let gentle = |s: Vec<f64>| {
        let a = 1.0 - (-std::f64::consts::TAU * 1_000.0 / 44_100.0).exp();
        let mut out = s;
        for _ in 0..3 {
            let mut y = 0.0;
            for v in out.iter_mut() {
                y += a * (*v - y);
                *v = y;
            }
        }
        out.iter().map(|v| v * 4.0).collect()
    };
    let p = noise_file(dir.path(), "suave.wav", 44_100, 6.0, gentle);
    let (c, _) = detect(&p);
    assert!(!c.cliff, "{c:?}");
    assert!(matches!(
        c.estimated_source,
        SourceEstimate::BandLimited | SourceEstimate::LosslessOrHigh
    ));
}

#[test]
fn silencio_sin_contenido() {
    let dir = temp_dir();
    let p = noise_file(dir.path(), "sil.wav", 96_000, 1.0, |s| vec![0.0; s.len()]);
    let (c, u) = detect(&p);
    assert_eq!(c.kind, CutoffKind::NoContent);
    assert_eq!(u, UpsamplingResult::NotApplicable);
}

#[test]
fn mp3_reales_con_ffmpeg() {
    require_ffmpeg!();
    let dir = temp_dir();
    let src = noise_file(dir.path(), "src.wav", 44_100, 20.0, |s| s);
    for (bitrate, expected) in [
        ("128k", &[128u32][..]),
        ("192k", &[192][..]),
        ("256k", &[256][..]),
        ("320k", &[320][..]),
    ] {
        let mp3 = dir.path().join(format!("x{bitrate}.mp3"));
        ffmpeg_encode(&src, &mp3, &["-c:a", "libmp3lame", "-b:a", bitrate]).unwrap();
        // Como lo haría un "falso sin pérdida": decodificado y guardado en WAV de 16 bits
        // (en Spector era FLAC; aquí los tests leen WAV).
        let wav = dir.path().join(format!("x{bitrate}.wav"));
        ffmpeg_encode(&mp3, &wav, &["-c:a", "pcm_s16le"]).unwrap();
        let (c, _) = detect(&wav);
        println!("MP3 {bitrate}: {c:?}");
        let est = c.estimated_source;
        let ok = match kbps(est) {
            Some(k) => expected.contains(&k),
            None => bitrate == "320k" && est == SourceEstimate::LosslessOrHigh,
        };
        assert!(ok, "MP3 {bitrate}: estimado {}", est.label());
        assert!(c.cliff, "MP3 {bitrate}: pendiente {}", c.slope_db);
    }
}

// ---------------------------------------------------------------------------------------------
// Profundidad de bits y clipping (F2-T3)
// ---------------------------------------------------------------------------------------------
