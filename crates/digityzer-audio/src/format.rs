//! Formatos de muestra que se graban **sin conversión**.

use std::fmt;

/// Formato entero little-endian que se puede escribir tal cual en un WAV.
///
/// `S24_LE` de ALSA (24 bits en la parte baja de 32) no está: un WAV exige las muestras
/// alineadas a la izquierda del contenedor, así que habría que desplazarlas y eso ya es
/// convertir. Si la tarjeta solo ofrece ese formato, se avisa en vez de grabar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// 16 bits.
    S16Le,
    /// 24 bits empaquetados en 3 bytes.
    S24_3Le,
    /// 32 bits (a menudo 24 bits útiles en la parte alta).
    S32Le,
}

impl SampleFormat {
    /// Orden de preferencia cuando la tarjeta admite varios: el de más resolución.
    pub const PREFERENCE: [SampleFormat; 3] = [
        SampleFormat::S32Le,
        SampleFormat::S24_3Le,
        SampleFormat::S16Le,
    ];

    /// Bytes por muestra.
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::S16Le => 2,
            SampleFormat::S24_3Le => 3,
            SampleFormat::S32Le => 4,
        }
    }

    /// Bits por muestra en el contenedor.
    pub fn bits(self) -> u16 {
        (self.bytes() * 8) as u16
    }

    /// Nombre ALSA.
    pub fn alsa_name(self) -> &'static str {
        match self {
            SampleFormat::S16Le => "S16_LE",
            SampleFormat::S24_3Le => "S24_3LE",
            SampleFormat::S32Le => "S32_LE",
        }
    }

    /// Formato desde su nombre ALSA (sin distinguir mayúsculas).
    pub fn from_alsa_name(s: &str) -> Option<Self> {
        Self::PREFERENCE
            .into_iter()
            .find(|f| f.alsa_name().eq_ignore_ascii_case(s))
    }

    /// Formato de la crate `alsa`.
    pub fn to_alsa(self) -> alsa::pcm::Format {
        match self {
            SampleFormat::S16Le => alsa::pcm::Format::S16LE,
            SampleFormat::S24_3Le => alsa::pcm::Format::S243LE,
            SampleFormat::S32Le => alsa::pcm::Format::S32LE,
        }
    }

    /// Valor de una muestra como entero con signo justificado a la derecha (solo para medir: el
    /// master guarda los bytes tal cual). `bytes` debe tener [`Self::bytes`] bytes.
    pub fn decode(self, b: &[u8]) -> i32 {
        match self {
            SampleFormat::S16Le => i32::from(i16::from_le_bytes([b[0], b[1]])),
            SampleFormat::S24_3Le => i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8,
            SampleFormat::S32Le => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.alsa_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_y_tamanos() {
        for f in SampleFormat::PREFERENCE {
            assert_eq!(SampleFormat::from_alsa_name(f.alsa_name()), Some(f));
            assert_eq!(usize::from(f.bits()), f.bytes() * 8);
        }
        assert_eq!(
            SampleFormat::from_alsa_name("s24_3le"),
            Some(SampleFormat::S24_3Le)
        );
        assert_eq!(SampleFormat::from_alsa_name("S24_LE"), None);
    }

    #[test]
    fn decodifica_con_signo() {
        assert_eq!(SampleFormat::S16Le.decode(&[0xFF, 0xFF]), -1);
        assert_eq!(
            SampleFormat::S24_3Le.decode(&[0x00, 0x00, 0x80]),
            -8_388_608
        );
        assert_eq!(SampleFormat::S24_3Le.decode(&[0xFF, 0xFF, 0x7F]), 8_388_607);
        assert_eq!(SampleFormat::S32Le.decode(&[0, 1, 0, 0]), 256);
    }
}
