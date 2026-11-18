//! Grabación completa con el origen sintético: master, JSON, xruns, huella y parada.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;

use digityzer_audio::wav::HEADER_LEN;
use digityzer_audio::{
    CaptureSource, EngineConfig, ReadOutcome, RecordRequest, Recorder, SampleFormat, StreamConfig,
    SyntheticSource, verify_master,
};
use digityzer_project::MasterInfo;

fn config(format: SampleFormat) -> StreamConfig {
    StreamConfig {
        rate: 96_000,
        format,
        channels: 12,
        period_frames: 1024,
    }
}

fn all_bytes(mut s: SyntheticSource) -> Vec<u8> {
    let fb = s.config().frame_bytes();
    let mut buf = vec![0u8; fb * 1024];
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

fn pair(all: &[u8], format: SampleFormat, p: [usize; 2]) -> Vec<u8> {
    let sb = format.bytes();
    all.chunks_exact(sb * 12)
        .flat_map(|f| p.iter().flat_map(move |&c| f[c * sb..c * sb + sb].to_vec()))
        .collect()
}

fn request(path: std::path::PathBuf) -> RecordRequest {
    RecordRequest {
        path,
        pair: [3, 4],
        notes: "Sello Records CAT-001, cara A".into(),
        engine: EngineConfig {
            realtime_priority: None,
            ..EngineConfig::default()
        },
    }
}

#[test]
fn master_json_xrun_y_huella() {
    let dir = digityzer_testkit::temp_dir();
    let path = dir.path().join("cara-a.wav");
    // 3 s a 96 kHz, 12 canales en S32_LE, con un xrun provocado.
    let src = SyntheticSource::new(config(SampleFormat::S32Le))
        .with_total_frames(288_000)
        .with_xruns_at(vec![100_000]);
    let expected = pair(&all_bytes(src.clone()), SampleFormat::S32Le, [2, 3]);
    let rec = Recorder::start(Box::new(src), request(path.clone())).unwrap();
    while !rec.capture_finished() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let r = rec.stop().unwrap();
    assert!(r.capture_error.is_none());

    // El par 3–4 byte a byte.
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[HEADER_LEN as usize..], &expected[..]);

    // JSON.
    let info = MasterInfo::read(&r.json).unwrap();
    assert_eq!(info, r.info);
    assert_eq!(info.sample_rate, 96_000);
    assert_eq!(info.sample_format, "S32_LE");
    assert_eq!(info.container_bits, 32);
    assert_eq!(info.lsb_zero_bits, 8);
    assert_eq!(info.channels_opened, 12);
    assert_eq!(info.channel_pair, [3, 4]);
    assert_eq!(info.container, "WAV");
    assert_eq!(info.frames, 288_000);
    assert!((info.duration_s - 3.0).abs() < 1e-9);
    assert_eq!(info.xruns.len(), 1);
    assert_eq!(info.xruns[0].frame, 100_000);
    assert!((info.xruns[0].seconds - 100_000.0 / 96_000.0).abs() < 1e-9);
    assert_eq!(info.frames_lost_ring, 0);
    assert_eq!(info.notes, "Sello Records CAT-001, cara A");
    assert_eq!(info.hash.blocks.len(), expected.len().div_ceil(1 << 20));
    assert!(info.started_at.ends_with('Z'));

    // T1.3: la huella por bloques de lo leído coincide con la del archivo.
    let v = verify_master(&path).unwrap();
    assert!(v.ok(), "{v:?}");

    // Un solo bit cambiado en el bloque 2 se detecta en ese bloque.
    let mut f = OpenOptions::new().write(true).open(&path).unwrap();
    let at = HEADER_LEN + 2 * (1 << 20) + 17;
    f.seek(SeekFrom::Start(at)).unwrap();
    f.write_all(&[bytes[at as usize] ^ 0x01]).unwrap();
    drop(f);
    let v = verify_master(&path).unwrap();
    assert_eq!(v.mismatched, vec![2]);
    assert!(!v.total_ok && !v.ok());
}

#[test]
fn parar_a_mitad_no_pierde_lo_capturado() {
    let dir = digityzer_testkit::temp_dir();
    let path = dir.path().join("en-vivo.wav");
    let src = SyntheticSource::new(config(SampleFormat::S24_3Le)).realtime();
    let rec = Recorder::start(Box::new(src.clone()), request(path.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let st = rec.status();
    assert!(st.frames_written > 0, "{st:?}");
    let r = rec.stop().unwrap();
    // Todo lo capturado está en el master: los primeros N frames del origen, sin huecos.
    let n = r.info.frames;
    assert!(n >= st.frames_written);
    let expected = pair(
        &all_bytes(src.with_total_frames(n)),
        SampleFormat::S24_3Le,
        [2, 3],
    );
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[HEADER_LEN as usize..], &expected[..]);
    assert!(verify_master(&path).unwrap().ok());
}

#[test]
fn no_sobrescribe_ni_acepta_un_par_imposible() {
    let dir = digityzer_testkit::temp_dir();
    let path = dir.path().join("x.wav");
    std::fs::write(MasterInfo::path_for(&path), b"{}").unwrap();
    let src = SyntheticSource::new(config(SampleFormat::S16Le)).with_total_frames(10);
    assert!(Recorder::start(Box::new(src.clone()), request(path.clone())).is_err());
    assert!(!path.exists());

    let mut req = request(dir.path().join("y.wav"));
    req.pair = [13, 14];
    let e = Recorder::start(Box::new(src), req).unwrap_err().to_string();
    assert!(e.contains("13–14") && e.contains("12 canales"), "{e}");
}

#[test]
fn sesion_armada_grabar_parar_y_volver_a_grabar() {
    use digityzer_audio::Session;
    let dir = digityzer_testkit::temp_dir();
    let src = SyntheticSource::new(config(SampleFormat::S32Le)).realtime();
    // La señal esperada se calcula antes y sin el ritmo de tiempo real: 20 s de sobra.
    let all =
        all_bytes(SyntheticSource::new(config(SampleFormat::S32Le)).with_total_frames(96_000 * 20));
    let expected = pair(&all, SampleFormat::S32Le, [2, 3]);
    let mut s = Session::open(
        Box::new(src),
        [3, 4],
        EngineConfig {
            realtime_priority: None,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    // Armada: medidores en marcha, nada grabado.
    std::thread::sleep(Duration::from_millis(300));
    let snap = s.snapshot().clone();
    assert!(snap.frames > 0);
    // Origen sintético: canal 3 = 1.320 Hz y canal 4 = 1.760 Hz a −12 dBFS de pico.
    assert!(
        (snap.levels[0].peak_dbfs + 12.04).abs() < 0.2,
        "{:?}",
        snap.levels
    );
    assert!(!s.status().recording);

    for name in ["cara-a.wav", "cara-b.wav"] {
        let path = dir.path().join(name);
        s.start_recording(&path, name).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(s.status().recording && s.status().frames_written > 0);
        let r = s.stop_recording().unwrap();
        assert!(verify_master(&path).unwrap().ok());
        assert_eq!(r.info.notes, name);
        // El master es un tramo contiguo de la señal, empezando en un frame completo.
        let bytes = std::fs::read(&path).unwrap();
        let data = &bytes[HEADER_LEN as usize..];
        assert!(!data.is_empty() && data.len().is_multiple_of(8));
        let at = expected
            .windows(64)
            .position(|w| w == &data[..64])
            .expect("el inicio del master está en la señal");
        assert!(at.is_multiple_of(8));
        assert_eq!(&expected[at..at + data.len()], data);
    }
    assert!(s.close().unwrap().is_none());
}
