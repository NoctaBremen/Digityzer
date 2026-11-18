//! Espectrograma de archivos sintéticos: escala en dBFS, posición en frecuencia, cortes
//! abruptos, resolución, cancelación y progreso.

#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests de integración (ver CLAUDE.md §8).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod common;

use audio_core::{
    CancelToken, ChannelView, CoreError, DbGrid, NoProgress, Progress, SpectrogramData,
    SpectrogramParams, WindowKind,
};
use digityzer_testkit::*;

/// `compute_spectrogram` de Spector recibía una ruta; aquí se lee el WAV y se pasan los bloques.
fn compute_spectrogram(
    path: &Path,
    params: &SpectrogramParams,
    cancel: &CancelToken,
    progress: &dyn Progress,
) -> Result<SpectrogramData, CoreError> {
    let (blocks, format) = common::read_wav_blocks(path);
    audio_core::compute_spectrogram(blocks, format, params, cancel, progress)
}

fn wav(dir: &Path, name: &str, rate: u32, channels: u16, signal: &[f64]) -> PathBuf {
    let chans: Vec<Vec<f64>> = (0..channels).map(|_| signal.to_vec()).collect();
    let p = dir.join(name);
    write_wav_int(&p, rate, 24, channels, &quantize(&interleave(&chans), 24)).unwrap();
    p
}

fn spec(path: &Path, params: &SpectrogramParams) -> SpectrogramData {
    compute_spectrogram(path, params, &CancelToken::new(), &NoProgress).unwrap()
}

/// Media temporal (en potencia) de cada fila, en dB.
fn row_means_db(s: &SpectrogramData) -> Vec<f32> {
    (0..s.rows)
        .map(|r| {
            let mean: f64 = (0..s.columns)
                .map(|c| 10f64.powf(f64::from(s.db_at(c, r)) / 10.0))
                .sum::<f64>()
                / s.columns as f64;
            (10.0 * mean.max(1e-20).log10()) as f32
        })
        .collect()
}

fn band_db(s: &SpectrogramData, means: &[f32], lo: f64, hi: f64) -> f32 {
    let (a, b) = (s.row_for_hz(lo), s.row_for_hz(hi));
    let lin: f64 = means[a..=b]
        .iter()
        .map(|&d| 10f64.powf(f64::from(d) / 10.0))
        .sum::<f64>()
        / (b - a + 1) as f64;
    (10.0 * lin.max(1e-20).log10()) as f32
}

#[test]
fn seno_en_su_fila_y_a_su_nivel() {
    let dir = temp_dir();
    let params = SpectrogramParams::default();
    for (freq, amp, want_db, tol) in [
        // 1.500 Hz cae justo en un bin (FFT 8.192 a 48 kHz): sin pérdida por festoneado.
        (1500.0, 1.0, 0.0, 0.5),
        (1500.0, 0.1, -20.0, 0.5),
        // 1 kHz cae entre bins: la ventana Hann pierde hasta ~1,4 dB.
        (1000.0, 1.0, 0.0, 1.5),
        (1000.0, 0.1, -20.0, 1.5),
    ] {
        let p = wav(
            dir.path(),
            &format!("s{freq}_{amp}.wav"),
            48_000,
            2,
            &sine(freq, 48_000, 3.0, amp),
        );
        let s = spec(&p, &params);
        let means = row_means_db(&s);
        let (peak_row, peak_db) = means
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |m, (i, &d)| if d > m.1 { (i, d) } else { m });
        let want_row = s.row_for_hz(freq);
        assert!(
            peak_row.abs_diff(want_row) <= 1,
            "{freq} Hz: fila {peak_row} (esperada {want_row})"
        );
        assert!(
            (peak_db - want_db).abs() <= tol,
            "{freq} Hz, amp {amp}: {peak_db} dB"
        );
    }
}

#[test]
fn corte_de_mp3_a_16_khz_visible() {
    let dir = temp_dir();
    let noise = lowpass_brickwall(&white_noise(44_100, 8.0, 0.5, 21), 44_100, 16_000.0);
    let p = wav(dir.path(), "mp3like.wav", 44_100, 2, &noise);
    for window in [WindowKind::Hann, WindowKind::BlackmanHarris] {
        let s = spec(
            &p,
            &SpectrogramParams {
                window,
                ..Default::default()
            },
        );
        let means = row_means_db(&s);
        let pass = band_db(&s, &means, 1_000.0, 15_000.0);
        let stop = band_db(&s, &means, 16_500.0, 21_500.0);
        assert!(
            pass - stop >= 60.0,
            "{window:?}: pasante {pass:.1} dB, sobre el corte {stop:.1} dB"
        );
    }
}

#[test]
fn resolucion_y_duracion() {
    let dir = temp_dir();
    let p = wav(
        dir.path(),
        "largo.wav",
        48_000,
        1,
        &white_noise(48_000, 30.0, 0.3, 2),
    );
    for max_columns in [64usize, 256, 1000] {
        let params = SpectrogramParams {
            max_columns,
            max_rows: 512,
            ..Default::default()
        };
        let s = spec(&p, &params);
        // Frames de la FFT: 1 + (muestras − fft) / salto.
        let frames = 1 + (30 * 48_000 - params.fft_size) / (params.fft_size / 2);
        let min = (max_columns / 2).min(frames);
        assert!(
            s.columns >= min && s.columns <= max_columns,
            "{max_columns}: {}",
            s.columns
        );
        assert!(s.rows <= 512);
        assert_eq!(s.db.len(), s.columns * s.rows);
        assert!((s.duration_s - 30.0).abs() < 1e-9);
        // Las columnas cubren toda la duración.
        let covered = s.seconds_per_column * s.columns as f64;
        assert!((covered - 30.0).abs() / 30.0 < 0.1, "{covered} s");
        assert!((s.nyquist_hz() - 24_000.0).abs() < 1e-9);
    }
}

#[test]
fn filas_y_hz_por_fila() {
    let dir = temp_dir();
    let p = wav(
        dir.path(),
        "r.wav",
        96_000,
        1,
        &white_noise(96_000, 1.0, 0.3, 3),
    );
    // FFT 32.768 → 16.385 bins, agrupados en ≤ 2.048 filas.
    let s = spec(
        &p,
        &SpectrogramParams {
            fft_size: 32_768,
            ..Default::default()
        },
    );
    assert!(s.rows <= 2048);
    assert!((s.hz_per_row * s.rows as f64 - 48_000.0).abs() <= s.hz_per_row * 2.0);
    // FFT 1.024 → 513 bins, sin agrupar.
    let s = spec(
        &p,
        &SpectrogramParams {
            fft_size: 1024,
            ..Default::default()
        },
    );
    assert_eq!(s.rows, 513);
    assert!((s.hz_per_row - 93.75).abs() < 1e-9);
}

#[test]
fn vistas_de_canal() {
    let dir = temp_dir();
    // L = seno de 2 kHz, R = seno de 5 kHz.
    let l = quantize(&sine(2000.0, 48_000, 2.0, 0.5), 24);
    let r = quantize(&sine(5000.0, 48_000, 2.0, 0.5), 24);
    let p = dir.path().join("lr.wav");
    write_wav_int(&p, 48_000, 24, 2, &interleave(&[l, r])).unwrap();
    let peak_hz = |view| {
        let s = spec(
            &p,
            &SpectrogramParams {
                channel_view: view,
                ..Default::default()
            },
        );
        let m = row_means_db(&s);
        let row = m
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |a, (i, &d)| if d > a.1 { (i, d) } else { a })
            .0;
        row as f64 * s.hz_per_row
    };
    assert!((peak_hz(ChannelView::Channel(0)) - 2000.0).abs() < 40.0);
    assert!((peak_hz(ChannelView::Channel(1)) - 5000.0).abs() < 40.0);

    let mono = wav(
        dir.path(),
        "mono.wav",
        48_000,
        1,
        &sine(1000.0, 48_000, 0.5, 0.5),
    );
    let side = SpectrogramParams {
        channel_view: ChannelView::Side,
        ..Default::default()
    };
    assert!(matches!(
        compute_spectrogram(&mono, &side, &CancelToken::new(), &NoProgress),
        Err(CoreError::Unsupported(_))
    ));
}

#[test]
fn audio_mas_corto_que_la_fft() {
    let dir = temp_dir();
    let p = wav(
        dir.path(),
        "corto.wav",
        44_100,
        1,
        &sine(440.0, 44_100, 0.05, 0.5),
    );
    let s = spec(&p, &SpectrogramParams::default());
    assert_eq!(s.columns, 1);
}

#[test]
fn silencio_digital() {
    let dir = temp_dir();
    let p = wav(dir.path(), "silencio.wav", 44_100, 2, &silence(44_100, 1.0));
    let s = spec(&p, &SpectrogramParams::default());
    assert!(s.db.iter().all(|&d| d == -200.0));
}

#[test]
fn cancelacion() {
    let dir = temp_dir();
    let p = wav(
        dir.path(),
        "c.wav",
        44_100,
        2,
        &white_noise(44_100, 2.0, 0.3, 5),
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    let r = compute_spectrogram(&p, &SpectrogramParams::default(), &cancel, &NoProgress);
    assert!(matches!(r, Err(CoreError::Cancelled)));
}

#[test]
fn progreso_monotono_hasta_1() {
    let dir = temp_dir();
    let p = wav(
        dir.path(),
        "p.wav",
        44_100,
        2,
        &white_noise(44_100, 5.0, 0.3, 6),
    );
    let seen = Mutex::new(Vec::new());
    let progress = |f: f32| seen.lock().unwrap().push(f);
    compute_spectrogram(
        &p,
        &SpectrogramParams::default(),
        &CancelToken::new(),
        &progress,
    )
    .unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.len() >= 3, "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*seen.last().unwrap(), 1.0);
}

#[test]
fn parametros_invalidos() {
    let dir = temp_dir();
    let p = wav(dir.path(), "x.wav", 44_100, 1, &silence(44_100, 0.1));
    let bad = SpectrogramParams {
        fft_size: 1000,
        ..Default::default()
    };
    assert!(compute_spectrogram(&p, &bad, &CancelToken::new(), &NoProgress).is_err());
}

/// Rendimiento del DSP (sin E/S): 10 minutos de estéreo a 96 kHz. Objetivo: < 10 s en release.
/// `cargo test --release -p audio-core --test spectrogram -- --ignored --nocapture`
#[test]
#[ignore]
fn rendimiento_dsp_10_minutos_96k() {
    use audio_core::{FrameAnalyzer, SpectrogramBuilder};
    use audio_core::{PcmBlock, PcmData};

    let rate = 96_000u32;
    let block_frames = 4096;
    let noise = quantize(
        &white_noise(rate, block_frames as f64 / f64::from(rate), 0.5, 1),
        24,
    );
    let block = PcmBlock {
        channels: 2,
        frames: block_frames,
        data: PcmData::Int {
            bits: 24,
            samples: interleave(&[noise.clone(), noise]),
        },
    };
    let params = SpectrogramParams::default();
    let start = std::time::Instant::now();
    let mut a = FrameAnalyzer::new(params.fft_size, params.window, params.channel_view, 2).unwrap();
    let mut b =
        SpectrogramBuilder::new(params.max_columns, params.max_rows, a.bins(), a.hop(), rate);
    let blocks = (600 * rate as usize) / block_frames;
    for _ in 0..blocks {
        a.push(&block, |p| b.push_frame(p)).unwrap();
    }
    a.finish(|p| b.push_frame(p));
    let s = b.finish(a.samples_seen());
    let elapsed = start.elapsed();
    println!(
        "10 min 96 kHz estéreo: {elapsed:?} ({} columnas × {} filas)",
        s.columns, s.rows
    );
    assert!(elapsed.as_secs_f64() < 10.0);
}

#[test]
fn render_de_un_archivo_orientado_y_con_el_corte_oscuro() {
    use audio_core::{Palette, SpectrogramPreview, render_rgba};
    let dir = temp_dir();
    let noise = lowpass_brickwall(&white_noise(44_100, 4.0, 0.5, 31), 44_100, 16_000.0);
    let p = wav(dir.path(), "r.wav", 44_100, 2, &noise);
    let s = spec(&p, &SpectrogramParams::default());
    let img = render_rgba(&s, Palette::Gray, -120.0, 0.0);
    assert_eq!((img.width, img.height), (s.columns, s.rows));
    let x = s.columns / 2;
    // Fila de imagen y = rows − 1 − fila de datos (arriba = Nyquist).
    let y_of = |hz: f64| s.rows - 1 - s.row_for_hz(hz);
    let low = img.pixel(x, y_of(8_000.0))[0];
    let high = img.pixel(x, y_of(20_000.0))[0];
    assert!(low > 100 && high < 10, "8 kHz = {low}, 20 kHz = {high}");
    // La vista previa da prácticamente la misma imagen.
    let prev = render_rgba(&SpectrogramPreview::from(&s), Palette::Gray, -120.0, 0.0);
    assert!(prev.pixel(x, y_of(8_000.0))[0].abs_diff(low) <= 2);
}
