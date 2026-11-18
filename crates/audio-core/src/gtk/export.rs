//! Exportar el espectrograma a PNG con ejes (F5-T6).
//!
//! El hilo principal dibuja con cairo/Pango solo la capa de ejes, textos y rejilla (rápido) y la
//! copia en un búfer `Send`. Un trabajador renderiza la imagen del espectrograma, la escala a la
//! zona del gráfico, compone la capa encima y codifica el PNG.

use std::path::Path;
use std::sync::Arc;

use crate::palette::{Palette, RgbaImage, render_rgba};
use crate::spectrogram::SpectrogramData;

/// Rectángulo en píxeles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Izquierda.
    pub x: usize,
    /// Arriba.
    pub y: usize,
    /// Ancho.
    pub w: usize,
    /// Alto.
    pub h: usize,
}

/// Todo lo que necesita el trabajador (solo tipos `Send`).
pub struct ExportJob {
    /// Capa de ejes en ARGB32 de cairo (BGRA premultiplicado en little-endian).
    pub overlay: Vec<u8>,
    /// Bytes por fila de `overlay`.
    pub stride: usize,
    /// Ancho total.
    pub width: usize,
    /// Alto total.
    pub height: usize,
    /// Zona del gráfico (transparente en la capa).
    pub plot: Rect,
    /// Datos del espectrograma.
    pub data: Arc<SpectrogramData>,
    /// Paleta.
    pub palette: Palette,
    /// dB del color mínimo.
    pub db_min: f32,
    /// dB del color máximo.
    pub db_max: f32,
}

/// Compone la imagen final en RGB: dentro de `plot`, la imagen escalada (vecino más cercano)
/// con la capa encima; fuera, la capa tal cual (es opaca).
pub fn compose(job: &ExportJob, img: &RgbaImage) -> Vec<u8> {
    let (w, h) = (job.width, job.height);
    let mut out = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let o = y * job.stride + x * 4;
            let (ob, og, or, oa) = match job.overlay.get(o..o + 4) {
                Some(p) => (p[0], p[1], p[2], p[3]),
                None => (0, 0, 0, 0),
            };
            let p = &job.plot;
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            if x >= p.x
                && x < p.x + p.w
                && y >= p.y
                && y < p.y + p.h
                && img.width > 0
                && img.height > 0
            {
                let sx = ((x - p.x) * img.width / p.w).min(img.width - 1);
                let sy = ((y - p.y) * img.height / p.h).min(img.height - 1);
                let i = (sy * img.width + sx) * 4;
                if let Some(px) = img.data.get(i..i + 3) {
                    (r, g, b) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
                }
            }
            // Capa premultiplicada encima: c = capa + fondo · (1 − alfa).
            let inv = 255 - u32::from(oa);
            let blend =
                |ov: u8, base: u32| (u32::from(ov) + (base * inv + 127) / 255).min(255) as u8;
            let d = (y * w + x) * 3;
            out[d] = blend(or, r);
            out[d + 1] = blend(og, g);
            out[d + 2] = blend(ob, b);
        }
    }
    out
}

/// Renderiza, compone y guarda el PNG (en un trabajador).
pub fn run(job: &ExportJob, path: &Path) -> std::io::Result<()> {
    let img = render_rgba(&*job.data, job.palette, job.db_min, job.db_max);
    let rgb = compose(job, &img);
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, job.width as u32, job.height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&rgb).map_err(std::io::Error::other)?;
    }
    crate::fsutil::write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Arc<SpectrogramData> {
        Arc::new(SpectrogramData {
            columns: 2,
            rows: 2,
            sample_rate: 44_100,
            duration_s: 1.0,
            hz_per_row: 11_025.0,
            seconds_per_column: 0.5,
            db: vec![0.0; 4],
        })
    }

    #[test]
    fn composicion_de_la_capa_sobre_la_imagen() {
        // 4×2: la columna 0 es "eje" opaco gris, el resto gráfico transparente salvo un píxel
        // blanco al 50 %.
        let (w, h, stride) = (4, 2, 16);
        let mut overlay = vec![0u8; stride * h];
        for y in 0..h {
            overlay[y * stride..y * stride + 4].copy_from_slice(&[40, 40, 40, 255]);
        }
        overlay[2 * 4..2 * 4 + 4].copy_from_slice(&[128, 128, 128, 128]);
        let job = ExportJob {
            overlay,
            stride,
            width: w,
            height: h,
            plot: Rect {
                x: 1,
                y: 0,
                w: 3,
                h: 2,
            },
            data: data(),
            palette: Palette::Gray,
            db_min: -120.0,
            db_max: 0.0,
        };
        let img = RgbaImage {
            width: 1,
            height: 1,
            data: vec![200, 100, 0, 255],
        };
        let out = compose(&job, &img);
        assert_eq!(&out[0..3], &[40, 40, 40], "eje opaco");
        assert_eq!(&out[3..6], &[200, 100, 0], "imagen sin capa");
        assert_eq!(&out[6..9], &[228, 178, 128], "blanco al 50 % encima");
    }

    #[test]
    fn png_valido() {
        let dir = digityzer_testkit::temp_dir();
        let p = dir.path().join("x.png");
        let job = ExportJob {
            overlay: vec![0; 8 * 4 * 4],
            stride: 32,
            width: 8,
            height: 4,
            plot: Rect {
                x: 0,
                y: 0,
                w: 8,
                h: 4,
            },
            data: data(),
            palette: Palette::Magma,
            db_min: -120.0,
            db_max: 0.0,
        };
        run(&job, &p).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[1..4], b"PNG");
    }
}
