//! Generadores de archivos corruptos y raros (F5-T1) y utilidades de los tests de robustez.
//!
//! A partir de un archivo válido producen variantes con nombre: truncados, bits volteados (semilla
//! fija), cabecera a cero, tamaños de chunk enormes, canales/frecuencia/bits imposibles, archivos
//! de 1 byte, texto con extensión de audio y archivos con basura al final.

use std::sync::mpsc;
use std::time::Duration;

use crate::signals::Rng;

/// Formato del archivo válido de partida (para las corrupciones específicas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// RIFF/WAVE con `fmt ` en el byte 12 y `data` en el 36 (lo que escribe `write_wav_int`).
    Wav,
    /// FLAC con STREAMINFO como primer bloque.
    Flac,
    /// AIFF con `COMM` en el byte 12 y `SSND` en el 38 (lo que escribe [`aiff_bytes`]).
    Aiff,
    /// Cualquier otro (solo corrupciones genéricas).
    Other,
}

/// AIFF PCM mínimo (big-endian, alineado a la izquierda), independiente de `spector-aiff`.
pub fn aiff_bytes(rate: u32, bits: u16, channels: u16, samples: &[i32]) -> Vec<u8> {
    let bps = usize::from(bits).div_ceil(8);
    let shift = 8 * bps as u32 - u32::from(bits);
    let mut data = Vec::with_capacity(samples.len() * bps);
    for &s in samples {
        let w = ((i64::from(s) << shift) as u32).to_be_bytes();
        data.extend_from_slice(&w[4 - bps..]);
    }
    let frames = (samples.len() / usize::from(channels.max(1))) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"AIFF");
    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&18u32.to_be_bytes());
    out.extend_from_slice(&channels.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&bits.to_be_bytes());
    out.extend_from_slice(&ext80(f64::from(rate)));
    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&(8 + data.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    let form = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&form.to_be_bytes());
    out
}

/// Extended de 80 bits de un entero positivo (suficiente para frecuencias).
fn ext80(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if v <= 0.0 {
        return out;
    }
    let n = v as u64;
    let lz = n.leading_zeros();
    let exp = 16_383 + 63 - lz;
    out[..2].copy_from_slice(&(exp as u16).to_be_bytes());
    out[2..].copy_from_slice(&(n << lz).to_be_bytes());
    out
}

fn put(bytes: &mut [u8], at: usize, val: &[u8]) {
    if at + val.len() <= bytes.len() {
        bytes[at..at + val.len()].copy_from_slice(val);
    }
}

/// Variantes corruptas de `valid`, con un nombre descriptivo cada una.
pub fn corruptions(kind: Kind, valid: &[u8], seed: u64) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = Vec::new();
    let len = valid.len();
    // Truncados: cabecera, mitad y último byte.
    for cut in [4, 12, 30, 44, 60, len / 2, len.saturating_sub(1)] {
        if cut < len {
            v.push((format!("truncado a {cut} bytes"), valid[..cut].to_vec()));
        }
    }
    // Bits volteados (semilla fija): en la cabecera y por todo el archivo.
    let mut rng = Rng::new(seed);
    for round in 0..12 {
        let mut b = valid.to_vec();
        let span = if round < 6 { len.min(128) } else { len };
        for _ in 0..(1 + round % 4) {
            if span > 0 {
                let i = (rng.next_u64() % span as u64) as usize;
                b[i] ^= 1 << (rng.next_u64() % 8);
            }
        }
        v.push((format!("bits volteados ({round})"), b));
    }
    let mut z = valid.to_vec();
    z[..len.min(64)].fill(0);
    v.push(("cabecera a cero".into(), z));
    v.push(("1 byte".into(), valid[..len.min(1)].to_vec()));
    v.push(("vacío".into(), Vec::new()));
    v.push((
        "texto con extensión de audio".into(),
        b"Esto no es audio, es una nota de texto.\n".repeat(20),
    ));
    let mut garbage = valid.to_vec();
    garbage.extend((0..4096).map(|i| (i * 37 % 256) as u8));
    v.push(("basura al final".into(), garbage));
    let mut dup = valid.to_vec();
    dup.extend_from_slice(valid);
    v.push(("archivo repetido dos veces".into(), dup));

    let huge = 0xFFFF_FFFFu32;
    let mut specific = |name: &str, patches: &[(usize, Vec<u8>)]| {
        let mut b = valid.to_vec();
        for (at, val) in patches {
            put(&mut b, *at, val);
        }
        v.push((name.to_string(), b));
    };
    match kind {
        Kind::Wav => {
            specific("RIFF de tamaño enorme", &[(4, huge.to_le_bytes().to_vec())]);
            specific(
                "data de tamaño enorme",
                &[(40, huge.to_le_bytes().to_vec())],
            );
            specific("fmt de tamaño enorme", &[(16, huge.to_le_bytes().to_vec())]);
            specific("0 canales", &[(22, 0u16.to_le_bytes().to_vec())]);
            specific("frecuencia 0", &[(24, 0u32.to_le_bytes().to_vec())]);
            specific("0 bits", &[(34, 0u16.to_le_bytes().to_vec())]);
            specific("64 bits", &[(34, 64u16.to_le_bytes().to_vec())]);
            specific("65535 canales", &[(22, 0xFFFFu16.to_le_bytes().to_vec())]);
            specific("block align 0", &[(32, 0u16.to_le_bytes().to_vec())]);
        }
        Kind::Aiff => {
            specific("FORM de tamaño enorme", &[(4, huge.to_be_bytes().to_vec())]);
            specific(
                "COMM de tamaño enorme",
                &[(16, huge.to_be_bytes().to_vec())],
            );
            specific(
                "SSND de tamaño enorme",
                &[(42, huge.to_be_bytes().to_vec())],
            );
            specific("frames enormes", &[(22, huge.to_be_bytes().to_vec())]);
            specific("0 canales", &[(20, 0u16.to_be_bytes().to_vec())]);
            specific("frecuencia 0", &[(28, vec![0; 10])]);
            specific(
                "frecuencia infinita",
                &[(28, vec![0x7F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0])],
            );
            specific("0 bits", &[(26, 0u16.to_be_bytes().to_vec())]);
            specific("64 bits", &[(26, 64u16.to_be_bytes().to_vec())]);
            specific(
                "offset de SSND enorme",
                &[(46, huge.to_be_bytes().to_vec())],
            );
        }
        Kind::Flac => {
            // STREAMINFO empieza en el byte 8.
            specific("bloque de metadatos enorme", &[(5, vec![0xFF, 0xFF, 0xFF])]);
            specific("tamaño de bloque 0", &[(8, vec![0, 0, 0, 0])]);
            specific(
                "frecuencia 0",
                &[
                    (18, vec![0, 0]),
                    (20, vec![0x0F & valid.get(20).copied().unwrap_or(0)]),
                ],
            );
            specific("total de muestras enorme", &[(21, vec![0xFF; 5])]);
            specific(
                "bloque final marcado al principio",
                &[(4, vec![0x80 | valid.get(4).copied().unwrap_or(0)])],
            );
        }
        Kind::Other => {}
    }
    v
}

/// Limita el espacio de direcciones del proceso de test (`RLIMIT_AS`, límite blando) a `bytes`:
/// una reserva descontrolada (p. ej. `Vec::with_capacity` con un tamaño leído de un archivo
/// corrupto) falla y aborta el test en lugar de pasar desapercibida. Solo Linux.
pub fn limit_address_space(bytes: u64) {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let current = getrlimit(Resource::As);
    let new = Rlimit {
        current: Some(current.maximum.map_or(bytes, |m| m.min(bytes))),
        maximum: current.maximum,
    };
    if let Err(e) = setrlimit(Resource::As, new) {
        eprintln!("aviso: no se pudo limitar la memoria del test: {e}");
    }
}

/// Ejecuta `f` en un hilo y espera como mucho `limit`. `Err` si tarda más (el hilo se queda
/// colgado: el test debe fallar) o si entra en pánico (con el mensaje).
pub fn run_with_timeout<T: Send + 'static>(
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("robustez".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = tx.send(r);
        })
        .map_err(|e| format!("no se pudo crear el hilo: {e}"))?;
    match rx.recv_timeout(limit) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(p)) => Err(format!(
            "PÁNICO: {}",
            p.downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default()
        )),
        Err(_) => Err(format!("TIEMPO AGOTADO (> {limit:?})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variantes_deterministas_y_con_nombre() {
        let valid = aiff_bytes(44_100, 16, 2, &[1, -1, 2, -2]);
        assert_eq!(&valid[..4], b"FORM");
        assert_eq!(valid.len(), 54 + 8);
        let a = corruptions(Kind::Aiff, &valid, 7);
        let b = corruptions(Kind::Aiff, &valid, 7);
        assert_eq!(a, b);
        assert!(a.len() > 30);
        assert!(a.iter().any(|(n, _)| n == "SSND de tamaño enorme"));
        assert_eq!(ext80(44_100.0), [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn temporizador() {
        assert_eq!(run_with_timeout(Duration::from_secs(1), || 5).unwrap(), 5);
        assert!(
            run_with_timeout(Duration::from_secs(1), || panic!("x"))
                .unwrap_err()
                .contains("PÁNICO")
        );
        assert!(
            run_with_timeout(Duration::from_millis(50), || std::thread::sleep(
                Duration::from_secs(1)
            ))
            .unwrap_err()
            .contains("TIEMPO")
        );
    }
}
