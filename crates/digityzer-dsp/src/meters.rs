//! Medidores de nivel (T2.2): pico y RMS en dBFS, retención de pico y aviso de clipping.
//!
//! Convenciones:
//! - dBFS respecto a la escala completa del formato: un seno a escala completa marca 0 dBFS de
//!   pico y −3,01 dBFS de RMS (RMS sin el +3 dB de algunas normas).
//! - RMS en una ventana rectangular deslizante de [`MeterConfig::rms_window_s`].
//! - Clipping: [`MeterConfig::clip_run`] o más muestras **seguidas** en el código máximo o mínimo
//!   del formato. Una muestra suelta en el extremo no cuenta.

use crate::sliding::SlidingSum;

/// Suelo de los valores en dB (silencio digital).
pub const DB_FLOOR: f32 = -200.0;

/// Configuración de los medidores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterConfig {
    /// Ventana del RMS (s).
    pub rms_window_s: f64,
    /// Tiempo de retención del pico (s).
    pub peak_hold_s: f64,
    /// Muestras seguidas a fondo de escala para avisar de clipping.
    pub clip_run: usize,
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self {
            rms_window_s: 0.3,
            peak_hold_s: 2.0,
            clip_run: 3,
        }
    }
}

/// Lectura de un canal. Sin señal todavía, todo en el suelo ([`DB_FLOOR`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelLevel {
    /// Pico desde la lectura anterior (dBFS).
    pub peak_dbfs: f32,
    /// RMS de la ventana (dBFS).
    pub rms_dbfs: f32,
    /// Pico retenido (dBFS).
    pub peak_hold_dbfs: f32,
    /// Rachas de clipping desde el principio.
    pub clip_events: u64,
}

impl Default for ChannelLevel {
    fn default() -> Self {
        Self {
            peak_dbfs: DB_FLOOR,
            rms_dbfs: DB_FLOOR,
            peak_hold_dbfs: DB_FLOOR,
            clip_events: 0,
        }
    }
}

/// `20 · log10(x)` con suelo [`DB_FLOOR`].
pub fn amp_to_db(x: f64) -> f32 {
    if x <= 0.0 {
        DB_FLOOR
    } else {
        ((20.0 * x.log10()) as f32).max(DB_FLOOR)
    }
}

/// Medidor de un canal de muestras enteras justificadas a la derecha.
#[derive(Debug, Clone)]
pub struct LevelMeter {
    rate: u32,
    cfg: MeterConfig,
    scale: f64,
    max_code: i32,
    min_code: i32,
    squares: SlidingSum,
    peak: f64,
    hold: f64,
    hold_age: u64,
    run: usize,
    clip_events: u64,
}

impl LevelMeter {
    /// Medidor para muestras de `bits` bits (1–32) a `rate` Hz.
    pub fn new(rate: u32, bits: u32, cfg: MeterConfig) -> Self {
        let bits = bits.clamp(1, 32);
        let max_code = ((1i64 << (bits - 1)) - 1) as i32;
        let min_code = (-(1i64 << (bits - 1))) as i32;
        Self {
            rate,
            cfg,
            scale: (1i64 << (bits - 1)) as f64,
            max_code,
            min_code,
            squares: SlidingSum::new((cfg.rms_window_s * f64::from(rate)).round() as usize),
            peak: 0.0,
            hold: 0.0,
            hold_age: 0,
            run: 0,
            clip_events: 0,
        }
    }

    /// Procesa una muestra.
    #[inline]
    pub fn push(&mut self, s: i32) {
        let x = f64::from(s) / self.scale;
        self.squares.push(x * x);
        self.peak = self.peak.max(x.abs());
        if s == self.max_code || s == self.min_code {
            self.run += 1;
            if self.run == self.cfg.clip_run {
                self.clip_events += 1;
            }
        } else {
            self.run = 0;
        }
    }

    /// Lectura actual. Reinicia el pico del intervalo y actualiza la retención.
    pub fn take(&mut self, samples_since_last: u64) -> ChannelLevel {
        self.hold_age += samples_since_last;
        let hold_len = (self.cfg.peak_hold_s * f64::from(self.rate)) as u64;
        if self.peak >= self.hold || self.hold_age > hold_len {
            self.hold = self.peak;
            self.hold_age = 0;
        }
        let n = self.squares.filled().max(1) as f64;
        let level = ChannelLevel {
            peak_dbfs: amp_to_db(self.peak),
            rms_dbfs: amp_to_db((self.squares.sum() / n).sqrt()),
            peak_hold_dbfs: amp_to_db(self.hold),
            clip_events: self.clip_events,
        };
        self.peak = 0.0;
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_testkit::{db_to_amp, quantize, sine, white_noise};

    fn feed(m: &mut LevelMeter, s: &[i32]) -> ChannelLevel {
        for &v in s {
            m.push(v);
        }
        m.take(s.len() as u64)
    }

    #[test]
    fn seno_dentro_de_0_1_db() {
        for (bits, rate) in [(16u32, 44_100u32), (24, 96_000), (32, 96_000)] {
            for db in [0.0, -6.0, -20.0, -60.0] {
                let a = db_to_amp(db) * 0.999_999;
                let s = quantize(&sine(997.0, rate, 1.0, a), bits);
                let mut m = LevelMeter::new(rate, bits, MeterConfig::default());
                let l = feed(&mut m, &s);
                assert!(
                    (f64::from(l.peak_dbfs) - db).abs() < 0.1,
                    "{bits} {db}: {l:?}"
                );
                let rms = db - 3.0103;
                assert!(
                    (f64::from(l.rms_dbfs) - rms).abs() < 0.1,
                    "{bits} {db}: {l:?}"
                );
            }
        }
    }

    #[test]
    fn ruido_blanco_rms() {
        // Uniforme en [−a, a]: RMS = a/√3.
        let s = quantize(&white_noise(48_000, 1.0, 0.5, 9), 24);
        let mut m = LevelMeter::new(48_000, 24, MeterConfig::default());
        let l = feed(&mut m, &s);
        let expected = 20.0 * (0.5f64 / 3f64.sqrt()).log10();
        assert!((f64::from(l.rms_dbfs) - expected).abs() < 0.1, "{l:?}");
    }

    #[test]
    fn retencion_de_pico() {
        let mut m = LevelMeter::new(1000, 16, MeterConfig::default());
        let loud = feed(&mut m, &[16_000; 10]);
        let quiet = feed(&mut m, &[100; 500]);
        assert!((quiet.peak_hold_dbfs - loud.peak_dbfs).abs() < 1e-6);
        assert!(quiet.peak_dbfs < -40.0);
        // Pasados 2 s, la retención baja al pico actual.
        let later = feed(&mut m, &[100; 2_000]);
        assert!((later.peak_hold_dbfs - later.peak_dbfs).abs() < 1e-6);
    }

    #[test]
    fn clipping_solo_con_rachas() {
        let mut m = LevelMeter::new(48_000, 16, MeterConfig::default());
        let l = feed(&mut m, &[0, 32_767, 0, -32_768, -32_768, 0]);
        assert_eq!(l.clip_events, 0);
        let l = feed(
            &mut m,
            &[32_767, 32_767, 32_767, 32_767, 0, -32_768, -32_768, -32_768],
        );
        assert_eq!(l.clip_events, 2);
    }

    #[test]
    fn sin_senal_en_el_suelo() {
        let l = ChannelLevel::default();
        assert_eq!(
            (l.peak_dbfs, l.rms_dbfs, l.peak_hold_dbfs),
            (DB_FLOOR, DB_FLOOR, DB_FLOOR)
        );
    }

    #[test]
    fn silencio() {
        let mut m = LevelMeter::new(48_000, 24, MeterConfig::default());
        let l = feed(&mut m, &[0; 4800]);
        assert_eq!((l.peak_dbfs, l.rms_dbfs), (DB_FLOOR, DB_FLOOR));
    }
}
