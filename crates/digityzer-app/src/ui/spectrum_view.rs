//! Estado de la vista del espectro (T2.6–T2.8): vista elegida, suavizado, balística de las tres
//! capas, cursor e historial de la cascada. Sin GTK: se prueba y se usa también para las
//! capturas de referencia (T2.9f).

use digityzer_dsp::LiveSnapshot;
use digityzer_dsp::live::SPECTRUM_F_MIN;
use digityzer_dsp::spectrum::{
    Ballistics, DISPLAY_TAU_S, interp_power, log_bands, log_grid, smooth_curve,
};

/// Vista del espectro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectrumMode {
    /// Curva suave con tres capas (por defecto).
    Curve,
    /// Barras logarítmicas.
    Bars,
    /// Espectrograma en cascada en vivo.
    Waterfall,
}

impl SpectrumMode {
    /// Todas, en el orden del selector.
    pub const ALL: [SpectrumMode; 3] = [
        SpectrumMode::Curve,
        SpectrumMode::Bars,
        SpectrumMode::Waterfall,
    ];

    /// Nombre visible.
    pub fn label(self) -> &'static str {
        match self {
            SpectrumMode::Curve => "Curva suave",
            SpectrumMode::Bars => "Barras",
            SpectrumMode::Waterfall => "Cascada",
        }
    }

    /// Identificador para la configuración.
    pub fn id(self) -> &'static str {
        match self {
            SpectrumMode::Curve => "curva",
            SpectrumMode::Bars => "barras",
            SpectrumMode::Waterfall => "cascada",
        }
    }

    /// Desde el identificador; si no se conoce, la curva.
    pub fn from_id(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|m| m.id() == s)
            .unwrap_or(SpectrumMode::Curve)
    }
}

/// Fracciones de octava del suavizado.
pub const SMOOTHING: [u32; 3] = [3, 6, 12];
/// Puntos de la curva.
pub const CURVE_POINTS: usize = 320;
/// Número de barras.
pub const BARS: usize = 160;
/// Columnas del historial de la cascada.
pub const WATERFALL_COLUMNS: usize = 600;
/// Filas de la cascada.
pub const WATERFALL_ROWS: usize = 256;
/// Rango de colores de la cascada (dBFS): por debajo, color de `gráfica`; por encima, el más
/// claro del degradado.
pub const WATERFALL_DB_MIN: f32 = -90.0;
/// Tope del rango de colores de la cascada (dBFS).
pub const WATERFALL_DB_MAX: f32 = -10.0;
/// Escala de dB del espectro.
pub const DB_MIN: f32 = -130.0;
/// Tope de la escala de dB del espectro.
pub const DB_MAX: f32 = 0.0;

/// Estado de la vista.
#[derive(Debug, Clone)]
pub struct SpectrumView {
    /// Vista elegida.
    pub mode: SpectrumMode,
    /// Suavizado en fracciones de octava (3, 6 o 12).
    pub fraction: u32,
    /// Posición del cursor en píxeles del área, si está encima.
    pub cursor: Option<(f64, f64)>,
    rate: u32,
    grid: Vec<f64>,
    target: Vec<f32>,
    instant: Ballistics,
    average: Ballistics,
    peak: Ballistics,
    bars: Ballistics,
    /// Curvas mostradas (dBFS sobre [`Self::grid`]).
    pub curves: [Vec<f32>; 3],
    /// Barras mostradas (dBFS).
    pub bar_values: Vec<f32>,
    /// Historial de la cascada: `WATERFALL_COLUMNS × WATERFALL_ROWS` en dBFS, columna a columna.
    pub waterfall: Vec<f32>,
    /// Próxima columna que se escribe (la más antigua).
    pub waterfall_next: usize,
    /// Columnas escritas desde el último repintado completo.
    pub waterfall_dirty: Vec<usize>,
    last_frames: u64,
}

impl SpectrumView {
    /// Vista con este modo y suavizado.
    pub fn new(mode: SpectrumMode, fraction: u32) -> Self {
        Self {
            mode,
            fraction,
            cursor: None,
            rate: 0,
            grid: Vec::new(),
            target: Vec::new(),
            instant: Ballistics::new(DISPLAY_TAU_S * 0.4),
            average: Ballistics::new(DISPLAY_TAU_S),
            peak: Ballistics::new(DISPLAY_TAU_S * 0.4),
            bars: Ballistics::new(DISPLAY_TAU_S * 0.4),
            curves: [Vec::new(), Vec::new(), Vec::new()],
            bar_values: Vec::new(),
            waterfall: vec![DB_MIN; WATERFALL_COLUMNS * WATERFALL_ROWS],
            waterfall_next: 0,
            waterfall_dirty: Vec::new(),
            last_frames: 0,
        }
    }

    /// Frecuencia máxima mostrada.
    pub fn f_max(&self) -> f64 {
        f64::from(self.rate.max(2)) / 2.0
    }

    /// Frecuencias de la curva.
    pub fn grid(&self) -> &[f64] {
        &self.grid
    }

    /// Olvida curvas e historial (al desarmar o cambiar de dispositivo).
    pub fn reset(&mut self) {
        for b in [
            &mut self.instant,
            &mut self.average,
            &mut self.peak,
            &mut self.bars,
        ] {
            b.reset();
        }
        for c in &mut self.curves {
            c.clear();
        }
        self.bar_values.clear();
        self.waterfall.iter_mut().for_each(|v| *v = DB_MIN);
        self.waterfall_next = 0;
        self.waterfall_dirty.clear();
        self.last_frames = 0;
        self.rate = 0;
    }

    /// Avanza `dt_s` segundos con la instantánea `snap`.
    pub fn update(&mut self, snap: &LiveSnapshot, dt_s: f64) {
        let l = &snap.spectrum;
        if l.average.len() < 2 || snap.rate == 0 {
            return;
        }
        if snap.rate != self.rate {
            self.reset();
            self.rate = snap.rate;
            self.grid = log_grid(CURVE_POINTS, SPECTRUM_F_MIN, self.f_max());
        }
        let frac = f64::from(self.fraction.max(1));
        let f_max = self.f_max();
        match self.mode {
            SpectrumMode::Curve => {
                for (layer, ball, out) in [
                    (&l.instant, &mut self.instant, 0),
                    (&l.average, &mut self.average, 1),
                    (&l.peak, &mut self.peak, 2),
                ] {
                    smooth_curve(layer, l.hz_per_bin, &self.grid, frac, &mut self.target);
                    let shown = ball.update(&self.target, dt_s);
                    self.curves[out].clear();
                    self.curves[out].extend_from_slice(shown);
                }
            }
            SpectrumMode::Bars => {
                log_bands(
                    &l.instant,
                    l.hz_per_bin,
                    BARS,
                    SPECTRUM_F_MIN,
                    f_max,
                    &mut self.target,
                );
                let shown = self.bars.update(&self.target, dt_s);
                self.bar_values.clear();
                self.bar_values.extend_from_slice(shown);
            }
            SpectrumMode::Waterfall => {}
        }
        // La cascada se alimenta siempre, para que al cambiar de vista ya tenga historia.
        if l.frames != self.last_frames {
            self.last_frames = l.frames;
            let rows = log_grid(WATERFALL_ROWS, SPECTRUM_F_MIN, f_max);
            smooth_curve(&l.instant, l.hz_per_bin, &rows, 12.0, &mut self.target);
            let c = self.waterfall_next;
            self.waterfall[c * WATERFALL_ROWS..(c + 1) * WATERFALL_ROWS]
                .copy_from_slice(&self.target);
            self.waterfall_dirty.push(c);
            self.waterfall_next = (c + 1) % WATERFALL_COLUMNS;
        }
    }

    /// Frecuencia en la posición horizontal `x` de un área de anchura `w` con margen izquierdo
    /// `left`.
    pub fn freq_at(&self, x: f64, left: f64, w: f64) -> f64 {
        let t = ((x - left) / (w - left).max(1.0)).clamp(0.0, 1.0);
        SPECTRUM_F_MIN * (self.f_max() / SPECTRUM_F_MIN).powf(t)
    }

    /// Nivel de la capa media en `hz` (interpolado en la curva mostrada), en dBFS.
    pub fn average_at(&self, hz: f64, snap: &LiveSnapshot) -> f32 {
        let c = &self.curves[1];
        if c.len() == self.grid.len() && !c.is_empty() {
            let i = self.grid.partition_point(|&g| g < hz).min(c.len() - 1);
            if i == 0 {
                return c[0];
            }
            let (f0, f1) = (self.grid[i - 1], self.grid[i]);
            let t = ((hz.ln() - f0.ln()) / (f1.ln() - f0.ln())).clamp(0.0, 1.0) as f32;
            return c[i - 1] + t * (c[i] - c[i - 1]);
        }
        let l = &snap.spectrum;
        audio_core::dsp::frame::power_to_db(interp_power(&l.average, l.hz_per_bin, hz))
    }
}

/// Posición (0–1) de un nivel en el degradado de la cascada, con respuesta cuadrática: el ruido
/// y los niveles medios quedan oscuros y solo lo fuerte llega a los colores claros del tema.
pub fn waterfall_frac(db: f32) -> f64 {
    let x = f64::from(
        ((db - WATERFALL_DB_MIN) / (WATERFALL_DB_MAX - WATERFALL_DB_MIN)).clamp(0.0, 1.0),
    );
    x * x
}

/// Fracción vertical (0 abajo, 1 arriba) de un nivel en la escala del espectro.
pub fn db_frac(db: f32) -> f64 {
    f64::from(((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use digityzer_dsp::LiveAnalyzer;
    use digityzer_testkit::{interleave, quantize, sine};

    fn snapshot(freq: f64, rate: u32) -> LiveSnapshot {
        let s = quantize(&sine(freq, rate, 3.0, 0.5), 24);
        let mut a = LiveAnalyzer::new(rate, 24);
        a.push_interleaved(&interleave(&[s.clone(), s]));
        let mut snap = LiveSnapshot::default();
        a.snapshot_into(&mut snap);
        snap
    }

    #[test]
    fn cascada_oscura_en_el_ruido() {
        assert_eq!(waterfall_frac(-200.0), 0.0);
        assert_eq!(waterfall_frac(0.0), 1.0);
        assert!(waterfall_frac(-70.0) < 0.1);
        assert!(waterfall_frac(-15.0) > 0.85);
    }

    #[test]
    fn modos_e_identificadores() {
        for m in SpectrumMode::ALL {
            assert_eq!(SpectrumMode::from_id(m.id()), m);
        }
        assert_eq!(SpectrumMode::from_id("?"), SpectrumMode::Curve);
    }

    /// T2.7: el cursor sobre un tono de 62,1 Hz lo lee como «B1 +10 cents» a su nivel.
    #[test]
    fn cursor_sobre_un_tono_grave() {
        let snap = snapshot(62.1, 96_000);
        let mut v = SpectrumView::new(SpectrumMode::Curve, 12);
        v.update(&snap, 1.0);
        let (left, w) = (34.0, 834.0);
        // Posición x de 62,1 Hz y vuelta.
        let t = (62.1f64 / SPECTRUM_F_MIN).ln() / (v.f_max() / SPECTRUM_F_MIN).ln();
        let x = left + t * (w - left);
        let hz = v.freq_at(x, left, w);
        assert!((hz - 62.1).abs() < 0.01, "{hz}");
        assert_eq!(digityzer_dsp::spectrum::note_text(hz), "B1 +10 cents");
        // El máximo de la curva media está en el tono.
        let c = &v.curves[1];
        let i = c
            .iter()
            .enumerate()
            .fold((0, f32::MIN), |a, (i, &d)| if d > a.1 { (i, d) } else { a })
            .0;
        assert!((v.grid()[i] / 62.1 - 1.0).abs() < 0.03, "{}", v.grid()[i]);
        assert!(v.average_at(62.1, &snap) > -20.0);
    }

    #[test]
    fn barras_y_cascada() {
        let snap = snapshot(1000.0, 48_000);
        let mut v = SpectrumView::new(SpectrumMode::Bars, 6);
        v.update(&snap, 1.0);
        assert_eq!(v.bar_values.len(), BARS);
        assert_eq!(v.waterfall_dirty, vec![0]);
        assert_eq!(v.waterfall_next, 1);
        // Sin espectro nuevo no se añade columna.
        v.update(&snap, 1.0 / 60.0);
        assert_eq!(v.waterfall_next, 1);
        let col = &v.waterfall[..WATERFALL_ROWS];
        assert!(col.iter().copied().fold(f32::MIN, f32::max) > -20.0);
        v.reset();
        assert!(v.waterfall.iter().all(|&d| d == DB_MIN));
    }
}
