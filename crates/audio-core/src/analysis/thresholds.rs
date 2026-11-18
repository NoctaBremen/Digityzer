//! Umbrales de los detectores. Todos los valores por defecto están en `docs/ANALYSIS.md`.
//!
//! Se guardan en TOML ([`Thresholds::to_toml`], [`Thresholds::from_toml_or_default`]) y se
//! validan con [`Thresholds::validate`].

use std::cmp::Ordering;

/// Un escalón de la tabla de bitrate estimado: un corte por debajo de `max_hz` sugiere `kbps`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BitrateStep {
    /// Frecuencia de corte máxima (exclusiva) del escalón.
    pub max_hz: f64,
    /// Bitrate aproximado del origen.
    pub kbps: u32,
}

/// Corte mínimo esperable en un archivo con pérdida de `min_kbps` o más (si el corte real es menor,
/// se transcodificó desde un bitrate inferior).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LossyExpectation {
    /// Bitrate medio a partir del que aplica.
    pub min_kbps: u32,
    /// Corte mínimo esperable.
    pub min_cutoff_hz: f64,
}

/// Umbrales de los detectores y del veredicto. Se guardan en TOML (`#[serde(default)]`: los
/// campos que falten toman el valor por defecto).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Thresholds {
    // --- Corte de frecuencia (F2-T2) ---
    /// Percentil del perfil espectral (por bin, sobre los frames activos).
    pub cutoff_percentile: f64,
    /// Anchura de la media móvil del perfil, en Hz.
    pub smoothing_hz: f64,
    /// Si la referencia (1–8 kHz) es menor, el archivo no tiene contenido útil.
    pub no_content_db: f32,
    /// Fracción superior de la banda (bajo Nyquist) que se usa para estimar el suelo.
    pub floor_band_fraction: f64,
    /// Si `referencia − suelo` es menor, hay contenido hasta arriba: banda completa.
    pub fullband_range_db: f32,
    /// El contenido está "presente" si supera el suelo en esta cantidad.
    pub content_delta_db: f32,
    /// Anchura mínima de la banda continua con contenido que termina en el corte.
    pub min_band_hz: f64,
    /// Un corte a menos de esto de Nyquist cuenta como banda completa.
    pub fullband_margin_hz: f64,
    /// Ventana de la pendiente: de `inner` a `outer` Hz a cada lado del corte.
    pub slope_inner_hz: f64,
    /// Ver `slope_inner_hz`.
    pub slope_outer_hz: f64,
    /// Pendiente mínima (dB) para considerar el corte un "acantilado" de codificador.
    pub cliff_db: f32,
    /// Tabla de bitrate estimado (orden creciente de `max_hz`).
    pub bitrate_table: Vec<BitrateStep>,

    // --- Sobremuestreo (F2-T2) ---
    /// Margen sobre el Nyquist de la frecuencia de origen candidata.
    pub upsample_margin_hz: f64,
    /// Para 88,2/96 kHz: contenido por debajo de esto y por encima del Nyquist de origen + margen
    /// → no concluyente.
    pub upsample_inconclusive_hz: f64,
    /// Para 48 kHz: un acantilado a 22.050 Hz ± esta tolerancia sugiere un origen de 44,1 kHz.
    pub upsample_48k_tolerance_hz: f64,

    // --- Profundidad de bits (F2-T3) ---
    /// Un archivo de ≥ 24 bits cuya profundidad efectiva es ≤ esto se considera relleno.
    pub padded_max_effective_bits: u32,
    /// Fracción mínima de muestras que deben concentrar ≤ 2 valores del byte bajo para hablar de
    /// relleno con patrón.
    pub pattern_fraction: f64,
    /// Activa el indicio del suelo de ruido de 16 bits.
    pub noise_floor16_enabled: bool,
    /// Rango (RMS total, dBFS) en el que un suelo plano sugiere un origen de 16 bits.
    pub noise_floor16_range_db: (f64, f64),
    /// Desviación típica máxima (dB) del suelo en la banda para considerarlo plano.
    pub noise_floor_flatness_db: f64,
    /// Banda en la que se mide el suelo de ruido (Hz).
    pub noise_floor_band_hz: (f64, f64),
    /// Percentil (por bin, frames activos) del que se estima el suelo (con corrección de sesgo).
    pub noise_floor_percentile: f64,

    // --- Clipping (F2-T3) ---
    /// Longitud mínima de una racha en el pico para contarla como recorte.
    pub clipping_min_run: u64,
    /// Recortes por minuto a partir de los que se avisa.
    pub clipping_warn_per_min: f64,
    /// Recortes por minuto a partir de los que se marca Falso (si `clipping_can_fail`).
    pub clipping_fail_per_min: f64,
    /// Si el clipping puede llevar a Falso (desactivado por defecto).
    pub clipping_can_fail: bool,

    // --- Veredicto (F2-T4) ---
    /// Sin pérdida con acantilado por debajo de esto → Falso.
    pub fake_cutoff_hz: f64,
    /// Sin pérdida con acantilado entre `fake_cutoff_hz` y esto → Sospechoso.
    pub suspicious_cutoff_hz: f64,
    /// Sin pérdida, sin acantilado y con el contenido limitado por debajo de esto → Sospechoso.
    pub bandlimited_suspicious_hz: f64,
    /// Formatos con pérdida: corte mínimo esperable según su bitrate medio (orden decreciente).
    pub lossy_expectations: Vec<LossyExpectation>,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cutoff_percentile: 90.0,
            smoothing_hz: 250.0,
            no_content_db: -90.0,
            floor_band_fraction: 0.04,
            fullband_range_db: 20.0,
            content_delta_db: 10.0,
            min_band_hz: 300.0,
            fullband_margin_hz: 500.0,
            slope_inner_hz: 200.0,
            slope_outer_hz: 1200.0,
            cliff_db: 25.0,
            // Escalones a mitad de camino entre los cortes **medidos** de LAME 3.100 (ffmpeg
            // libmp3lame): 96/112 → 15,4 kHz, 128 → 16,8, 160 → 17,5, 192 → 18,9, 224/256 → 19,6,
            // 320 → 20,25. Los codificadores antiguos (FhG) cortan 128 kbps a 16 kHz.
            bitrate_table: vec![
                BitrateStep {
                    max_hz: 11_000.0,
                    kbps: 64,
                },
                BitrateStep {
                    max_hz: 15_800.0,
                    kbps: 96,
                },
                BitrateStep {
                    max_hz: 17_150.0,
                    kbps: 128,
                },
                BitrateStep {
                    max_hz: 18_200.0,
                    kbps: 160,
                },
                BitrateStep {
                    max_hz: 19_250.0,
                    kbps: 192,
                },
                BitrateStep {
                    max_hz: 19_900.0,
                    kbps: 256,
                },
                BitrateStep {
                    max_hz: 20_800.0,
                    kbps: 320,
                },
            ],
            upsample_margin_hz: 500.0,
            upsample_inconclusive_hz: 30_000.0,
            upsample_48k_tolerance_hz: 300.0,
            padded_max_effective_bits: 16,
            pattern_fraction: 0.999,
            noise_floor16_enabled: true,
            // 16 bits sin dither: −101,1 dBFS; con dither TPDF: −96,3 dBFS.
            noise_floor16_range_db: (-103.0, -90.0),
            noise_floor_flatness_db: 3.0,
            noise_floor_band_hz: (2_000.0, 16_000.0),
            // El plan proponía el percentil 1, pero en un solo frame la potencia del ruido por bin es
            // exponencial y el p1 queda ~20 dB por debajo del nivel real (y es muy inestable). Se usa
            // el p10 con corrección del sesgo (ver `bitdepth.rs`).
            noise_floor_percentile: 10.0,
            clipping_min_run: 3,
            clipping_warn_per_min: 5.0,
            clipping_fail_per_min: 100.0,
            clipping_can_fail: false,
            fake_cutoff_hz: 19_500.0,
            suspicious_cutoff_hz: 20_500.0,
            bandlimited_suspicious_hz: 16_000.0,
            // Márgenes amplios bajo los cortes de LAME (320 → 20,25 kHz; 256 → 19,6; 192 → 18,9):
            // un "320" con corte de 16–17 kHz es un 128 transcodificado. El bitrate es la media del
            // archivo (incluye las etiquetas), de ahí los mínimos algo por debajo de los nominales.
            lossy_expectations: vec![
                LossyExpectation {
                    min_kbps: 300,
                    min_cutoff_hz: 19_500.0,
                },
                LossyExpectation {
                    min_kbps: 240,
                    min_cutoff_hz: 18_500.0,
                },
                LossyExpectation {
                    min_kbps: 180,
                    min_cutoff_hz: 17_500.0,
                },
            ],
        }
    }
}

impl Thresholds {
    /// Comprueba la coherencia de los umbrales. Devuelve los problemas, en castellano.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut e = Vec::new();
        let pct = |v: f64| (0.0..=100.0).contains(&v);
        if !pct(self.cutoff_percentile) {
            e.push("el percentil del perfil debe estar entre 0 y 100".to_string());
        }
        if !pct(self.noise_floor_percentile) || self.noise_floor_percentile == 0.0 {
            e.push("el percentil del suelo de ruido debe estar entre 0 (excluido) y 100".into());
        }
        // `partial_cmp` también rechaza NaN.
        if self.fake_cutoff_hz.partial_cmp(&self.suspicious_cutoff_hz) != Some(Ordering::Less) {
            e.push("el corte de Falso debe ser menor que el de Sospechoso".into());
        }
        if self.fake_cutoff_hz <= 0.0 || self.bandlimited_suspicious_hz <= 0.0 {
            e.push("las frecuencias de corte deben ser positivas".into());
        }
        if self.cliff_db <= 0.0 || self.content_delta_db <= 0.0 || self.fullband_range_db <= 0.0 {
            e.push("los umbrales en dB del corte deben ser positivos".into());
        }
        if self.slope_inner_hz.partial_cmp(&self.slope_outer_hz) != Some(Ordering::Less)
            || self.slope_inner_hz < 0.0
        {
            e.push("la ventana de la pendiente debe cumplir 0 ≤ interior < exterior".into());
        }
        if !(0.0..1.0).contains(&self.floor_band_fraction) || self.floor_band_fraction == 0.0 {
            e.push("la fracción de banda del suelo debe estar entre 0 y 1".into());
        }
        if self
            .bitrate_table
            .windows(2)
            .any(|w| w[0].max_hz >= w[1].max_hz)
        {
            e.push("la tabla de bitrates debe tener cortes crecientes".into());
        }
        if self.noise_floor16_range_db.0 >= self.noise_floor16_range_db.1 {
            e.push("el rango del suelo de 16 bits está invertido".into());
        }
        if self.noise_floor_band_hz.0 >= self.noise_floor_band_hz.1 {
            e.push("la banda del suelo de ruido está invertida".into());
        }
        if self.clipping_min_run < 2 {
            e.push("la racha mínima de clipping debe ser ≥ 2".into());
        }
        if self.clipping_warn_per_min < 0.0
            || self.clipping_fail_per_min < self.clipping_warn_per_min
        {
            e.push("los umbrales de clipping deben cumplir 0 ≤ aviso ≤ falso".into());
        }
        if !(1..=32).contains(&self.padded_max_effective_bits) {
            e.push("los bits efectivos de relleno deben estar entre 1 y 32".into());
        }
        if e.is_empty() { Ok(()) } else { Err(e) }
    }

    /// Lee umbrales de TOML. Si el texto no es válido o los umbrales no son coherentes, devuelve
    /// los valores por defecto y los avisos.
    pub fn from_toml_or_default(text: &str) -> (Self, Vec<String>) {
        match toml::from_str::<Self>(text) {
            Ok(t) => match t.validate() {
                Ok(()) => (t, Vec::new()),
                Err(errs) => (Self::default(), errs),
            },
            Err(e) => (Self::default(), vec![format!("umbrales ilegibles: {e}")]),
        }
    }

    /// Umbrales en TOML (para editarlos a mano).
    pub fn to_toml(&self) -> String {
        // Serializar una estructura de tipos simples no puede fallar.
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn por_defecto_validos_y_round_trip_toml() {
        let t = Thresholds::default();
        assert!(t.validate().is_ok());
        let text = t.to_toml();
        assert!(text.contains("fake_cutoff_hz"));
        let (back, warnings) = Thresholds::from_toml_or_default(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, t);
    }

    #[test]
    fn campos_que_faltan_toman_el_valor_por_defecto() {
        let (t, w) = Thresholds::from_toml_or_default("cliff_db = 30.0\n");
        assert!(w.is_empty());
        assert_eq!(t.cliff_db, 30.0);
        assert_eq!(t.fake_cutoff_hz, Thresholds::default().fake_cutoff_hz);
    }

    #[test]
    fn invalidos_dan_avisos_y_valores_por_defecto() {
        let (t, w) = Thresholds::from_toml_or_default("fake_cutoff_hz = 21000.0\n");
        assert_eq!(t, Thresholds::default());
        assert!(w[0].contains("Falso"));
        let (_, w) = Thresholds::from_toml_or_default("esto no es toml = = =");
        assert!(w[0].starts_with("umbrales ilegibles"));
        let bad = Thresholds {
            clipping_min_run: 1,
            ..Thresholds::default()
        };
        assert!(bad.validate().is_err());
    }
}
