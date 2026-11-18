//! Detector de sobremuestreo falso (F2-T2): un archivo de 96 kHz sin contenido por encima de
//! ~22 kHz viene de 44,1/48 kHz.
//!
//! - Frecuencias ≥ 88,2 kHz: se buscan frecuencias de origen candidatas (44,1 y 48 kHz; para
//!   176,4/192 kHz también 88,2 y 96 kHz). Si el contenido no pasa del Nyquist de alguna + margen,
//!   el archivo está sobremuestreado, y se elige la **más baja compatible** (la más ajustada al
//!   contenido): un 44,1 kHz llevado a 96 kHz tiene contenido hasta 22,05 kHz y se atribuye a 44,1,
//!   no a 48 (el plan proponía preferir la familia del mismo múltiplo, que acertaba menos). Si el
//!   contenido es mucho más bajo (p. ej. un MP3 a 16 kHz), cualquier origen es posible y se informa
//!   el más bajo.
//! - 48 kHz: un acantilado a 22.050 Hz ± tolerancia sugiere un origen de 44,1 kHz.

use super::cutoff::{CutoffKind, CutoffResult};
use super::thresholds::Thresholds;

/// Resultado del detector de sobremuestreo.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum UpsamplingResult {
    /// No aplica (frecuencia de muestreo baja o sin contenido).
    NotApplicable,
    /// Hay contenido por encima de cualquier origen candidato: frecuencia genuina.
    Genuine,
    /// El contenido no pasa del Nyquist de `from_rate_guess`.
    Upsampled {
        /// Frecuencia de origen probable.
        from_rate_guess: u32,
        /// Frecuencia máxima con contenido.
        content_max_hz: f64,
    },
    /// Contenido algo por encima del Nyquist de origen, pero no lo bastante para descartarlo.
    Inconclusive {
        /// Frecuencia máxima con contenido.
        content_max_hz: f64,
    },
}

/// Detecta el sobremuestreo a partir del corte y de la frecuencia de muestreo.
pub fn detect_upsampling(
    cutoff: &CutoffResult,
    sample_rate: u32,
    t: &Thresholds,
) -> UpsamplingResult {
    if cutoff.kind == CutoffKind::NoContent {
        return UpsamplingResult::NotApplicable;
    }
    let content = cutoff.cutoff_hz;
    if sample_rate == 48_000 {
        let near_44k1 = (content - 22_050.0).abs() <= t.upsample_48k_tolerance_hz;
        return if cutoff.kind == CutoffKind::Cutoff && cutoff.cliff && near_44k1 {
            UpsamplingResult::Upsampled {
                from_rate_guess: 44_100,
                content_max_hz: content,
            }
        } else {
            UpsamplingResult::NotApplicable
        };
    }
    if sample_rate < 88_200 {
        return UpsamplingResult::NotApplicable;
    }
    let fits = |src: u32| content <= f64::from(src) / 2.0 + t.upsample_margin_hz;
    if let Some(src) = candidate_sources(sample_rate)
        .into_iter()
        .find(|&s| fits(s))
    {
        return UpsamplingResult::Upsampled {
            from_rate_guess: src,
            content_max_hz: content,
        };
    }
    // Para 88,2/96 kHz, la zona entre el Nyquist de 44,1/48 + margen y ~30 kHz es dudosa.
    if sample_rate < 176_400 && content < t.upsample_inconclusive_hz {
        return UpsamplingResult::Inconclusive {
            content_max_hz: content,
        };
    }
    UpsamplingResult::Genuine
}

/// Frecuencias de origen candidatas, de menor a mayor.
pub fn candidate_sources(sample_rate: u32) -> Vec<u32> {
    if sample_rate >= 176_400 {
        vec![44_100, 48_000, 88_200, 96_000]
    } else {
        vec![44_100, 48_000]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::cutoff::SourceEstimate;

    fn cut(kind: CutoffKind, hz: f64, cliff: bool) -> CutoffResult {
        CutoffResult {
            kind,
            cutoff_hz: hz,
            cliff,
            slope_db: if cliff { 60.0 } else { 5.0 },
            reference_db: -50.0,
            floor_db: -140.0,
            estimated_source: SourceEstimate::LosslessOrHigh,
        }
    }

    fn from(r: UpsamplingResult) -> Option<u32> {
        match r {
            UpsamplingResult::Upsampled {
                from_rate_guess, ..
            } => Some(from_rate_guess),
            _ => None,
        }
    }

    #[test]
    fn origen_mas_ajustado() {
        let t = Thresholds::default();
        let up = |hz, rate| {
            from(detect_upsampling(
                &cut(CutoffKind::Cutoff, hz, true),
                rate,
                &t,
            ))
        };
        // 96 kHz con un MP3 a 20,5 kHz o un 44,1 a 22,05 kHz: la más baja compatible, 44,1.
        assert_eq!(up(20_500.0, 96_000), Some(44_100));
        assert_eq!(up(22_050.0, 96_000), Some(44_100));
        assert_eq!(up(22_000.0, 88_200), Some(44_100));
        // Contenido hasta 24 kHz: solo cabe en 48.
        assert_eq!(up(24_000.0, 88_200), Some(48_000));
        assert_eq!(up(24_000.0, 96_000), Some(48_000));
        // 192 kHz con contenido hasta 40 kHz: viene de 88,2 o 96 (se informa la más baja).
        assert_eq!(up(40_000.0, 192_000), Some(88_200));
        let r = detect_upsampling(&cut(CutoffKind::Cutoff, 20_500.0, true), 96_000, &t);
        assert_eq!(
            r,
            UpsamplingResult::Upsampled {
                from_rate_guess: 44_100,
                content_max_hz: 20_500.0
            }
        );
    }

    #[test]
    fn genuino_y_dudoso() {
        let t = Thresholds::default();
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::FullBand, 48_000.0, false), 96_000, &t),
            UpsamplingResult::Genuine
        );
        assert!(matches!(
            detect_upsampling(&cut(CutoffKind::Cutoff, 27_000.0, false), 96_000, &t),
            UpsamplingResult::Inconclusive { .. }
        ));
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::Cutoff, 35_000.0, false), 96_000, &t),
            UpsamplingResult::Genuine
        );
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::Cutoff, 70_000.0, false), 192_000, &t),
            UpsamplingResult::Genuine
        );
    }

    #[test]
    fn casos_de_48_y_bajas_frecuencias() {
        let t = Thresholds::default();
        assert_eq!(
            from(detect_upsampling(
                &cut(CutoffKind::Cutoff, 22_100.0, true),
                48_000,
                &t
            )),
            Some(44_100)
        );
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::Cutoff, 22_100.0, false), 48_000, &t),
            UpsamplingResult::NotApplicable,
            "sin acantilado no se afirma nada"
        );
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::Cutoff, 16_000.0, true), 44_100, &t),
            UpsamplingResult::NotApplicable
        );
        assert_eq!(
            detect_upsampling(&cut(CutoffKind::NoContent, 0.0, false), 96_000, &t),
            UpsamplingResult::NotApplicable
        );
    }

    #[test]
    fn candidatas() {
        assert_eq!(candidate_sources(88_200), vec![44_100, 48_000]);
        assert_eq!(candidate_sources(96_000), vec![44_100, 48_000]);
        assert_eq!(
            candidate_sources(192_000),
            vec![44_100, 48_000, 88_200, 96_000]
        );
    }
}
