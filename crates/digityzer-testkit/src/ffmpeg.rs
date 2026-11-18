//! Helpers opcionales de `ffmpeg` para generar fixtures que no se pueden crear en Rust puro
//! (MP3, AAC, ALAC…).
//!
//! FFmpeg **solo** se usa en los tests y nunca es obligatorio: si no está en el PATH, los tests
//! que lo necesitan se saltan con [`require_ffmpeg!`](crate::require_ffmpeg).

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// `true` si `ffmpeg -version` se ejecuta correctamente (se comprueba una sola vez).
pub fn ffmpeg_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Ejecuta `ffmpeg -hide_banner -loglevel error -y -i <src> <args…> <dst>`.
///
/// Ejemplos de `args`: `["-c:a", "libmp3lame", "-b:a", "128k"]`, `["-c:a", "alac"]`.
pub fn ffmpeg_encode(src: &Path, dst: &Path, args: &[&str]) -> io::Result<()> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(src)
        .args(args)
        .arg(dst)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ffmpeg falló ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
