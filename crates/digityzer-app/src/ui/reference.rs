//! Capturas de referencia (T2.9f): todas las gráficas con una señal de prueba en cada tema, un
//! PNG por tema, para revisarlos de un vistazo. `digityzer render-themes <carpeta>`.
//!
//! La señal de prueba se genera aquí (sin el testkit, que es solo para tests): ruido rosa a
//! unos −30 dBFS, tonos de 62,1 Hz, 1 kHz y 8 kHz, el canal R algo distinto para que el
//! goniómetro tenga forma, y un último tramo recortado a fondo de escala para que se vean el
//! aviso CLIP y la forma de onda en `clip`.

use std::path::{Path, PathBuf};

use audio_core::theme::{THEMES, Theme};
use digityzer_dsp::{LiveAnalyzer, LiveSnapshot};
use gtk4::cairo::{self, Context};

use super::draw::{self, SPECTRUM_LEFT, Waterfall};
use super::spectrum_view::{SpectrumMode, SpectrumView};

/// Tamaño de cada captura.
pub const SIZE: (i32, i32) = (1400, 860);
const RATE: u32 = 96_000;

/// Señal de prueba estéreo, entrelazada, en enteros de 24 bits.
fn test_signal(secs: f64) -> Vec<i32> {
    let n = (secs * f64::from(RATE)) as usize;
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut b = [0.0f64; 7];
    let full = 8_388_607.0;
    let clip_from = n - (0.06 * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let w = (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
        b[0] = 0.99886 * b[0] + w * 0.0555179;
        b[1] = 0.99332 * b[1] + w * 0.0750759;
        b[2] = 0.96900 * b[2] + w * 0.1538520;
        b[3] = 0.86650 * b[3] + w * 0.3104856;
        b[4] = 0.55000 * b[4] + w * 0.5329522;
        b[5] = -0.7616 * b[5] - w * 0.0168980;
        let pink = (b.iter().sum::<f64>() + w * 0.5362) * 0.011;
        b[6] = w * 0.115926;
        let t = i as f64 / f64::from(RATE);
        let tau = std::f64::consts::TAU;
        let tones = 0.25 * (tau * 62.1 * t).sin()
            + 0.125 * (tau * 1000.0 * t).sin()
            + 0.03 * (tau * 8000.0 * t).sin();
        let l = pink + tones;
        let r = 0.8 * pink + 0.25 * (tau * 62.1 * t + 0.6).sin() + 0.125 * (tau * 1000.0 * t).sin();
        let (l, r) = if i >= clip_from {
            ((l * 8.0).clamp(-1.0, 1.0), (r * 8.0).clamp(-1.0, 1.0))
        } else {
            (l, r)
        };
        out.push((l * full).round() as i32);
        out.push((r * full).round() as i32);
    }
    out
}

/// Estado alimentado con la señal de prueba: instantánea y las tres vistas del espectro.
struct Scene {
    snap: LiveSnapshot,
    curve: SpectrumView,
    bars: SpectrumView,
    cascade: SpectrumView,
}

fn scene() -> Scene {
    let lr = test_signal(8.0);
    let mut a = LiveAnalyzer::new(RATE, 24);
    let mut snap = LiveSnapshot::default();
    let mut curve = SpectrumView::new(SpectrumMode::Curve, 6);
    let mut bars = SpectrumView::new(SpectrumMode::Bars, 6);
    let mut cascade = SpectrumView::new(SpectrumMode::Waterfall, 6);
    let chunk = (RATE / 60) as usize * 2;
    for c in lr.chunks(chunk) {
        a.push_interleaved(c);
        a.snapshot_into(&mut snap);
        for v in [&mut curve, &mut bars, &mut cascade] {
            v.update(&snap, 1.0 / 60.0);
        }
    }
    Scene {
        snap,
        curve,
        bars,
        cascade,
    }
}

fn panel(cr: &Context, x: f64, y: f64, f: impl FnOnce(&Context)) {
    let _ = cr.save();
    cr.translate(x, y);
    f(cr);
    let _ = cr.restore();
}

/// Dibuja la captura de un tema en `cr` (tamaño [`SIZE`]).
fn render(cr: &Context, th: &Theme, sc: &mut Scene) {
    let (w, h) = (f64::from(SIZE.0), f64::from(SIZE.1));
    let (r, g, b) = th.fondo.unit();
    cr.set_source_rgb(r, g, b);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
    let (r, g, b) = th.texto.unit();
    cr.set_source_rgb(r, g, b);
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(18.0);
    cr.move_to(16.0, 28.0);
    let _ = cr.show_text(&format!("Digityzer · {}", th.name));
    let m = 12.0;
    let top = 44.0;
    let row1 = 340.0;
    let (mw, gw) = (140.0, 300.0);
    let sw = w - mw - gw - 4.0 * m;
    panel(cr, m, top, |cr| {
        draw::meters(cr, th, mw, row1, &sc.snap, [true, false])
    });
    // Cursor de la curva sobre el tono de 62,1 Hz.
    let f_max = sc.curve.f_max();
    let t = (62.1f64 / 20.0).ln() / (f_max / 20.0).ln();
    sc.curve.cursor = Some((SPECTRUM_LEFT + t * (sw - SPECTRUM_LEFT - 6.0), 40.0));
    let mut wf = Waterfall::new();
    panel(cr, 2.0 * m + mw, top, |cr| {
        draw::spectrum(cr, th, sw, row1, &mut sc.curve, &sc.snap, &mut wf)
    });
    panel(cr, 3.0 * m + mw + sw, top, |cr| {
        draw::goniometer(cr, th, gw, row1, &sc.snap)
    });
    let row2 = 290.0;
    let half = (w - 3.0 * m) / 2.0;
    let y2 = top + row1 + m;
    panel(cr, m, y2, |cr| {
        draw::spectrum(cr, th, half, row2, &mut sc.bars, &sc.snap, &mut wf)
    });
    let mut wf2 = Waterfall::new();
    panel(cr, 2.0 * m + half, y2, |cr| {
        draw::spectrum(cr, th, half, row2, &mut sc.cascade, &sc.snap, &mut wf2)
    });
    let y3 = y2 + row2 + m;
    panel(cr, m, y3, |cr| {
        draw::waveform(cr, th, w - 2.0 * m, h - y3 - m, &sc.snap, true)
    });
}

/// Guarda una superficie ARGB32 como PNG RGB.
fn write_png(surface: &mut cairo::ImageSurface, path: &Path) -> anyhow::Result<()> {
    let (w, h) = (surface.width() as usize, surface.height() as usize);
    let stride = surface.stride() as usize;
    surface.flush();
    let data = surface.data()?;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let p = &data[y * stride + x * 4..y * stride + x * 4 + 4];
            rgb.extend_from_slice(&[p[2], p[1], p[0]]);
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

/// Renderiza los diez temas en `dir` (`<tema>.png`) y devuelve las rutas.
pub fn render_all(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut sc = scene();
    let mut out = Vec::new();
    for t in &THEMES {
        let th = t.effective();
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, SIZE.0, SIZE.1)?;
        {
            let cr = Context::new(&surface)?;
            render(&cr, &th, &mut sc);
        }
        let path = dir.join(format!("{}.png", t.id));
        write_png(&mut surface, &path)?;
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diez_capturas_con_el_fondo_de_su_tema() {
        let dir = tempfile::tempdir().unwrap();
        let files = render_all(dir.path()).unwrap();
        assert_eq!(files.len(), 10);
        for (f, t) in files.iter().zip(&THEMES) {
            let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(f).unwrap()));
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            assert_eq!((info.width, info.height), (SIZE.0 as u32, SIZE.1 as u32));
            // Esquina superior derecha: fondo del tema.
            let at = (SIZE.0 as usize - 2) * 3;
            assert_eq!(
                &buf[at..at + 3],
                &[t.fondo.0, t.fondo.1, t.fondo.2],
                "{}",
                t.id
            );
        }
    }
}
