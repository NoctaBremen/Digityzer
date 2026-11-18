//! Paletas de color y render del espectrograma a una imagen RGBA (sin GTK).
//!
//! La interfaz recibe un [`RgbaImage`] ya generado en un hilo trabajador y lo sube como textura.

use std::f64::consts::PI;

use crate::spectrogram::DbGrid;

/// Paleta de color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    /// Magma (matplotlib): perceptualmente uniforme, clara sobre fondo oscuro. Por defecto.
    #[default]
    Magma,
    /// "Spectrum" de Spek (algoritmo de Dan Bruton con corrección de intensidad).
    Spectrum,
    /// Paleta por defecto de SoX (Rob Sykes), también la de Spek.
    Sox,
    /// Escala de grises.
    Gray,
}

impl Palette {
    /// Todas las paletas, en el orden de la interfaz.
    pub const ALL: [Palette; 4] = [
        Palette::Magma,
        Palette::Spectrum,
        Palette::Sox,
        Palette::Gray,
    ];

    /// Nombre para la interfaz.
    pub fn name(self) -> &'static str {
        match self {
            Self::Magma => "Magma",
            Self::Spectrum => "Spectrum",
            Self::Sox => "SoX",
            Self::Gray => "Grises",
        }
    }

    /// Color para `level` en [0, 1] (0 = mínimo del rango, 1 = máximo). Fuera de rango se recorta.
    pub fn color(self, level: f32) -> [u8; 3] {
        let level = if level.is_nan() {
            0.0
        } else {
            level.clamp(0.0, 1.0)
        };
        match self {
            Self::Magma => {
                let x = f64::from(level) * 255.0;
                let i = (x.floor() as usize).min(254);
                let t = x - i as f64;
                let (a, b) = (MAGMA[i], MAGMA[i + 1]);
                [0, 1, 2].map(|k| (f64::from(a[k]) * (1.0 - t) + f64::from(b[k]) * t).round() as u8)
            }
            Self::Spectrum => spectrum(f64::from(level)),
            Self::Sox => sox(f64::from(level)),
            Self::Gray => {
                let v = (level * 255.0).round() as u8;
                [v, v, v]
            }
        }
    }
}

/// Imagen RGBA8 sin premultiplicar. `data.len() == width · height · 4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Ancho en píxeles.
    pub width: usize,
    /// Alto en píxeles.
    pub height: usize,
    /// Píxeles, fila a fila desde arriba.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Píxel `(x, y)` (`y = 0` arriba).
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }
}

/// Tamaño de la tabla de colores precalculada.
const LUT_SIZE: usize = 1024;

/// Renderiza una rejilla de dB: una columna por píxel en X y una fila por píxel en Y, con la
/// **fila 0 de la imagen arriba** (= Nyquist) y la última abajo (= 0 Hz).
///
/// `db_min` → color mínimo, `db_max` → máximo. Si `db_min ≥ db_max`, se intercambian (y si son
/// iguales, se usa un rango de 1 dB).
pub fn render_rgba(src: &impl DbGrid, palette: Palette, db_min: f32, db_max: f32) -> RgbaImage {
    let (lo, hi) = ordered_range(db_min, db_max);
    let lut: Vec<[u8; 3]> = (0..LUT_SIZE)
        .map(|i| palette.color(i as f32 / (LUT_SIZE - 1) as f32))
        .collect();
    let (w, h) = (src.columns(), src.rows());
    let scale = (LUT_SIZE - 1) as f32 / (hi - lo);
    let mut data = vec![255u8; w * h * 4];
    // Recorrido por bloques: el origen está por columnas y la imagen por filas; con bloques de
    // TILE × TILE ambos accesos quedan en caché.
    const TILE: usize = 64;
    let max_idx = (LUT_SIZE - 1) as f32;
    for x0 in (0..w).step_by(TILE) {
        let x1 = (x0 + TILE).min(w);
        for r0 in (0..h).step_by(TILE) {
            let r1 = (r0 + TILE).min(h);
            for x in x0..x1 {
                for row in r0..r1 {
                    let v = src.db_at(x, row);
                    // `as usize` satura y convierte NaN en 0 (color mínimo).
                    let idx = ((v - lo) * scale).clamp(0.0, max_idx) as usize;
                    let o = ((h - 1 - row) * w + x) * 4;
                    data[o..o + 3].copy_from_slice(&lut[idx]);
                }
            }
        }
    }
    RgbaImage {
        width: w,
        height: h,
        data,
    }
}

/// Barra vertical de la paleta (1 px de ancho, `height` px de alto, arriba = máximo).
pub fn palette_strip(palette: Palette, height: usize) -> RgbaImage {
    let mut data = Vec::with_capacity(height * 4);
    for y in 0..height {
        let level = if height > 1 {
            1.0 - y as f32 / (height - 1) as f32
        } else {
            1.0
        };
        let c = palette.color(level);
        data.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    RgbaImage {
        width: 1,
        height,
        data,
    }
}

fn ordered_range(a: f32, b: f32) -> (f32, f32) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if hi - lo < 1e-3 {
        (lo, lo + 1.0)
    } else {
        (lo, hi)
    }
}

fn pack(r: f64, g: f64, b: f64) -> [u8; 3] {
    [r, g, b].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// "Spectrum" de Spek: algoritmo de Dan Bruton modificado (`spek-palette.cc`).
fn spectrum(level: f64) -> [u8; 3] {
    let level = level * 0.6625;
    let (r, g, b) = if level < 0.15 {
        ((0.15 - level) / (0.15 + 0.075), 0.0, 1.0)
    } else if level < 0.275 {
        (0.0, (level - 0.15) / (0.275 - 0.15), 1.0)
    } else if level < 0.325 {
        (0.0, 1.0, (0.325 - level) / (0.325 - 0.275))
    } else if level < 0.5 {
        ((level - 0.325) / (0.5 - 0.325), 1.0, 0.0)
    } else {
        (1.0, (0.6625 - level) / (0.6625 - 0.5), 0.0)
    };
    // Corrección de intensidad: oscurece el extremo inferior.
    let cf = if level < 0.1 { level / 0.1 } else { 1.0 };
    pack(r * cf, g * cf, b * cf)
}

/// Paleta de SoX (Rob Sykes), como en Spek.
fn sox(level: f64) -> [u8; 3] {
    let r = if level >= 0.73 {
        1.0
    } else if level >= 0.13 {
        ((level - 0.13) / 0.60 * PI / 2.0).sin()
    } else {
        0.0
    };
    let g = if level >= 0.91 {
        1.0
    } else if level >= 0.6 {
        ((level - 0.6) / 0.31 * PI / 2.0).sin()
    } else {
        0.0
    };
    let b = if level < 0.60 {
        0.5 * (level / 0.6 * PI).sin()
    } else if level >= 0.78 {
        (level - 0.78) / 0.22
    } else {
        0.0
    };
    pack(r, g, b)
}

/// Mapa de color "magma" de matplotlib (256 entradas, dominio público / CC0).
#[rustfmt::skip]
const MAGMA: [[u8; 3]; 256] = [
    [0, 0, 4], [1, 0, 5], [1, 1, 6], [1, 1, 8], [2, 1, 9], [2, 2, 11], [2, 2, 13], [3, 3, 15],
    [3, 3, 18], [4, 4, 20], [5, 4, 22], [6, 5, 24], [6, 5, 26], [7, 6, 28], [8, 7, 30], [9, 7, 32],
    [10, 8, 34], [11, 9, 36], [12, 9, 38], [13, 10, 41], [14, 11, 43], [16, 11, 45], [17, 12, 47], [18, 13, 49],
    [19, 13, 52], [20, 14, 54], [21, 14, 56], [22, 15, 59], [24, 15, 61], [25, 16, 63], [26, 16, 66], [28, 16, 68],
    [29, 17, 71], [30, 17, 73], [32, 17, 75], [33, 17, 78], [34, 17, 80], [36, 18, 83], [37, 18, 85], [39, 18, 88],
    [41, 17, 90], [42, 17, 92], [44, 17, 95], [45, 17, 97], [47, 17, 99], [49, 17, 101], [51, 16, 103], [52, 16, 105],
    [54, 16, 107], [56, 16, 108], [57, 15, 110], [59, 15, 112], [61, 15, 113], [63, 15, 114], [64, 15, 116], [66, 15, 117],
    [68, 15, 118], [69, 16, 119], [71, 16, 120], [73, 16, 120], [74, 16, 121], [76, 17, 122], [78, 17, 123], [79, 18, 123],
    [81, 18, 124], [82, 19, 124], [84, 19, 125], [86, 20, 125], [87, 21, 126], [89, 21, 126], [90, 22, 126], [92, 22, 127],
    [93, 23, 127], [95, 24, 127], [96, 24, 128], [98, 25, 128], [100, 26, 128], [101, 26, 128], [103, 27, 128], [104, 28, 129],
    [106, 28, 129], [107, 29, 129], [109, 29, 129], [110, 30, 129], [112, 31, 129], [114, 31, 129], [115, 32, 129], [117, 33, 129],
    [118, 33, 129], [120, 34, 129], [121, 34, 130], [123, 35, 130], [124, 35, 130], [126, 36, 130], [128, 37, 130], [129, 37, 129],
    [131, 38, 129], [132, 38, 129], [134, 39, 129], [136, 39, 129], [137, 40, 129], [139, 41, 129], [140, 41, 129], [142, 42, 129],
    [144, 42, 129], [145, 43, 129], [147, 43, 128], [148, 44, 128], [150, 44, 128], [152, 45, 128], [153, 45, 128], [155, 46, 127],
    [156, 46, 127], [158, 47, 127], [160, 47, 127], [161, 48, 126], [163, 48, 126], [165, 49, 126], [166, 49, 125], [168, 50, 125],
    [170, 51, 125], [171, 51, 124], [173, 52, 124], [174, 52, 123], [176, 53, 123], [178, 53, 123], [179, 54, 122], [181, 54, 122],
    [183, 55, 121], [184, 55, 121], [186, 56, 120], [188, 57, 120], [189, 57, 119], [191, 58, 119], [192, 58, 118], [194, 59, 117],
    [196, 60, 117], [197, 60, 116], [199, 61, 115], [200, 62, 115], [202, 62, 114], [204, 63, 113], [205, 64, 113], [207, 64, 112],
    [208, 65, 111], [210, 66, 111], [211, 67, 110], [213, 68, 109], [214, 69, 108], [216, 69, 108], [217, 70, 107], [219, 71, 106],
    [220, 72, 105], [222, 73, 104], [223, 74, 104], [224, 76, 103], [226, 77, 102], [227, 78, 101], [228, 79, 100], [229, 80, 100],
    [231, 82, 99], [232, 83, 98], [233, 84, 98], [234, 86, 97], [235, 87, 96], [236, 88, 96], [237, 90, 95], [238, 91, 94],
    [239, 93, 94], [240, 95, 94], [241, 96, 93], [242, 98, 93], [242, 100, 92], [243, 101, 92], [244, 103, 92], [244, 105, 92],
    [245, 107, 92], [246, 108, 92], [246, 110, 92], [247, 112, 92], [247, 114, 92], [248, 116, 92], [248, 118, 92], [249, 120, 93],
    [249, 121, 93], [249, 123, 93], [250, 125, 94], [250, 127, 94], [250, 129, 95], [251, 131, 95], [251, 133, 96], [251, 135, 97],
    [252, 137, 97], [252, 138, 98], [252, 140, 99], [252, 142, 100], [252, 144, 101], [253, 146, 102], [253, 148, 103], [253, 150, 104],
    [253, 152, 105], [253, 154, 106], [253, 155, 107], [254, 157, 108], [254, 159, 109], [254, 161, 110], [254, 163, 111], [254, 165, 113],
    [254, 167, 114], [254, 169, 115], [254, 170, 116], [254, 172, 118], [254, 174, 119], [254, 176, 120], [254, 178, 122], [254, 180, 123],
    [254, 182, 124], [254, 183, 126], [254, 185, 127], [254, 187, 129], [254, 189, 130], [254, 191, 132], [254, 193, 133], [254, 194, 135],
    [254, 196, 136], [254, 198, 138], [254, 200, 140], [254, 202, 141], [254, 204, 143], [254, 205, 144], [254, 207, 146], [254, 209, 148],
    [254, 211, 149], [254, 213, 151], [254, 215, 153], [254, 216, 154], [253, 218, 156], [253, 220, 158], [253, 222, 160], [253, 224, 161],
    [253, 226, 163], [253, 227, 165], [253, 229, 167], [253, 231, 169], [253, 233, 170], [253, 235, 172], [252, 236, 174], [252, 238, 176],
    [252, 240, 178], [252, 242, 180], [252, 244, 182], [252, 246, 184], [252, 247, 185], [252, 249, 187], [252, 251, 189], [252, 253, 191],
];

#[cfg(test)]
mod tests {
    use super::*;

    struct Grid {
        cols: usize,
        rows: usize,
        db: Vec<f32>,
    }
    impl DbGrid for Grid {
        fn columns(&self) -> usize {
            self.cols
        }
        fn rows(&self) -> usize {
            self.rows
        }
        fn db_at(&self, c: usize, r: usize) -> f32 {
            self.db[c * self.rows + r]
        }
    }

    fn luma(c: [u8; 3]) -> f32 {
        0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2])
    }

    #[test]
    fn extremos() {
        assert_eq!(Palette::Magma.color(0.0), [0, 0, 4]);
        assert_eq!(Palette::Magma.color(1.0), [252, 253, 191]);
        assert_eq!(Palette::Gray.color(0.0), [0, 0, 0]);
        assert_eq!(Palette::Gray.color(1.0), [255, 255, 255]);
        assert_eq!(
            Palette::Spectrum.color(0.0),
            [0, 0, 0],
            "Spectrum empieza en negro"
        );
        assert_eq!(
            Palette::Spectrum.color(1.0),
            [255, 0, 0],
            "y termina en rojo"
        );
        assert_eq!(Palette::Sox.color(0.0), [0, 0, 0]);
        assert_eq!(Palette::Sox.color(1.0), [255, 255, 255]);
        // Fuera de rango y NaN se recortan.
        for p in Palette::ALL {
            assert_eq!(p.color(-3.0), p.color(0.0));
            assert_eq!(p.color(7.0), p.color(1.0));
            assert_eq!(p.color(f32::NAN), p.color(0.0));
        }
    }

    #[test]
    fn luminancia_monotona() {
        for p in [Palette::Magma, Palette::Gray] {
            let mut prev = -1.0;
            for i in 0..=200 {
                let l = luma(p.color(i as f32 / 200.0));
                assert!(l >= prev - 0.5, "{p:?} no es monótona en {i}");
                prev = l;
            }
        }
    }

    #[test]
    fn render_orientacion_y_colores() {
        // 2 columnas × 3 filas. Fila 0 = 0 Hz (abajo en la imagen).
        let g = Grid {
            cols: 2,
            rows: 3,
            db: vec![-120.0, -60.0, 0.0, 0.0, 0.0, -200.0],
        };
        let img = render_rgba(&g, Palette::Gray, -120.0, 0.0);
        assert_eq!((img.width, img.height, img.data.len()), (2, 3, 24));
        assert_eq!(
            img.pixel(0, 2),
            [0, 0, 0, 255],
            "columna 0, 0 Hz = −120 dB → negro, abajo"
        );
        assert_eq!(
            img.pixel(0, 0),
            [255, 255, 255, 255],
            "columna 0, Nyquist = 0 dB → blanco, arriba"
        );
        let mid = img.pixel(0, 1)[0];
        assert!((126..=129).contains(&mid), "−60 dB → gris medio ({mid})");
        assert_eq!(
            img.pixel(1, 0),
            [0, 0, 0, 255],
            "−200 dB recortado al mínimo"
        );
        // Rango invertido: mismo resultado.
        assert_eq!(render_rgba(&g, Palette::Gray, 0.0, -120.0), img);
        // Rango nulo: no entra en pánico.
        let _ = render_rgba(&g, Palette::Magma, -10.0, -10.0);
    }

    /// Render a tamaño máximo (4.096 × 2.048). Objetivo: < 150 ms en release (F1-T6).
    /// `cargo test --release -p spector-core --lib render_rendimiento -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn render_rendimiento() {
        let (cols, rows) = (4096, 2048);
        let g = Grid {
            cols,
            rows,
            db: (0..cols * rows).map(|i| -((i % 140) as f32)).collect(),
        };
        let start = std::time::Instant::now();
        let img = render_rgba(&g, Palette::Magma, -120.0, 0.0);
        let elapsed = start.elapsed();
        println!("render 4096×2048: {elapsed:?}");
        assert_eq!(img.data.len(), cols * rows * 4);
        assert!(elapsed.as_millis() < 150);
    }

    #[test]
    fn barra_de_paleta() {
        let s = palette_strip(Palette::Gray, 256);
        assert_eq!((s.width, s.height), (1, 256));
        assert_eq!(s.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(s.pixel(0, 255), [0, 0, 0, 255]);
        assert_eq!(palette_strip(Palette::Magma, 1).height, 1);
        assert_eq!(palette_strip(Palette::Magma, 0).data.len(), 0);
    }
}
