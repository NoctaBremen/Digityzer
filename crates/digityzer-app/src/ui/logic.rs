//! Lógica de la interfaz sin GTK: nombres de archivo, escalas y textos. Se prueba sin pantalla.

use std::path::{Path, PathBuf};

/// Cara del disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Cara A.
    A,
    /// Cara B.
    B,
}

impl Side {
    /// Letra.
    pub fn letter(self) -> char {
        match self {
            Side::A => 'A',
            Side::B => 'B',
        }
    }
}

/// Nombre del disco apto para archivo: sin `/`, sin caracteres de control y sin espacios a los
/// lados. Vacío → `Disco`.
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c == '/' || c.is_control() { '-' } else { c })
        .collect();
    let s = s.trim().trim_start_matches('.').trim().to_string();
    if s.is_empty() { "Disco".into() } else { s }
}

/// Ruta del master: `<carpeta>/<disco> - Cara A.wav`, o con ` (2)`, ` (3)`… si ya existe ese
/// master o su JSON.
pub fn master_path(
    folder: &Path,
    disc: &str,
    side: Side,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let base = format!("{} - Cara {}", sanitize(disc), side.letter());
    let taken = |p: &Path| exists(p) || exists(&digityzer_project::MasterInfo::path_for(p));
    let first = folder.join(format!("{base}.wav"));
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| folder.join(format!("{base} ({n}).wav")))
        .find(|p| !taken(p))
        .unwrap_or(first)
}

/// Rango de los medidores (dBFS).
pub const METER_DB_MIN: f32 = -60.0;

/// Fracción de la altura del medidor para un nivel (0 = abajo, 1 = 0 dBFS).
pub fn meter_fraction(db: f32) -> f64 {
    f64::from(((db - METER_DB_MIN) / -METER_DB_MIN).clamp(0.0, 1.0))
}

/// Zona de color del medidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Por debajo de −18 dBFS.
    Normal,
    /// De −18 a −3 dBFS.
    High,
    /// Por encima de −3 dBFS.
    Hot,
}

/// Zona de un nivel.
pub fn zone(db: f32) -> Zone {
    if db >= -3.0 {
        Zone::Hot
    } else if db >= -18.0 {
        Zone::High
    } else {
        Zone::Normal
    }
}

/// Posición horizontal (0–1) de una frecuencia en un eje logarítmico de `f_min` a `f_max`.
pub fn log_x(hz: f64, f_min: f64, f_max: f64) -> f64 {
    ((hz.max(f_min) / f_min).ln() / (f_max / f_min).ln()).clamp(0.0, 1.0)
}

/// `HH:MM:SS`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// Espacio libre legible, con coma decimal (`12,3 GB`, `850 MB`).
pub fn bytes_human(b: u64) -> String {
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let b = b as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB).replace('.', ",")
    } else {
        format!("{:.0} MB", b / MB)
    }
}

/// Minutos de grabación que caben en `free` bytes al ritmo `bytes_per_second`.
pub fn minutes_left(free: u64, bytes_per_second: u64) -> u64 {
    free.checked_div(bytes_per_second).map_or(0, |s| s / 60)
}

/// Nivel en dB con coma decimal y un decimal (`−12,3`), o `−∞` por debajo de −120.
pub fn db_text(db: f32) -> String {
    if db <= -120.0 {
        "−∞".into()
    } else {
        format!("{db:.1}").replace('.', ",").replace('-', "−")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn nombres_de_master() {
        let dir = Path::new("/discos");
        let none = |_: &Path| false;
        assert_eq!(
            master_path(dir, "Artista / Título", Side::A, none),
            Path::new("/discos/Artista - Título - Cara A.wav")
        );
        assert_eq!(
            master_path(dir, "  ", Side::B, none),
            Path::new("/discos/Disco - Cara B.wav")
        );
        let taken: HashSet<PathBuf> = [
            PathBuf::from("/discos/LP - Cara A.wav"),
            PathBuf::from("/discos/LP - Cara A (2).wav.json"),
        ]
        .into();
        assert_eq!(
            master_path(dir, "LP", Side::A, |p| taken.contains(p)),
            Path::new("/discos/LP - Cara A (3).wav")
        );
        assert_eq!(sanitize("..oculto"), "oculto");
    }

    #[test]
    fn escalas() {
        assert_eq!(meter_fraction(0.0), 1.0);
        assert_eq!(meter_fraction(-60.0), 0.0);
        assert_eq!(meter_fraction(-200.0), 0.0);
        assert!((meter_fraction(-30.0) - 0.5).abs() < 1e-9);
        assert_eq!(zone(-2.0), Zone::Hot);
        assert_eq!(zone(-4.0), Zone::High);
        assert_eq!(zone(-12.0), Zone::High);
        assert_eq!(zone(-30.0), Zone::Normal);
        assert!((log_x(200.0, 20.0, 20_000.0) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(log_x(10.0, 20.0, 20_000.0), 0.0);
    }

    #[test]
    fn textos() {
        assert_eq!(clock(3723.4), "01:02:03");
        assert_eq!(bytes_human(12_345_000_000), "12,3 GB");
        assert_eq!(bytes_human(850_000_000), "850 MB");
        // 96 kHz · 2 canales · 4 bytes = 768.000 B/s → 1 GB ≈ 21 min.
        assert_eq!(minutes_left(1_000_000_000, 768_000), 21);
        assert_eq!(db_text(-12.34), "−12,3");
        assert_eq!(db_text(-200.0), "−∞");
    }
}
