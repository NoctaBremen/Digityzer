//! `digityzer-cli probe` (T1.1): tarjetas ALSA y lo que admite cada `hw:` en captura.

use std::fmt::Write as _;

use alsa::pcm::{Format, HwParams, PCM};
use alsa::{Card, Ctl, Direction};

use crate::error::{EBUSY, errno_of};

/// Frecuencias que se comprueban.
pub const PROBE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// Formatos que se comprueban (los grabables y, a título informativo, S24_LE y float).
pub const PROBE_FORMATS: [(Format, &str); 5] = [
    (Format::S16LE, "S16_LE"),
    (Format::S243LE, "S24_3LE"),
    (Format::S24LE, "S24_LE"),
    (Format::S32LE, "S32_LE"),
    (Format::FloatLE, "FLOAT_LE"),
];

/// Capacidades de captura de un dispositivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureCaps {
    /// Admite esto.
    Available {
        /// Formatos (nombres ALSA).
        formats: Vec<String>,
        /// Frecuencias de [`PROBE_RATES`] que admite.
        rates: Vec<u32>,
        /// Canales mínimos.
        channels_min: u32,
        /// Canales máximos.
        channels_max: u32,
    },
    /// Lo tiene abierto otro programa.
    Busy,
    /// No se pudo consultar.
    Error(String),
}

/// Un dispositivo PCM de captura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmDevice {
    /// Número de dispositivo.
    pub device: u32,
    /// Nombre del dispositivo.
    pub name: String,
    /// Nombre `hw:` estable (por identificador de tarjeta, no por índice).
    pub hw: String,
    /// Capacidades.
    pub caps: CaptureCaps,
}

/// Una tarjeta con sus dispositivos de captura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardInfo {
    /// Índice de la tarjeta.
    pub index: i32,
    /// Identificador (`USB1`…).
    pub id: String,
    /// Nombre corto.
    pub name: String,
    /// Nombre largo.
    pub longname: String,
    /// Dispositivos con captura.
    pub devices: Vec<PcmDevice>,
}

fn query(hw: &str) -> CaptureCaps {
    let pcm = match PCM::new(hw, Direction::Capture, true) {
        Ok(p) => p,
        Err(e) if errno_of(&e) == EBUSY => return CaptureCaps::Busy,
        Err(e) => return CaptureCaps::Error(e.to_string()),
    };
    let hwp = match HwParams::any(&pcm) {
        Ok(h) => h,
        Err(e) => return CaptureCaps::Error(e.to_string()),
    };
    CaptureCaps::Available {
        formats: PROBE_FORMATS
            .iter()
            .filter(|(f, _)| hwp.test_format(*f).is_ok())
            .map(|(_, n)| (*n).to_string())
            .collect(),
        rates: PROBE_RATES
            .into_iter()
            .filter(|&r| hwp.test_rate(r).is_ok())
            .collect(),
        channels_min: hwp.get_channels_min().unwrap_or(0),
        channels_max: hwp.get_channels_max().unwrap_or(0),
    }
}

/// Recorre las tarjetas ALSA. Sin tarjetas, devuelve una lista vacía.
pub fn probe() -> Vec<CardInfo> {
    let mut out = Vec::new();
    for card in alsa::card::Iter::new().filter_map(Result::ok) {
        out.push(probe_card(&card));
    }
    out
}

fn probe_card(card: &Card) -> CardInfo {
    let index = card.get_index();
    let mut info = CardInfo {
        index,
        id: index.to_string(),
        name: card.get_name().unwrap_or_default(),
        longname: card.get_longname().unwrap_or_default(),
        devices: Vec::new(),
    };
    let Ok(ctl) = Ctl::from_card(card, false) else {
        return info;
    };
    if let Some(id) = ctl
        .card_info()
        .ok()
        .and_then(|ci| ci.get_id().ok().map(str::to_string))
    {
        info.id = id;
    }
    for dev in alsa::ctl::DeviceIter::new(&ctl) {
        let Ok(device) = u32::try_from(dev) else {
            continue;
        };
        let Ok(pi) = ctl.pcm_info(device, 0, Direction::Capture) else {
            continue; // Sin captura.
        };
        let hw = format!("hw:CARD={},DEV={device}", info.id);
        info.devices.push(PcmDevice {
            device,
            name: pi.get_name().unwrap_or_default().to_string(),
            caps: query(&hw),
            hw,
        });
    }
    info
}

/// Frecuencia en kHz con coma decimal (`44,1 kHz`, `96 kHz`).
pub fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", f64::from(rate) / 1000.0).replace('.', ",")
    }
}

/// Texto de `probe` para la terminal.
pub fn format_report(cards: &[CardInfo]) -> String {
    let mut s = String::new();
    if cards.is_empty() {
        s.push_str("No hay tarjetas ALSA.\n");
        return s;
    }
    for c in cards {
        let _ = writeln!(s, "Tarjeta {} «{}» — {}", c.index, c.id, c.name);
        if !c.longname.is_empty() && c.longname != c.name {
            let _ = writeln!(s, "  {}", c.longname);
        }
        if c.devices.is_empty() {
            s.push_str("  (sin dispositivos de captura)\n");
        }
        for d in &c.devices {
            let _ = writeln!(s, "  {}  {}", d.hw, d.name);
            match &d.caps {
                CaptureCaps::Available {
                    formats,
                    rates,
                    channels_min,
                    channels_max,
                } => {
                    let fmts = if formats.is_empty() {
                        "ninguno".to_string()
                    } else {
                        formats.join(", ")
                    };
                    let rates: Vec<String> = rates.iter().map(|&r| khz(r)).collect();
                    let ch = if channels_min == channels_max {
                        channels_max.to_string()
                    } else {
                        format!("{channels_min}–{channels_max}")
                    };
                    let _ = writeln!(s, "    formatos:    {fmts}");
                    let _ = writeln!(s, "    frecuencias: {}", rates.join(", "));
                    let _ = writeln!(s, "    canales:     {ch}");
                }
                CaptureCaps::Busy => {
                    s.push_str("    en uso por otro programa (¿xwax abierto?)\n");
                }
                CaptureCaps::Error(e) => {
                    let _ = writeln!(s, "    no se pudo consultar: {e}");
                }
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn informe_legible() {
        let cards = vec![CardInfo {
            index: 1,
            id: "USB1".into(),
            name: "XONE 96 USB 1".into(),
            longname: "Allen & Heath XONE 96 USB 1 at usb-0000:00:14.0-2".into(),
            devices: vec![
                PcmDevice {
                    device: 0,
                    name: "USB Audio".into(),
                    hw: "hw:CARD=USB1,DEV=0".into(),
                    caps: CaptureCaps::Available {
                        formats: vec!["S32_LE".into()],
                        rates: vec![44_100, 48_000, 96_000],
                        channels_min: 12,
                        channels_max: 12,
                    },
                },
                PcmDevice {
                    device: 1,
                    name: "USB Audio #1".into(),
                    hw: "hw:CARD=USB1,DEV=1".into(),
                    caps: CaptureCaps::Busy,
                },
            ],
        }];
        let r = format_report(&cards);
        assert!(r.contains("Tarjeta 1 «USB1» — XONE 96 USB 1"), "{r}");
        assert!(r.contains("hw:CARD=USB1,DEV=0"));
        assert!(r.contains("formatos:    S32_LE"));
        assert!(r.contains("frecuencias: 44,1 kHz, 48 kHz, 96 kHz"));
        assert!(r.contains("canales:     12\n"));
        assert!(r.contains("en uso por otro programa"));
        assert_eq!(format_report(&[]), "No hay tarjetas ALSA.\n");
    }

    #[test]
    fn sin_tarjetas_no_falla() {
        // En el CI no hay tarjetas: la lista sale vacía, sin pánicos.
        let _ = probe();
    }
}
