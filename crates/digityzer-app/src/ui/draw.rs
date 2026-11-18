//! Dibujo de los medidores en vivo con cairo. Cada función recibe la instantánea y el tamaño;
//! GTK compone el resultado en la GPU.

use digityzer_dsp::LiveSnapshot;
use digityzer_dsp::live::{SPECTRUM_BANDS, SPECTRUM_F_MIN};
use gtk4::cairo::Context;

use super::logic::{METER_DB_MIN, Zone, db_text, log_x, meter_fraction, zone};

const BG: (f64, f64, f64) = (0.10, 0.10, 0.11);
const GRID: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.10);
const TEXT: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.65);
const GREEN: (f64, f64, f64) = (0.30, 0.78, 0.42);
const YELLOW: (f64, f64, f64) = (0.95, 0.77, 0.25);
const RED: (f64, f64, f64) = (0.93, 0.30, 0.27);
const ACCENT: (f64, f64, f64) = (0.40, 0.65, 0.98);

fn background(cr: &Context, w: f64, h: f64) {
    cr.set_source_rgb(BG.0, BG.1, BG.2);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
}

fn text(cr: &Context, s: &str, x: f64, y: f64, size: f64) {
    cr.set_source_rgba(TEXT.0, TEXT.1, TEXT.2, TEXT.3);
    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );
    cr.set_font_size(size);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

fn zone_rgb(z: Zone) -> (f64, f64, f64) {
    match z {
        Zone::Normal => GREEN,
        Zone::High => YELLOW,
        Zone::Hot => RED,
    }
}

/// Medidores de pico (barra) y RMS (barra interior), retención de pico (línea) y clipping.
/// `clipped[c]` = ha habido clipping desde que se armó o se empezó a grabar.
pub fn meters(cr: &Context, w: f64, h: f64, snap: &LiveSnapshot, clipped: [bool; 2]) {
    background(cr, w, h);
    let top = 22.0;
    let bottom = h - 22.0;
    let bar_h = (bottom - top).max(1.0);
    let scale_w = 26.0;
    let bar_w = ((w - scale_w - 18.0) / 2.0).max(4.0);
    // Escala.
    for db in (METER_DB_MIN as i32..=0).step_by(6) {
        let y = bottom - meter_fraction(db as f32) * bar_h;
        cr.set_source_rgba(GRID.0, GRID.1, GRID.2, GRID.3);
        cr.rectangle(scale_w, y, w - scale_w, 1.0);
        let _ = cr.fill();
        text(cr, &db.to_string().replace('-', "−"), 2.0, y + 4.0, 9.0);
    }
    for (c, level) in snap.levels.iter().enumerate() {
        let x = scale_w + 6.0 + c as f64 * (bar_w + 6.0);
        // Pico por zonas.
        let peak = meter_fraction(level.peak_dbfs);
        for (lo, hi) in [(METER_DB_MIN, -18.0), (-18.0, -6.0), (-6.0, 0.0)] {
            let (a, b) = (meter_fraction(lo), meter_fraction(hi).min(peak));
            if b > a {
                let (r, g, bl) = zone_rgb(zone(lo + 0.01));
                cr.set_source_rgba(r, g, bl, 0.55);
                cr.rectangle(x, bottom - b * bar_h, bar_w, (b - a) * bar_h);
                let _ = cr.fill();
            }
        }
        // RMS.
        let rms = meter_fraction(level.rms_dbfs);
        let (r, g, bl) = zone_rgb(zone(level.rms_dbfs));
        cr.set_source_rgb(r, g, bl);
        cr.rectangle(
            x + bar_w * 0.25,
            bottom - rms * bar_h,
            bar_w * 0.5,
            rms * bar_h,
        );
        let _ = cr.fill();
        // Retención.
        let hold = meter_fraction(level.peak_hold_dbfs);
        let (r, g, bl) = zone_rgb(zone(level.peak_hold_dbfs));
        cr.set_source_rgb(r, g, bl);
        cr.rectangle(x, bottom - hold * bar_h - 1.0, bar_w, 2.0);
        let _ = cr.fill();
        // Clipping y cifra.
        if clipped[c] {
            cr.set_source_rgb(RED.0, RED.1, RED.2);
            cr.rectangle(x, 2.0, bar_w, 16.0);
            let _ = cr.fill();
            text(cr, "CLIP", x + 2.0, 14.0, 10.0);
        } else {
            text(
                cr,
                if c == 0 { "L" } else { "R" },
                x + bar_w / 2.0 - 3.0,
                14.0,
                11.0,
            );
        }
        text(cr, &db_text(level.peak_hold_dbfs), x, h - 6.0, 10.0);
    }
}

/// Espectro en vivo con eje logarítmico de frecuencia y dBFS.
pub fn spectrum(cr: &Context, w: f64, h: f64, snap: &LiveSnapshot) {
    background(cr, w, h);
    let (db_min, db_max) = (-130.0f64, 0.0f64);
    let f_max = f64::from(snap.rate.max(2)) / 2.0;
    let (left, bottom) = (34.0, h - 18.0);
    let pw = (w - left - 6.0).max(1.0);
    let ph = (bottom - 6.0).max(1.0);
    let y_of = |db: f64| bottom - ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0) * ph;
    for db in (-120..=0).step_by(20) {
        let y = y_of(f64::from(db));
        cr.set_source_rgba(GRID.0, GRID.1, GRID.2, GRID.3);
        cr.rectangle(left, y, pw, 1.0);
        let _ = cr.fill();
        text(cr, &db.to_string().replace('-', "−"), 2.0, y + 4.0, 9.0);
    }
    for (hz, label) in [
        (50.0, "50"),
        (100.0, "100"),
        (1_000.0, "1k"),
        (10_000.0, "10k"),
        (20_000.0, "20k"),
        (40_000.0, "40k"),
    ] {
        if hz > f_max {
            continue;
        }
        let x = left + log_x(hz, SPECTRUM_F_MIN, f_max) * pw;
        cr.set_source_rgba(GRID.0, GRID.1, GRID.2, GRID.3);
        cr.rectangle(x, 6.0, 1.0, ph);
        let _ = cr.fill();
        text(cr, label, x + 2.0, h - 5.0, 9.0);
    }
    if snap.spectrum.is_empty() {
        return;
    }
    let n = snap.spectrum.len().min(SPECTRUM_BANDS);
    cr.move_to(left, bottom);
    for (i, &db) in snap.spectrum.iter().take(n).enumerate() {
        let x = left + (i as f64 + 0.5) / n as f64 * pw;
        cr.line_to(x, y_of(f64::from(db)));
    }
    cr.line_to(left + pw, bottom);
    cr.close_path();
    cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.35);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
}

/// Goniómetro (medio arriba, lado a los lados) y barra de correlación debajo.
pub fn goniometer(cr: &Context, w: f64, h: f64, snap: &LiveSnapshot) {
    background(cr, w, h);
    let bar = 24.0;
    let side = (w.min(h - bar) - 8.0).max(4.0);
    let (cx, cy) = (w / 2.0, 4.0 + side / 2.0);
    let r = side / 2.0;
    cr.set_source_rgba(GRID.0, GRID.1, GRID.2, GRID.3);
    cr.set_line_width(1.0);
    cr.move_to(cx, cy - r);
    cr.line_to(cx, cy + r);
    cr.move_to(cx - r, cy);
    cr.line_to(cx + r, cy);
    cr.move_to(cx - r, cy - r);
    cr.line_to(cx + r, cy + r);
    cr.move_to(cx + r, cy - r);
    cr.line_to(cx - r, cy + r);
    let _ = cr.stroke();
    text(cr, "M", cx + 3.0, cy - r + 10.0, 9.0);
    text(cr, "L", cx - r + 2.0, cy - r + 10.0, 9.0);
    text(cr, "R", cx + r - 9.0, cy - r + 10.0, 9.0);
    // Puntos (lado, medio) a escala: ±1 llega al borde.
    cr.set_source_rgba(GREEN.0, GREEN.1, GREEN.2, 0.55);
    for &(s, m) in &snap.gonio {
        let x = cx + f64::from(s) * r;
        let y = cy - f64::from(m) * r;
        cr.rectangle(x, y, 1.5, 1.5);
    }
    let _ = cr.fill();
    // Correlación −1 … +1.
    let y = h - bar + 6.0;
    let (x0, bw) = (8.0, w - 16.0);
    cr.set_source_rgba(GRID.0, GRID.1, GRID.2, 0.25);
    cr.rectangle(x0, y, bw, 8.0);
    let _ = cr.fill();
    if let Some(c) = snap.correlation {
        let x = x0 + (f64::from(c) + 1.0) / 2.0 * bw;
        let col = if c < 0.0 {
            RED
        } else if c < 0.3 {
            YELLOW
        } else {
            GREEN
        };
        cr.set_source_rgb(col.0, col.1, col.2);
        cr.rectangle(x - 2.0, y - 2.0, 4.0, 12.0);
        let _ = cr.fill();
        let shown = if c.abs() < 0.005 { 0.0 } else { c };
        text(
            cr,
            &format!("{shown:+.2}").replace('.', ","),
            x0,
            h - 2.0,
            9.0,
        );
    }
    text(cr, "−1", x0, y - 2.0, 8.0);
    text(cr, "+1", x0 + bw - 12.0, y - 2.0, 8.0);
}

/// Forma de onda de los últimos segundos; lo más reciente a la derecha.
pub fn waveform(cr: &Context, w: f64, h: f64, snap: &LiveSnapshot, recording: bool) {
    background(cr, w, h);
    let mid = h / 2.0;
    cr.set_source_rgba(GRID.0, GRID.1, GRID.2, GRID.3);
    cr.rectangle(0.0, mid, w, 1.0);
    let _ = cr.fill();
    let n = snap.waveform.len();
    if n == 0 {
        return;
    }
    let col_w = (w / digityzer_dsp::live::WAVEFORM_COLUMNS as f64).max(0.5);
    let color = if recording { RED } else { ACCENT };
    cr.set_source_rgba(color.0, color.1, color.2, 0.8);
    for (i, &(lo, hi)) in snap.waveform.iter().enumerate() {
        let x = w - (n - i) as f64 * col_w;
        if x + col_w < 0.0 {
            continue;
        }
        let y0 = mid - f64::from(hi.clamp(-1.0, 1.0)) * mid;
        let y1 = mid - f64::from(lo.clamp(-1.0, 1.0)) * mid;
        cr.rectangle(x, y0, col_w.max(1.0), (y1 - y0).max(1.0));
    }
    let _ = cr.fill();
}
