//! Widget del espectrograma: imagen escalada en la GPU, ejes de tiempo y frecuencia, leyenda de
//! dB, rejilla tenue, lectura bajo el cursor y ficha del archivo.
//!
//! Se compone con widgets estándar (ver `docs/decisions/0001-widget-espectrograma-compuesto.md`):
//!
//! ```text
//! ┌───────────── ficha (nombre + línea de información) ─────────────┐
//! │ eje Hz │ Overlay[ Picture(textura) + rejilla + lectura ] │ leyenda dB │
//! │        │ eje de tiempo                                    │            │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Los ejes comparten fila o columna con la imagen, así que su longitud coincide con la suya.
//! Las conversiones píxel ↔ (tiempo, Hz, dB) son funciones puras (al final del archivo).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk4 as gtk;

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};

use crate::palette::{Palette, RgbaImage};
use crate::spectrogram::{DbGrid, SpectrogramData};
use crate::ticks::{db_ticks, fmt_db, fmt_khz_1, fmt_time_tenths, freq_ticks, time_ticks};

use crate::gtk::strings;

/// Ancho del eje de frecuencia.
const FREQ_AXIS_W: i32 = 68;
/// Alto del eje de tiempo.
const TIME_AXIS_H: i32 = 30;
/// Ancho de la leyenda de dB.
const LEGEND_W: i32 = 76;
/// Separación mínima entre marcas (px).
const MIN_SPACING_FREQ: f64 = 28.0;
const MIN_SPACING_TIME: f64 = 72.0;
const MIN_SPACING_DB: f64 = 26.0;
/// Longitud de las marcas.
const TICK_LEN: f64 = 5.0;
/// Opacidad del texto y de las marcas.
const TEXT_ALPHA: f64 = 0.7;
const TICK_ALPHA: f64 = 0.35;
const GRID_ALPHA: f64 = 0.08;
/// Color de la línea del corte (ámbar) y su opacidad.
const CUTOFF_RGB: (f64, f64, f64) = (0.97, 0.78, 0.30);
const CUTOFF_ALPHA: f64 = 0.75;

/// Datos que necesitan los dibujos (compartidos por los ejes, la leyenda y la lectura).
#[derive(Default)]
struct ViewModel {
    data: Option<Arc<SpectrogramData>>,
    palette: Palette,
    db_min: f32,
    db_max: f32,
    /// Corte detectado por el análisis (línea discontinua sobre la imagen).
    cutoff_hz: Option<f64>,
    show_cutoff: bool,
}

/// Widget del espectrograma.
pub struct SpectrogramView {
    /// Contenedor raíz.
    pub root: gtk::Box,
    title: gtk::Label,
    info: gtk::Label,
    header: gtk::Box,
    picture: gtk::Picture,
    areas: [gtk::DrawingArea; 4],
    model: Rc<RefCell<ViewModel>>,
}

impl Default for SpectrogramView {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectrogramView {
    /// Crea el widget vacío.
    pub fn new() -> Self {
        let model = Rc::new(RefCell::new(ViewModel {
            db_max: 0.0,
            db_min: -120.0,
            show_cutoff: true,
            ..Default::default()
        }));

        // --- Ficha ---
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let info = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .selectable(true)
            .css_classes(["dim-label", "caption"])
            .build();
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_start(FREQ_AXIS_W)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(8)
            .build();
        header.append(&title);
        header.append(&info);

        // --- Imagen con rejilla y lectura bajo el cursor ---
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let grid_lines = area(&model, draw_grid_lines);
        grid_lines.set_can_target(false);
        let readout = gtk::Label::builder()
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(8)
            .margin_end(8)
            .css_classes(["osd", "caption", "numeric"])
            .visible(false)
            .can_target(false)
            .build();
        readout.set_margin_start(6);
        let overlay = gtk::Overlay::builder().child(&picture).build();
        overlay.add_overlay(&grid_lines);
        overlay.add_overlay(&readout);
        install_readout(&overlay, &readout, &model);

        // --- Ejes y leyenda ---
        let freq_axis = area(&model, draw_freq_axis);
        freq_axis.set_content_width(FREQ_AXIS_W);
        let time_axis = area(&model, draw_time_axis);
        time_axis.set_content_height(TIME_AXIS_H);
        let legend = area(&model, draw_legend);
        legend.set_content_width(LEGEND_W);

        let grid = gtk::Grid::builder().margin_end(4).margin_bottom(4).build();
        grid.attach(&freq_axis, 0, 0, 1, 1);
        grid.attach(&overlay, 1, 0, 1, 1);
        grid.attach(&legend, 2, 0, 1, 1);
        grid.attach(&time_axis, 1, 1, 1, 1);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&header);
        root.append(&grid);

        Self {
            root,
            title,
            info,
            header,
            picture,
            areas: [freq_axis, time_axis, legend, grid_lines],
            model,
        }
    }

    /// Añade un widget debajo de la ficha (p. ej. el panel del veredicto).
    pub fn append_to_header(&self, widget: &impl IsA<gtk::Widget>) {
        self.header.append(widget);
    }

    /// Fija el corte detectado (`None` lo quita).
    pub fn set_cutoff(&self, hz: Option<f64>) {
        self.model.borrow_mut().cutoff_hz = hz;
        self.areas[3].queue_draw();
    }

    /// Muestra u oculta la línea del corte.
    pub fn set_cutoff_visible(&self, visible: bool) {
        self.model.borrow_mut().show_cutoff = visible;
        self.areas[3].queue_draw();
    }

    /// Muestra el nombre y la línea de información del archivo.
    pub fn set_file(&self, name: &str, info_line: &str) {
        self.title.set_label(name);
        self.info.set_label(info_line);
    }

    /// Muestra un espectrograma nuevo.
    pub fn set_spectrogram(
        &self,
        data: Arc<SpectrogramData>,
        rgba: RgbaImage,
        palette: Palette,
        db_min: f32,
        db_max: f32,
    ) {
        self.model.borrow_mut().data = Some(data);
        self.set_image(rgba, palette, db_min, db_max);
    }

    /// Sustituye la imagen (re-render con otra paleta o rango) conservando los datos.
    pub fn set_image(&self, rgba: RgbaImage, palette: Palette, db_min: f32, db_max: f32) {
        {
            let mut m = self.model.borrow_mut();
            m.palette = palette;
            m.db_min = db_min;
            m.db_max = db_max;
        }
        self.picture.set_paintable(texture_from_rgba(rgba).as_ref());
        for a in &self.areas {
            a.queue_draw();
        }
    }
}

/// Tamaño del gráfico al exportar a PNG.
pub const EXPORT_PLOT: (usize, usize) = (1600, 800);
/// Alto de la cabecera (nombre e información) al exportar.
const EXPORT_HEADER_H: usize = 52;
/// Margen exterior al exportar.
const EXPORT_PAD: usize = 12;

impl SpectrogramView {
    /// Dibuja la capa de ejes, textos y rejilla para exportar a PNG (en el hilo principal: es
    /// rápido) y devuelve el trabajo para el trabajador. `None` si no hay espectrograma.
    pub fn export_job(&self) -> Option<crate::gtk::export::ExportJob> {
        let m = self.model.borrow();
        let data = Arc::clone(m.data.as_ref()?);
        let (pw, ph) = EXPORT_PLOT;
        let fw = FREQ_AXIS_W as usize;
        let plot = crate::gtk::export::Rect {
            x: EXPORT_PAD + fw,
            y: EXPORT_PAD + EXPORT_HEADER_H,
            w: pw,
            h: ph,
        };
        let width = plot.x + pw + LEGEND_W as usize + EXPORT_PAD;
        let height = plot.y + ph + TIME_AXIS_H as usize + EXPORT_PAD;
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, width as i32, height as i32).ok()?;
        {
            let cr = cairo::Context::new(&surface).ok()?;
            cr.set_source_rgb(0.118, 0.118, 0.118);
            let _ = cr.paint();
            cr.set_operator(cairo::Operator::Clear);
            cr.rectangle(plot.x as f64, plot.y as f64, pw as f64, ph as f64);
            let _ = cr.fill();
            cr.set_operator(cairo::Operator::Over);
            let pango = pangocairo::functions::create_context(&cr);
            pango.set_font_description(&gtk::pango::FontDescription::from_string("Sans 11"));
            // Cabecera: nombre en negrita e información.
            let header = gtk::pango::Layout::new(&pango);
            header.set_markup(&format!(
                "<b>{}</b>\n<small>{}</small>",
                glib::markup_escape_text(&self.title.label()),
                glib::markup_escape_text(&self.info.label())
            ));
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.move_to(plot.x as f64, EXPORT_PAD as f64);
            pangocairo::functions::show_layout(&cr, &header);
            let at = |x: usize, y: usize, w: usize, h: usize, f: fn(&Canvas<'_>, &ViewModel)| {
                let _ = cr.save();
                cr.translate(x as f64, y as f64);
                let c = Canvas {
                    pango: pango.clone(),
                    cr: &cr,
                    w: w as f64,
                    h: h as f64,
                };
                f(&c, &m);
                let _ = cr.restore();
            };
            at(EXPORT_PAD, plot.y, fw, ph, draw_freq_axis);
            at(
                plot.x,
                plot.y + ph,
                pw,
                TIME_AXIS_H as usize,
                draw_time_axis,
            );
            at(plot.x + pw, plot.y, LEGEND_W as usize, ph, draw_legend);
            at(plot.x, plot.y, pw, ph, draw_grid_lines);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let overlay = surface.data().ok()?.to_vec();
        Some(crate::gtk::export::ExportJob {
            overlay,
            stride,
            width,
            height,
            plot,
            data,
            palette: m.palette,
            db_min: m.db_min,
            db_max: m.db_max,
        })
    }

    /// `true` si hay un espectrograma que exportar.
    pub fn has_data(&self) -> bool {
        self.model.borrow().data.is_some()
    }
}

fn area(model: &Rc<RefCell<ViewModel>>, draw: fn(&Canvas<'_>, &ViewModel)) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    let model = Rc::clone(model);
    a.set_draw_func(move |a, cr, w, h| {
        let c = Canvas {
            pango: a.pango_context(),
            cr,
            w: f64::from(w),
            h: f64::from(h),
        };
        draw(&c, &model.borrow());
    });
    a
}

fn install_readout(overlay: &gtk::Overlay, readout: &gtk::Label, model: &Rc<RefCell<ViewModel>>) {
    let motion = gtk::EventControllerMotion::new();
    let (r, m) = (readout.clone(), Rc::clone(model));
    motion.connect_motion(move |ctl, x, y| {
        let Some(w) = ctl.widget() else { return };
        let text = m
            .borrow()
            .data
            .as_ref()
            .and_then(|d| readout_at(d, x, y, f64::from(w.width()), f64::from(w.height())));
        match text {
            Some(t) => {
                r.set_label(&t);
                r.set_visible(true);
            }
            None => r.set_visible(false),
        }
    });
    let r = readout.clone();
    motion.connect_leave(move |_| r.set_visible(false));
    overlay.add_controller(motion);
}

// ---------------------------------------------------------------------------------------------
// Dibujo (cairo + Pango)
// ---------------------------------------------------------------------------------------------

/// Superficie de dibujo de un eje: el contexto Pango (la fuente), el de cairo y su tamaño. Sirve
/// tanto para el widget como para exportar a PNG (ver [`draw_export_overlay`]).
pub struct Canvas<'a> {
    /// Contexto Pango (fuente del tema en el widget; fija al exportar).
    pub pango: gtk::pango::Context,
    /// Contexto de cairo.
    pub cr: &'a cairo::Context,
    /// Ancho.
    pub w: f64,
    /// Alto.
    pub h: f64,
}

/// Pinta `text` con la fuente del tema (tamaño pequeño). `(ax, ay)` es el ancla dentro del texto
/// (0 = izquierda/arriba, 0,5 = centro, 1 = derecha/abajo). Se recorta a la superficie.
fn label(c: &Canvas<'_>, text: &str, x: f64, y: f64, ax: f64, ay: f64) {
    let layout = gtk::pango::Layout::new(&c.pango);
    layout.set_markup(&format!(
        "<small>{}</small>",
        glib::markup_escape_text(text)
    ));
    let (tw, th) = layout.pixel_size();
    let (tw, th) = (f64::from(tw), f64::from(th));
    let lx = (x - ax * tw).clamp(0.0, (c.w - tw).max(0.0));
    let ly = (y - ay * th).clamp(0.0, (c.h - th).max(0.0));
    c.cr.set_source_rgba(1.0, 1.0, 1.0, TEXT_ALPHA);
    c.cr.move_to(lx, ly);
    pangocairo::functions::show_layout(c.cr, &layout);
}

fn line(cr: &cairo::Context, x0: f64, y0: f64, x1: f64, y1: f64, alpha: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    cr.set_line_width(1.0);
    cr.move_to(x0, y0);
    cr.line_to(x1, y1);
    // Un error de cairo al dibujar no es recuperable ni grave: se ignora.
    let _ = cr.stroke();
}

/// Línea horizontal nítida (centrada en medio píxel).
fn crisp(v: f64) -> f64 {
    v.floor() + 0.5
}

fn draw_freq_axis(c: &Canvas<'_>, m: &ViewModel) {
    let (cr, w, h) = (c.cr, c.w, c.h);
    let Some(d) = &m.data else { return };
    for t in freq_ticks(d.nyquist_hz(), h, MIN_SPACING_FREQ) {
        let y = crisp((h - hz_to_y(t.value, h, d)).min(h - 0.5));
        line(cr, w - TICK_LEN, y, w, y, TICK_ALPHA);
        label(c, &t.label, w - TICK_LEN - 4.0, y, 1.0, 0.5);
    }
}

fn draw_time_axis(c: &Canvas<'_>, m: &ViewModel) {
    let (cr, w) = (c.cr, c.w);
    let Some(d) = &m.data else { return };
    let ticks = time_ticks(d.duration_s, w, MIN_SPACING_TIME);
    if ticks.is_empty() {
        label(
            c,
            strings::UNKNOWN_DURATION,
            w / 2.0,
            TICK_LEN + 2.0,
            0.5,
            0.0,
        );
        return;
    }
    for t in ticks {
        let x = crisp(t.pos_px.min(w - 0.5));
        line(cr, x, 0.0, x, TICK_LEN, TICK_ALPHA);
        label(c, &t.label, x, TICK_LEN + 2.0, 0.5, 0.0);
    }
}

fn draw_legend(c: &Canvas<'_>, m: &ViewModel) {
    let (cr, h) = (c.cr, c.h);
    if m.data.is_none() {
        return;
    }
    let (x0, bar_w) = (10.0, 12.0);
    let gradient = cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
    const STOPS: usize = 64;
    for i in 0..=STOPS {
        let t = i as f64 / STOPS as f64;
        let c = m.palette.color(1.0 - t as f32);
        gradient.add_color_stop_rgb(
            t,
            f64::from(c[0]) / 255.0,
            f64::from(c[1]) / 255.0,
            f64::from(c[2]) / 255.0,
        );
    }
    let _ = cr.set_source(&gradient);
    cr.rectangle(x0, 0.0, bar_w, h);
    let _ = cr.fill();
    for t in db_ticks(f64::from(m.db_min), f64::from(m.db_max), h, MIN_SPACING_DB) {
        let y = crisp((h - t.pos_px).min(h - 0.5));
        line(cr, x0 + bar_w, y, x0 + bar_w + TICK_LEN, y, TICK_ALPHA);
        label(c, &t.label, x0 + bar_w + TICK_LEN + 4.0, y, 0.0, 0.5);
    }
}

fn draw_grid_lines(c: &Canvas<'_>, m: &ViewModel) {
    let (cr, w, h) = (c.cr, c.w, c.h);
    let Some(d) = &m.data else { return };
    let ticks = freq_ticks(d.nyquist_hz(), h, MIN_SPACING_FREQ);
    let n = ticks.len();
    for t in ticks.into_iter().skip(1).take(n.saturating_sub(2)) {
        let y = crisp(h - hz_to_y(t.value, h, d));
        line(cr, 0.0, y, w, y, GRID_ALPHA);
    }
    // Línea discontinua del corte detectado (ámbar, discreta).
    if m.show_cutoff
        && let Some(hz) = m.cutoff_hz
        && hz > 0.0
        && hz < d.nyquist_hz()
    {
        let y = crisp(h - hz_to_y(hz, h, d));
        cr.set_source_rgba(CUTOFF_RGB.0, CUTOFF_RGB.1, CUTOFF_RGB.2, CUTOFF_ALPHA);
        cr.set_line_width(1.0);
        cr.set_dash(&[6.0, 4.0], 0.0);
        cr.move_to(0.0, y);
        cr.line_to(w, y);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
}

/// Convierte la imagen RGBA del núcleo en una textura de GDK (sin copiar los píxeles).
pub fn texture_from_rgba(img: RgbaImage) -> Option<gdk::MemoryTexture> {
    let (w, h) = (
        i32::try_from(img.width).ok()?,
        i32::try_from(img.height).ok()?,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let stride = img.width.checked_mul(4)?;
    let bytes = glib::Bytes::from_owned(img.data);
    Some(gdk::MemoryTexture::new(
        w,
        h,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        stride,
    ))
}

// ---------------------------------------------------------------------------------------------
// Geometría (funciones puras)
// ---------------------------------------------------------------------------------------------

/// Hz que cubre la imagen de abajo a arriba: `filas · Hz por fila` (≈ Nyquist; algo más si las
/// filas agrupan bins).
pub fn image_span_hz(d: &SpectrogramData) -> f64 {
    d.rows as f64 * d.hz_per_row
}

/// Distancia en píxeles **desde abajo** a la que se dibuja `hz` en una imagen de alto `h`.
pub fn hz_to_y(hz: f64, h: f64, d: &SpectrogramData) -> f64 {
    let span = image_span_hz(d);
    if span <= 0.0 { 0.0 } else { hz / span * h }
}

/// Celda (columna, fila de datos) bajo el punto `(x, y)` de una imagen de `w × h` px
/// (`y = 0` arriba). `None` fuera de la imagen o si está vacía.
pub fn cell_at(d: &SpectrogramData, x: f64, y: f64, w: f64, h: f64) -> Option<(usize, usize)> {
    if d.columns == 0 || d.rows == 0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
        return None;
    }
    let col = ((x / w * d.columns as f64) as usize).min(d.columns - 1);
    let row = (((h - y) / h * d.rows as f64) as usize).min(d.rows - 1);
    Some((col, row))
}

/// Texto de la lectura bajo el cursor: "1:23,4 · 16,2 kHz · −72 dB".
pub fn readout_at(d: &SpectrogramData, x: f64, y: f64, w: f64, h: f64) -> Option<String> {
    let (col, row) = cell_at(d, x, y, w, h)?;
    let secs = x / w * d.duration_s;
    let hz = ((h - y) / h * image_span_hz(d)).min(d.nyquist_hz());
    let db = d.db_at(col, row);
    Some(format!(
        "{} · {} · {}",
        fmt_time_tenths(secs),
        fmt_khz_1(hz),
        fmt_db(f64::from(db))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(columns: usize, rows: usize, rate: u32, duration_s: f64) -> SpectrogramData {
        SpectrogramData {
            columns,
            rows,
            sample_rate: rate,
            duration_s,
            hz_per_row: f64::from(rate) / 2.0 / rows as f64,
            seconds_per_column: duration_s / columns as f64,
            db: (0..columns * rows).map(|i| -(i as f32)).collect(),
        }
    }

    #[test]
    fn celda_bajo_el_cursor() {
        let d = data(100, 50, 48_000, 10.0);
        assert_eq!(
            cell_at(&d, 0.0, 999.0, 1000.0, 1000.0),
            Some((0, 0)),
            "abajo a la izquierda"
        );
        assert_eq!(
            cell_at(&d, 999.9, 0.0, 1000.0, 1000.0),
            Some((99, 49)),
            "arriba a la derecha"
        );
        assert_eq!(cell_at(&d, 500.0, 500.0, 1000.0, 1000.0), Some((50, 25)));
        assert_eq!(cell_at(&d, -1.0, 10.0, 1000.0, 1000.0), None);
        assert_eq!(cell_at(&d, 10.0, 1000.0, 1000.0, 1000.0), None);
        assert_eq!(cell_at(&d, 10.0, 10.0, 0.0, 1000.0), None);
    }

    #[test]
    fn lectura() {
        let d = data(100, 48, 48_000, 83.47 * 2.0);
        // Centro: 1:23,4, 12 kHz.
        let t = readout_at(&d, 500.0, 500.0, 1000.0, 1000.0).unwrap();
        assert!(t.starts_with("1:23,4 · 12,0 kHz · "), "{t}");
        assert!(t.ends_with(" dB"));
    }

    #[test]
    fn frecuencia_a_pixel() {
        let d = data(10, 1000, 44_100, 1.0);
        let y = hz_to_y(16_000.0, 600.0, &d);
        assert!((y - 16_000.0 / 22_050.0 * 600.0).abs() < 1e-9);
        assert_eq!(fmt_khz_1(16_234.0), "16,2 kHz");
    }

    #[test]
    fn textura_vacia() {
        assert!(
            texture_from_rgba(RgbaImage {
                width: 0,
                height: 3,
                data: vec![]
            })
            .is_none()
        );
    }
}
