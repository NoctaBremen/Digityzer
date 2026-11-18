//! `digityzer-cli` de punta a punta con el origen sintético (sin tarjeta).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_digityzer-cli");

#[test]
fn probe_sin_tarjetas_no_falla() {
    let out = Command::new(BIN).arg("probe").output().unwrap();
    assert!(out.status.success());
}

#[test]
fn record_con_duracion_y_verify() {
    let dir = tempfile::tempdir().unwrap();
    let master = dir.path().join("cara-a.wav");
    let out = Command::new(BIN)
        .args(["record", "--simulate", "--pair", "3-4", "--duration", "1"])
        .args(["--notes", "Prueba"])
        .arg(&master)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("0 xruns"), "{stdout}");
    let json = dir.path().join("cara-a.wav.json");
    let info: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(info["channel_pair"], serde_json::json!([3, 4]));
    assert_eq!(info["notes"], "Prueba");
    assert!(info["frames"].as_u64().unwrap() >= 90_000);

    let v = Command::new(BIN)
        .arg("verify")
        .arg(&master)
        .output()
        .unwrap();
    assert!(v.status.success(), "{}", String::from_utf8_lossy(&v.stdout));

    // Segunda vez al mismo archivo: error, sin tocar nada.
    let again = Command::new(BIN)
        .args(["record", "--simulate", "--pair", "3-4", "--duration", "1"])
        .arg(&master)
        .output()
        .unwrap();
    assert!(!again.status.success());
}

#[test]
fn ctrl_c_cierra_el_master_sin_perder_datos() {
    let dir = tempfile::tempdir().unwrap();
    let master = dir.path().join("ctrl-c.wav");
    let child = Command::new(BIN)
        .args(["record", "--simulate", "--pair", "5-6"])
        .arg(&master)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let kill = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = Command::new(BIN)
        .arg("verify")
        .arg(&master)
        .output()
        .unwrap();
    assert!(v.status.success(), "{}", String::from_utf8_lossy(&v.stdout));
    let json = dir.path().join("ctrl-c.wav.json");
    let info: serde_json::Value = serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
    assert!(info["frames"].as_u64().unwrap() > 96_000);
}

/// T1.6: matar la grabación con `kill -9` pierde como mucho 2 s, y lo recuperado es idéntico bit a
/// bit a lo capturado hasta ese punto.
#[test]
fn kill_9_y_recuperacion_bit_a_bit() {
    use digityzer_audio::{
        CaptureSource, ReadOutcome, SampleFormat, StreamConfig, SyntheticSource,
    };
    let dir = tempfile::tempdir().unwrap();
    let master = dir.path().join("corte.wav");
    let started = std::time::Instant::now();
    let mut child = Command::new(BIN)
        .args(["record", "--simulate", "--pair", "3-4"])
        .arg(&master)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(5500));
    child.kill().unwrap(); // SIGKILL
    let elapsed = started.elapsed().as_secs_f64();
    child.wait().unwrap();
    let partial = dir.path().join("corte.wav.partial");
    assert!(partial.exists() && !master.exists());

    let out = Command::new(BIN)
        .arg("recover")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(master.exists() && !partial.exists());
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("corte.wav.json")).unwrap()).unwrap();
    assert_eq!(info["recovered"], true);
    assert_eq!(info["interrupted"], true);
    let frames = info["frames"].as_u64().unwrap();
    let secs = frames as f64 / 96_000.0;
    assert!(secs >= elapsed - 2.5, "recuperados {secs} s de {elapsed} s");
    let lost = info["seconds_lost"].as_f64().unwrap();
    assert!(lost <= 2.5, "{lost}");
    // La estimación de lo perdido cuadra con lo que faltó (± medio segundo de arranque).
    assert!(
        (secs + lost - elapsed).abs() < 0.6,
        "{secs} + {lost} frente a {elapsed}"
    );
    let v = Command::new(BIN)
        .arg("verify")
        .arg(&master)
        .output()
        .unwrap();
    assert!(v.status.success());

    // Bit a bit: el origen sintético es determinista; el par 3–4 de sus primeros `frames`.
    let cfg = StreamConfig {
        rate: 96_000,
        format: SampleFormat::S32Le,
        channels: 12,
        period_frames: 1024,
    };
    let mut src = SyntheticSource::new(cfg).with_total_frames(frames);
    let mut buf = vec![0u8; 48 * 1024];
    let mut expected = Vec::new();
    src.start().unwrap();
    while let ReadOutcome::Frames(n) = src.read(&mut buf).unwrap() {
        for f in buf[..n * 48].chunks_exact(48) {
            expected.extend_from_slice(&f[8..16]);
        }
    }
    let bytes = std::fs::read(&master).unwrap();
    assert_eq!(bytes.len() - 104, expected.len());
    assert!(
        bytes[104..] == expected[..],
        "lo recuperado no coincide con lo capturado"
    );
}

#[test]
fn sin_dispositivo_ni_simulacion_es_un_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(["record", "--pair", "3-4"])
        .arg(dir.path().join("x.wav"))
        .output()
        .unwrap();
    assert!(!out.status.success());
}
