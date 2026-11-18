//! Dibujo de las gráficas con cairo (T2.2–T2.9). **Ningún color fijo**: todo sale de los roles
//! del tema ([`Theme`]); un test lo comprueba leyendo este archivo (T2.9d). GTK compone el
//! resultado en la GPU.

use audio_core::theme::{Rgb, Theme};
use digityzer_dsp::LiveSnapshot;
use digityzer_dsp::live::SPECTRUM_F_MIN;
use digityzer_dsp::spectrum::note_text;
use gtk4::cairo::{self, Context};

use super::logic::{METER_DB_MIN, Zone, db_text, log_x, meter_fraction, zone};
use super::spectrum_view::{
    DB_MAX, DB_MIN, SpectrumMode, SpectrumView, WATERFALL_COLUMNS, WATERFALL_ROWS, db_frac,
};

/// Margen izquierdo del espectro (escala de dB).
pub const SPECTRUM_LEFT: f64 = 34.0;

fn set(cr: &Context, c: Rgb, alpha: f64) {
    let (r, g, b) = c.unit();
    cr.set_source_rgba(r, g, b, alpha);
}

fn background(cr: &Context, th: &Theme, w: f64, h: f64) {
    set(cr, th.grafica, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
}

fn hline(cr: &Context, th: &Theme, x: f64, y: f64, w: f64) {
    set(cr, th.rejilla, 1.0);
    cr.rectangle(x, y.floor(), w, 1.0);
    let _ = cr.fill();
}

fn vline(cr: &Context, th: &Theme, x: f64, y: f64, h: f64) {
    set(cr, th.rejilla, 1.0);
    cr.rectangle(x.floor(), y, 1.0, h);
    let _ = cr.fill();
}

fn text(cr: &Context, c: Rgb, s: &str, x: f64, y: f64, size: f64) {
    set(cr, c, 1.0);
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(size);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

fn zone_color(th: &Theme, z: Zone) -> Rgb {
    match z {
        Zone::Normal => th.ok,
        Zone::High => th.aviso,
        Zone::Hot => th.clip,
    }
}

/// Medidores: pico por zonas (barra translúcida), RMS (barra interior), retención (línea) y
/// clipping. `clipped[c]` = ha habido clipping desde que se armó o se empezó a grabar.
pub fn meters(cr: &Context, th: &Theme, w: f64, h: f64, snap: &LiveSnapshot, clipped: [bool; 2]) {
    background(cr, th, w, h);
    let (top, bottom) = (22.0, h - 22.0);
    let bar_h = (bottom - top).max(1.0);
    let scale_w = 26.0;
    let bar_w = ((w - scale_w - 18.0) / 2.0).max(4.0);
    for db in (METER_DB_MIN as i32..=0).step_by(6) {
        let y = bottom - meter_fraction(db as f32) * bar_h;
        hline(cr, th, scale_w, y, w - scale_w);
        text(
            cr,
            th.tenue,
            &db.to_string().replace('-', "−"),
            2.0,
            y + 4.0,
            9.0,
        );
    }
    for (c, level) in snap.levels.iter().enumerate() {
        let x = scale_w + 6.0 + c as f64 * (bar_w + 6.0);
        let peak = meter_fraction(level.peak_dbfs);
        for (lo, hi) in [(METER_DB_MIN, -18.0), (-18.0, -3.0), (-3.0, 0.0)] {
            let (a, b) = (meter_fraction(lo), meter_fraction(hi).min(peak));
            if b > a {
                set(cr, zone_color(th, zone(lo + 0.01)), 0.45);
                cr.rectangle(x, bottom - b * bar_h, bar_w, (b - a) * bar_h);
                let _ = cr.fill();
            }
        }
        let rms = meter_fraction(level.rms_dbfs);
        set(cr, zone_color(th, zone(level.rms_dbfs)), 1.0);
        cr.rectangle(
            x + bar_w * 0.25,
            bottom - rms * bar_h,
            bar_w * 0.5,
            rms * bar_h,
        );
        let _ = cr.fill();
        let hold = meter_fraction(level.peak_hold_dbfs);
        set(cr, zone_color(th, zone(level.peak_hold_dbfs)), 1.0);
        cr.rectangle(x, bottom - hold * bar_h - 1.0, bar_w, 2.0);
        let _ = cr.fill();
        if clipped[c] {
            set(cr, th.clip, 1.0);
            cr.rectangle(x, 2.0, bar_w, 16.0);
            let _ = cr.fill();
            text(cr, th.grafica, "CLIP", x + 2.0, 14.0, 10.0);
        } else {
            let lr = if c == 0 { "L" } else { "R" };
            text(cr, th.tenue, lr, x + bar_w / 2.0 - 3.0, 14.0, 11.0);
        }
        text(
            cr,
            th.texto,
            &db_text(level.peak_hold_dbfs),
            x,
            h - 6.0,
            10.0,
        );
    }
}

/// Ejes del espectro: dB a la izquierda, frecuencia abajo (logarítmica).
fn spectrum_axes(cr: &Context, th: &Theme, w: f64, h: f64, f_max: f64) -> (f64, f64, f64) {
    let (left, bottom) = (SPECTRUM_LEFT, h - 18.0);
    let (pw, ph) = ((w - left - 6.0).max(1.0), (bottom - 6.0).max(1.0));
    for db in (-120..=0).step_by(20) {
        let y = bottom - db_frac(db as f32) * ph;
        hline(cr, th, left, y, pw);
        text(
            cr,
            th.tenue,
            &db.to_string().replace('-', "−"),
            2.0,
            y + 4.0,
            9.0,
        );
    }
    for (hz, label) in [
        (30.0, "30"),
        (50.0, "50"),
        (100.0, "100"),
        (200.0, "200"),
        (500.0, "500"),
        (1_000.0, "1k"),
        (2_000.0, "2k"),
        (5_000.0, "5k"),
        (10_000.0, "10k"),
        (20_000.0, "20k"),
        (40_000.0, "40k"),
    ] {
        if hz > f_max {
            continue;
        }
        let x = left + log_x(hz, SPECTRUM_F_MIN, f_max) * pw;
        vline(cr, th, x, 6.0, ph);
        text(cr, th.tenue, label, x + 2.0, h - 5.0, 9.0);
    }
    (left, bottom, ph)
}

/// Camino suave (Catmull-Rom → Bézier) por los puntos.
fn smooth_path(cr: &Context, pts: &[(f64, f64)]) {
    let Some(&first) = pts.first() else {
        return;
    };
    cr.move_to(first.0, first.1);
    let n = pts.len();
    for i in 0..n.saturating_sub(1) {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(n - 1)];
        cr.curve_to(
            p1.0 + (p2.0 - p0.0) / 6.0,
            p1.1 + (p2.1 - p0.1) / 6.0,
            p2.0 - (p3.0 - p1.0) / 6.0,
            p2.1 - (p3.1 - p1.1) / 6.0,
            p2.0,
            p2.1,
        );
    }
}

/// Espectro en la vista elegida. `wf` es el lienzo de la cascada (se actualiza aquí).
pub fn spectrum(
    cr: &Context,
    th: &Theme,
    w: f64,
    h: f64,
    view: &mut SpectrumView,
    snap: &LiveSnapshot,
    wf: &mut Waterfall,
) {
    background(cr, th, w, h);
    let f_max = view.f_max();
    if view.mode == SpectrumMode::Waterfall {
        waterfall(cr, th, w, h, view, wf);
        return;
    }
    let (left, bottom, ph) = spectrum_axes(cr, th, w, h, f_max);
    let pw = (w - left - 6.0).max(1.0);
    let y_of = |db: f32| bottom - db_frac(db) * ph;
    match view.mode {
        SpectrumMode::Curve => {
            let grid = view.grid();
            let pts = |c: &[f32]| -> Vec<(f64, f64)> {
                c.iter()
                    .zip(grid)
                    .map(|(&d, &f)| (left + log_x(f, SPECTRUM_F_MIN, f_max) * pw, y_of(d)))
                    .collect()
            };
            let [inst, avg, peak] = &view.curves;
            if inst.len() == grid.len() && !grid.is_empty() {
                let p = pts(inst);
                smooth_path(cr, &p);
                cr.line_to(left + pw, bottom);
                cr.line_to(left, bottom);
                cr.close_path();
                set(cr, th.curva, 0.35);
                let _ = cr.fill();
                smooth_path(cr, &p);
                set(cr, th.curva, 0.9);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
            if avg.len() == grid.len() && !grid.is_empty() {
                smooth_path(cr, &pts(avg));
                set(cr, th.media, 1.0);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
            if peak.len() == grid.len() && !grid.is_empty() {
                smooth_path(cr, &pts(peak));
                set(cr, th.picos, 0.7);
                cr.set_line_width(0.8);
                let _ = cr.stroke();
            }
        }
        SpectrumMode::Bars => {
            let n = view.bar_values.len();
            if n > 0 {
                let bw = pw / n as f64;
                for (i, &d) in view.bar_values.iter().enumerate() {
                    let y = y_of(d);
                    cr.rectangle(
                        left + i as f64 * bw + 0.5,
                        y,
                        (bw - 1.0).max(1.0),
                        bottom - y,
                    );
                }
                set(cr, th.curva, 0.35);
                let _ = cr.fill_preserve();
                set(cr, th.curva, 1.0);
                cr.set_line_width(0.8);
                let _ = cr.stroke();
            }
        }
        SpectrumMode::Waterfall => {}
    }
    // Cursor: frecuencia, nota y nivel de la capa media.
    if let (Some((x, _)), true) = (view.cursor, view.mode == SpectrumMode::Curve)
        && x >= left
        && x <= left + pw
    {
        let hz = view.freq_at(x, left, left + pw);
        set(cr, th.picos, 0.9);
        cr.rectangle(x.floor(), 6.0, 1.0, ph);
        let _ = cr.fill();
        let db = view.average_at(hz, snap);
        let label = format!(
            "{} · {} · {} dB",
            hz_text(hz),
            note_text(hz),
            db_text(db.clamp(DB_MIN, DB_MAX))
        );
        let lx = if x + 210.0 > w { x - 214.0 } else { x + 6.0 };
        set(cr, th.fondo, 0.9);
        cr.rectangle(lx - 3.0, 8.0, 208.0, 18.0);
        let _ = cr.fill();
        text(cr, th.texto, &label, lx, 21.0, 11.0);
    }
}

/// Frecuencia legible: `62,1 Hz`, `1,25 kHz`.
pub fn hz_text(hz: f64) -> String {
    let s = if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{hz:.1} Hz")
    };
    s.replace('.', ",")
}

/// Lienzo de la cascada: imagen de `WATERFALL_COLUMNS × WATERFALL_ROWS` que se repinta por
/// columnas nuevas, o entera al cambiar de tema.
pub struct Waterfall {
    surface: Option<cairo::ImageSurface>,
    theme: Option<&'static str>,
    lut: Vec<[u8; 3]>,
}

impl Default for Waterfall {
    fn default() -> Self {
        Self::new()
    }
}

impl Waterfall {
    /// Lienzo vacío.
    pub fn new() -> Self {
        Self {
            surface: None,
            theme: None,
            lut: Vec::new(),
        }
    }

    fn sync(&mut self, th: &Theme, view: &mut SpectrumView) {
        if self.surface.is_none() {
            self.surface = cairo::ImageSurface::create(
                cairo::Format::Rgb24,
                WATERFALL_COLUMNS as i32,
                WATERFALL_ROWS as i32,
            )
            .ok();
            self.theme = None;
        }
        let full = self.theme != Some(th.id);
        if full {
            self.theme = Some(th.id);
            self.lut = (0..256)
                .map(|i| {
                    let c = th.waterfall_at(f64::from(i) / 255.0);
                    [c.0, c.1, c.2]
                })
                .collect();
        }
        let cols: Vec<usize> = if full {
            view.waterfall_dirty.clear();
            (0..WATERFALL_COLUMNS).collect()
        } else {
            std::mem::take(&mut view.waterfall_dirty)
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let stride = surface.stride() as usize;
        surface.flush();
        if let Ok(mut data) = surface.data() {
            for c in cols {
                let column = &view.waterfall[c * WATERFALL_ROWS..(c + 1) * WATERFALL_ROWS];
                for (row, &db) in column.iter().enumerate() {
                    let y = WATERFALL_ROWS - 1 - row;
                    let k = (db_frac(db) * 255.0).round() as usize;
                    let [r, g, b] = self.lut[k.min(255)];
                    let at = y * stride + c * 4;
                    data[at] = b;
                    data[at + 1] = g;
                    data[at + 2] = r;
                    data[at + 3] = 0xff;
                }
            }
        }
        surface.mark_dirty();
    }
}

fn waterfall(
    cr: &Context,
    th: &Theme,
    w: f64,
    h: f64,
    view: &mut SpectrumView,
    wf: &mut Waterfall,
) {
    wf.sync(th, view);
    let Some(surface) = wf.surface.as_ref() else {
        return;
    };
    let (sx, sy) = (w / WATERFALL_COLUMNS as f64, h / WATERFALL_ROWS as f64);
    let next = view.waterfall_next;
    // Lo más antiguo a la izquierda: [next, W) y después [0, next).
    for (start, len, dest) in [
        (next, WATERFALL_COLUMNS - next, 0),
        (0, next, WATERFALL_COLUMNS - next),
    ] {
        if len == 0 {
            continue;
        }
        let _ = cr.save();
        cr.rectangle(dest as f64 * sx, 0.0, len as f64 * sx, h);
        cr.clip();
        cr.scale(sx, sy);
        let _ = cr.set_source_surface(surface, dest as f64 - start as f64, 0.0);
        cr.source().set_filter(cairo::Filter::Bilinear);
        let _ = cr.paint();
        let _ = cr.restore();
    }
    // Marcas de frecuencia a la derecha.
    for (hz, label) in [(100.0, "100"), (1_000.0, "1k"), (10_000.0, "10k")] {
        if hz <= view.f_max() {
            let y = h - log_x(hz, SPECTRUM_F_MIN, view.f_max()) * h;
            set(cr, th.texto, 0.5);
            cr.rectangle(w - 14.0, y.floor(), 14.0, 1.0);
            let _ = cr.fill();
            text(cr, th.texto, label, w - 30.0, y - 2.0, 9.0);
        }
    }
}

/// Goniómetro (medio arriba, lado a los lados) y barra de correlación debajo.
pub fn goniometer(cr: &Context, th: &Theme, w: f64, h: f64, snap: &LiveSnapshot) {
    background(cr, th, w, h);
    let bar = 24.0;
    let side = (w.min(h - bar) - 8.0).max(4.0);
    let (cx, cy) = (w / 2.0, 4.0 + side / 2.0);
    let r = side / 2.0;
    set(cr, th.rejilla, 1.0);
    cr.set_line_width(1.0);
    for (x0, y0, x1, y1) in [
        (cx, cy - r, cx, cy + r),
        (cx - r, cy, cx + r, cy),
        (cx - r, cy - r, cx + r, cy + r),
        (cx + r, cy - r, cx - r, cy + r),
    ] {
        cr.move_to(x0, y0);
        cr.line_to(x1, y1);
    }
    let _ = cr.stroke();
    text(cr, th.tenue, "M", cx + 3.0, cy - r + 10.0, 9.0);
    text(cr, th.tenue, "L", cx - r + 2.0, cy - r + 10.0, 9.0);
    text(cr, th.tenue, "R", cx + r - 9.0, cy - r + 10.0, 9.0);
    set(cr, th.ok, 0.6);
    for &(s, m) in &snap.gonio {
        cr.rectangle(cx + f64::from(s) * r, cy - f64::from(m) * r, 1.5, 1.5);
    }
    let _ = cr.fill();
    let y = h - bar + 6.0;
    let (x0, bw) = (8.0, w - 16.0);
    set(cr, th.rejilla, 1.0);
    cr.rectangle(x0, y, bw, 8.0);
    let _ = cr.fill();
    if let Some(c) = snap.correlation {
        let x = x0 + (f64::from(c) + 1.0) / 2.0 * bw;
        let col = if c < 0.0 {
            th.clip
        } else if c < 0.3 {
            th.aviso
        } else {
            th.ok
        };
        set(cr, col, 1.0);
        cr.rectangle(x - 2.0, y - 2.0, 4.0, 12.0);
        let _ = cr.fill();
        let shown = if c.abs() < 0.005 { 0.0 } else { c };
        text(
            cr,
            th.texto,
            &format!("{shown:+.2}").replace('.', ","),
            x0,
            h - 2.0,
            9.0,
        );
    }
    text(cr, th.tenue, "−1", x0, y - 2.0, 8.0);
    text(cr, th.tenue, "+1", x0 + bw - 12.0, y - 2.0, 8.0);
}

/// Forma de onda de los últimos segundos; lo más reciente a la derecha. Las columnas que tocan
/// el fondo de escala se marcan en `clip`.
pub fn waveform(cr: &Context, th: &Theme, w: f64, h: f64, snap: &LiveSnapshot, recording: bool) {
    background(cr, th, w, h);
    let mid = h / 2.0;
    hline(cr, th, 0.0, mid, w);
    let n = snap.waveform.len();
    if n == 0 {
        return;
    }
    let col_w = (w / digityzer_dsp::live::WAVEFORM_COLUMNS as f64).max(0.5);
    let alpha = if recording { 1.0 } else { 0.6 };
    for pass_clip in [false, true] {
        for (i, &(lo, hi)) in snap.waveform.iter().enumerate() {
            let clipped = hi >= 0.999 || lo <= -0.999;
            if clipped != pass_clip {
                continue;
            }
            let x = w - (n - i) as f64 * col_w;
            if x + col_w < 0.0 {
                continue;
            }
            let y0 = mid - f64::from(hi.clamp(-1.0, 1.0)) * mid;
            let y1 = mid - f64::from(lo.clamp(-1.0, 1.0)) * mid;
            cr.rectangle(x, y0, col_w.max(1.0), (y1 - y0).max(1.0));
        }
        set(cr, if pass_clip { th.clip } else { th.curva }, alpha);
        let _ = cr.fill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T2.9d: el código de las gráficas no tiene colores fijos (ni hexadecimales ni componentes
    /// numéricos en las llamadas de color de cairo).
    #[test]
    fn sin_colores_fijos() {
        let src = include_str!("draw.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        for (n, line) in code.lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with("//") {
                continue;
            }
            for bad in ["set_source_rgb(", "Rgb(", "Rgb::hex", "#ff", "0x"] {
                let ok = bad == "0x" && l.contains("0xff;"); // byte de alfa opaco del píxel
                assert!(ok || !l.contains(bad), "línea {}: {line}", n + 1);
            }
            if let Some(i) = l.find("set_source_rgba(") {
                let arg = &l[i + "set_source_rgba(".len()..];
                assert!(
                    !arg.starts_with(|c: char| c.is_ascii_digit()),
                    "línea {}",
                    n + 1
                );
            }
        }
    }

    #[test]
    fn frecuencias_legibles() {
        assert_eq!(hz_text(62.1), "62,1 Hz");
        assert_eq!(hz_text(1250.0), "1,25 kHz");
    }

    #[test]
    fn todas_las_graficas_en_todos_los_temas() {
        use audio_core::theme::THEMES;
        let snap = LiveSnapshot::default();
        let mut view = SpectrumView::new(SpectrumMode::Curve, 6);
        let mut wf = Waterfall::new();
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 200, 120).unwrap();
        let cr = Context::new(&surface).unwrap();
        for th in &THEMES {
            let th = th.effective();
            meters(&cr, &th, 200.0, 120.0, &snap, [true, false]);
            for m in SpectrumMode::ALL {
                view.mode = m;
                spectrum(&cr, &th, 200.0, 120.0, &mut view, &snap, &mut wf);
            }
            goniometer(&cr, &th, 200.0, 120.0, &snap);
            waveform(&cr, &th, 200.0, 120.0, &snap, true);
        }
    }
}
