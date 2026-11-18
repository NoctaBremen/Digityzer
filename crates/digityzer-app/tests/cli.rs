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
