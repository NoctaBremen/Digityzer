//! Ventana principal (T2.1): dispositivo y par, armar, grabar y parar, cara A/B, tiempo y
//! espacio libre, con los medidores en vivo (T2.2–T2.5), el selector de vista del espectro
//! (T2.8) y el de temas (T2.9e).
//!
//! La sesión de audio vive en el hilo principal pero no hace nada pesado en él: captura,
//! escritura y análisis van en sus hilos. Cerrar un master (fsync) se hace en un hilo aparte.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use audio_core::theme::{THEMES, Theme};
use digityzer_audio::probe::{CaptureCaps, khz, probe};
use digityzer_audio::{
    AlsaRequest, AlsaSource, AudioError, CaptureSource, EngineConfig, RecordingResult,
    SampleFormat, Session, StreamConfig, SyntheticSource,
};
use digityzer_dsp::LiveSnapshot;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gio, glib};

use super::config::Config;
use super::draw::{self, Waterfall};
use super::logic::{Side, bytes_human, clock, master_path, minutes_left};
use super::spectrum_view::{SMOOTHING, SpectrumMode, SpectrumView};

/// Frecuencias que se ofrecen (si la tarjeta las admite).
const RATES: [u32; 3] = [44_100, 48_000, 96_000];
/// Canales del origen sintético.
const SYNTH_CHANNELS: u16 = 12;

/// Estilo propio de la ventana (solo forma; los colores vienen del tema).
pub const CSS: &str = "
.rec-time { font-size: 30px; font-weight: bold; font-feature-settings: \"tnum\"; }
.meter-frame { border-radius: 6px; }
";

/// CSS completo para un tema: colores del tema + forma de la ventana.
pub fn css_for(theme: &Theme) -> String {
    format!("{}{CSS}", theme.gtk_css())
}

#[derive(Debug, Clone)]
struct DeviceChoice {
    label: String,
    /// `None` = origen sintético.
    hw: Option<String>,
    channels: u16,
    rates: Vec<u32>,
}

fn device_choices() -> Vec<DeviceChoice> {
    let mut v = Vec::new();
    for card in probe() {
        for d in card.devices {
            let label = format!("{} — {} ({})", card.name, d.name, d.hw);
            match d.caps {
                CaptureCaps::Available {
                    channels_max,
                    rates,
                    ..
                } => v.push(DeviceChoice {
                    label,
                    hw: Some(d.hw),
                    channels: u16::try_from(channels_max).unwrap_or(2),
                    rates: rates.into_iter().filter(|r| RATES.contains(r)).collect(),
                }),
                // Ocupado o ilegible: se ofrece igual; al armar se dirá por qué no se puede.
                CaptureCaps::Busy | CaptureCaps::Error(_) => v.push(DeviceChoice {
                    label: format!("{label} — ocupado"),
                    hw: Some(d.hw),
                    channels: 12,
                    rates: RATES.to_vec(),
                }),
            }
        }
    }
    v.push(DeviceChoice {
        label: "Origen sintético (prueba sin la mesa)".into(),
        hw: None,
        channels: SYNTH_CHANNELS,
        rates: RATES.to_vec(),
    });
    v
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn pairs(channels: u16) -> Vec<[u16; 2]> {
    (1..channels).step_by(2).map(|c| [c, c + 1]).collect()
}

struct App {
    window: gtk::ApplicationWindow,
    devices: RefCell<Vec<DeviceChoice>>,
    device_dd: gtk::DropDown,
    refresh_btn: gtk::Button,
    pair_dd: gtk::DropDown,
    rate_dd: gtk::DropDown,
    side_a: gtk::ToggleButton,
    side_b: gtk::ToggleButton,
    folder: RefCell<PathBuf>,
    folder_btn: gtk::Button,
    disc_entry: gtk::Entry,
    notes_entry: gtk::Entry,
    arm_btn: gtk::Button,
    rec_btn: gtk::Button,
    stop_btn: gtk::Button,
    time_lbl: gtk::Label,
    free_lbl: gtk::Label,
    status_lbl: gtk::Label,
    areas: [gtk::DrawingArea; 4],
    session: RefCell<Option<Session>>,
    snap: Rc<RefCell<LiveSnapshot>>,
    clip_base: Cell<[u64; 2]>,
    clipped: Rc<Cell<[bool; 2]>>,
    recording_view: Rc<Cell<bool>>,
    closing: Cell<bool>,
    last_labels: Cell<Option<Instant>>,
    theme: Rc<Cell<Theme>>,
    css: gtk::CssProvider,
    config: RefCell<Config>,
    theme_dd: gtk::DropDown,
    view_dd: gtk::DropDown,
    smooth_dd: gtk::DropDown,
    spec: Rc<RefCell<SpectrumView>>,
    waterfall: Rc<RefCell<Waterfall>>,
    last_frame_us: Cell<Option<i64>>,
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build()
}

/// Construye y muestra la ventana.
pub fn build(app: &gtk::Application) {
    let config = Config::load();
    let theme = Theme::by_id(&config.theme).effective();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css_for(&theme));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Digityzer")
        .default_width(1100)
        .default_height(720)
        .build();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    let music = [home.join("Música"), home.join("Music")]
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or(home);

    let area = |w: i32, h: i32| {
        gtk::DrawingArea::builder()
            .content_width(w)
            .content_height(h)
            .css_classes(["meter-frame"])
            .build()
    };
    let side_a = gtk::ToggleButton::with_label("Cara A");
    let side_b = gtk::ToggleButton::with_label("Cara B");
    side_b.set_group(Some(&side_a));
    side_a.set_active(true);

    let app_state = Rc::new(App {
        window: window.clone(),
        devices: RefCell::new(Vec::new()),
        device_dd: gtk::DropDown::from_strings(&[]),
        refresh_btn: gtk::Button::from_icon_name("view-refresh-symbolic"),
        pair_dd: gtk::DropDown::from_strings(&[]),
        rate_dd: gtk::DropDown::from_strings(&[]),
        side_a,
        side_b,
        folder: RefCell::new(music),
        folder_btn: gtk::Button::new(),
        disc_entry: gtk::Entry::builder()
            .placeholder_text("Artista - Título")
            .hexpand(true)
            .build(),
        notes_entry: gtk::Entry::builder()
            .placeholder_text("Sello, catálogo, estado del disco…")
            .hexpand(true)
            .build(),
        arm_btn: gtk::Button::with_label("Armar"),
        rec_btn: gtk::Button::builder()
            .label("● Grabar")
            .css_classes(["destructive-action"])
            .sensitive(false)
            .build(),
        stop_btn: gtk::Button::builder()
            .label("■ Parar")
            .sensitive(false)
            .build(),
        time_lbl: gtk::Label::builder()
            .label("00:00:00")
            .css_classes(["rec-time"])
            .build(),
        free_lbl: gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["numeric"])
            .build(),
        status_lbl: gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["dim-label"])
            .label("Elige dispositivo y par, y arma para ver los niveles.")
            .build(),
        areas: [
            area(130, 300),
            area(500, 300),
            area(240, 300),
            area(800, 110),
        ],
        session: RefCell::new(None),
        snap: Rc::new(RefCell::new(LiveSnapshot::default())),
        clip_base: Cell::new([0, 0]),
        clipped: Rc::new(Cell::new([false, false])),
        recording_view: Rc::new(Cell::new(false)),
        closing: Cell::new(false),
        last_labels: Cell::new(None),
        theme: Rc::new(Cell::new(theme)),
        css: provider,
        theme_dd: gtk::DropDown::from_strings(&THEMES.iter().map(|t| t.name).collect::<Vec<_>>()),
        view_dd: gtk::DropDown::from_strings(&SpectrumMode::ALL.map(SpectrumMode::label)),
        smooth_dd: gtk::DropDown::from_strings(&["1/3 octava", "1/6 octava", "1/12 octava"]),
        spec: Rc::new(RefCell::new(SpectrumView::new(
            SpectrumMode::from_id(&config.spectrum_view),
            if SMOOTHING.contains(&config.smoothing) {
                config.smoothing
            } else {
                6
            },
        ))),
        waterfall: Rc::new(RefCell::new(Waterfall::new())),
        last_frame_us: Cell::new(None),
        config: RefCell::new(config),
    });
    let a = &app_state;
    a.device_dd.set_hexpand(true);
    a.refresh_btn
        .set_tooltip_text(Some("Volver a buscar tarjetas"));
    a.areas[1].set_hexpand(true);
    a.areas[3].set_hexpand(true);
    a.update_folder_label();
    {
        let cfg = a.config.borrow();
        let ti = THEMES.iter().position(|t| t.id == cfg.theme).unwrap_or(0);
        a.theme_dd.set_selected(ti as u32);
        let sp = a.spec.borrow();
        let vi = SpectrumMode::ALL
            .iter()
            .position(|&m| m == sp.mode)
            .unwrap_or(0);
        a.view_dd.set_selected(vi as u32);
        let si = SMOOTHING
            .iter()
            .position(|&f| f == sp.fraction)
            .unwrap_or(1);
        a.smooth_dd.set_selected(si as u32);
        a.smooth_dd.set_sensitive(sp.mode == SpectrumMode::Curve);
    }

    // Dibujo.
    {
        let (snap, clipped, th) = (
            Rc::clone(&a.snap),
            Rc::clone(&a.clipped),
            Rc::clone(&a.theme),
        );
        a.areas[0].set_draw_func(move |_, cr, w, h| {
            let t = th.get();
            draw::meters(
                cr,
                &t,
                f64::from(w),
                f64::from(h),
                &snap.borrow(),
                clipped.get(),
            );
        });
        let (snap, th) = (Rc::clone(&a.snap), Rc::clone(&a.theme));
        let (spec, wf) = (Rc::clone(&a.spec), Rc::clone(&a.waterfall));
        a.areas[1].set_draw_func(move |_, cr, w, h| {
            let t = th.get();
            draw::spectrum(
                cr,
                &t,
                f64::from(w),
                f64::from(h),
                &mut spec.borrow_mut(),
                &snap.borrow(),
                &mut wf.borrow_mut(),
            );
        });
        let (snap, th) = (Rc::clone(&a.snap), Rc::clone(&a.theme));
        a.areas[2].set_draw_func(move |_, cr, w, h| {
            let t = th.get();
            draw::goniometer(cr, &t, f64::from(w), f64::from(h), &snap.borrow());
        });
        let (snap, rec, th) = (
            Rc::clone(&a.snap),
            Rc::clone(&a.recording_view),
            Rc::clone(&a.theme),
        );
        a.areas[3].set_draw_func(move |_, cr, w, h| {
            let t = th.get();
            draw::waveform(
                cr,
                &t,
                f64::from(w),
                f64::from(h),
                &snap.borrow(),
                rec.get(),
            );
        });
        // Cursor del espectro.
        let motion = gtk::EventControllerMotion::new();
        let spec = Rc::clone(&a.spec);
        motion.connect_motion(move |_, x, y| spec.borrow_mut().cursor = Some((x, y)));
        let spec = Rc::clone(&a.spec);
        motion.connect_leave(move |_| spec.borrow_mut().cursor = None);
        a.areas[1].add_controller(motion);
    }

    // Disposición.
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(10)
        .build();
    grid.attach(&label("Dispositivo"), 0, 0, 1, 1);
    let dev_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    dev_box.append(&a.device_dd);
    dev_box.append(&a.refresh_btn);
    grid.attach(&dev_box, 1, 0, 3, 1);
    grid.attach(&label("Par"), 4, 0, 1, 1);
    grid.attach(&a.pair_dd, 5, 0, 1, 1);
    grid.attach(&label("Frecuencia"), 6, 0, 1, 1);
    grid.attach(&a.rate_dd, 7, 0, 1, 1);
    grid.attach(&label("Carpeta"), 0, 1, 1, 1);
    grid.attach(&a.folder_btn, 1, 1, 1, 1);
    grid.attach(&label("Disco"), 2, 1, 1, 1);
    grid.attach(&a.disc_entry, 3, 1, 1, 1);
    let sides = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["linked"])
        .build();
    sides.append(&a.side_a);
    sides.append(&a.side_b);
    grid.attach(&label("Cara"), 4, 1, 1, 1);
    grid.attach(&sides, 5, 1, 3, 1);
    grid.attach(&label("Notas"), 0, 2, 1, 1);
    grid.attach(&a.notes_entry, 1, 2, 3, 1);
    grid.attach(&label("Tema"), 4, 2, 1, 1);
    grid.attach(&a.theme_dd, 5, 2, 3, 1);

    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    transport.append(&a.arm_btn);
    transport.append(&a.rec_btn);
    transport.append(&a.stop_btn);
    transport.append(&a.time_lbl);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.set_hexpand(true);
    info.append(&a.free_lbl);
    info.append(&a.status_lbl);
    transport.append(&info);

    let views = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    views.set_vexpand(true);
    views.append(&a.areas[0]);
    let spec_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    spec_col.set_hexpand(true);
    let spec_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    spec_bar.append(&label("Vista"));
    spec_bar.append(&a.view_dd);
    spec_bar.append(&label("Suavizado"));
    spec_bar.append(&a.smooth_dd);
    spec_col.append(&spec_bar);
    a.areas[1].set_vexpand(true);
    spec_col.append(&a.areas[1]);
    views.append(&spec_col);
    views.append(&a.areas[2]);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    root.append(&grid);
    root.append(&transport);
    root.append(&views);
    root.append(&a.areas[3]);
    window.set_child(Some(&root));

    // Señales.
    a.reload_devices();
    {
        let s = Rc::clone(a);
        a.device_dd
            .connect_selected_notify(move |_| s.device_changed());
        let s = Rc::clone(a);
        a.refresh_btn.connect_clicked(move |_| s.reload_devices());
        let s = Rc::clone(a);
        a.theme_dd
            .connect_selected_notify(move |dd| s.set_theme(dd.selected() as usize));
        let s = Rc::clone(a);
        a.view_dd.connect_selected_notify(move |dd| {
            let mode = SpectrumMode::ALL[(dd.selected() as usize).min(2)];
            s.spec.borrow_mut().mode = mode;
            s.smooth_dd.set_sensitive(mode == SpectrumMode::Curve);
            s.config.borrow_mut().spectrum_view = mode.id().into();
            s.config.borrow().save();
            s.areas[1].queue_draw();
        });
        let s = Rc::clone(a);
        a.smooth_dd.connect_selected_notify(move |dd| {
            let f = SMOOTHING[(dd.selected() as usize).min(2)];
            s.spec.borrow_mut().fraction = f;
            s.config.borrow_mut().smoothing = f;
            s.config.borrow().save();
        });
        let s = Rc::clone(a);
        a.folder_btn.connect_clicked(move |_| s.choose_folder());
        let s = Rc::clone(a);
        a.arm_btn.connect_clicked(move |_| s.toggle_arm());
        let s = Rc::clone(a);
        a.rec_btn.connect_clicked(move |_| s.start_recording());
        let s = Rc::clone(a);
        a.stop_btn.connect_clicked(move |_| s.stop_recording());
        let s = Rc::clone(a);
        window.add_tick_callback(move |_, clock| {
            s.tick(clock.frame_time());
            glib::ControlFlow::Continue
        });
        let s = Rc::clone(a);
        window.connect_close_request(move |_| {
            s.shutdown();
            glib::Propagation::Proceed
        });
    }
    window.present();
}

impl App {
    fn alert(&self, message: &str, detail: &str) {
        gtk::AlertDialog::builder()
            .message(message)
            .detail(detail)
            .modal(true)
            .build()
            .show(Some(&self.window));
    }

    fn update_folder_label(&self) {
        let f = self.folder.borrow();
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.display().to_string());
        self.folder_btn.set_label(&name);
        self.folder_btn
            .set_tooltip_text(Some(&f.display().to_string()));
        self.update_free_space();
    }

    fn reload_devices(&self) {
        let devices = device_choices();
        let labels: Vec<&str> = devices.iter().map(|d| d.label.as_str()).collect();
        self.device_dd
            .set_model(Some(&gtk::StringList::new(&labels)));
        *self.devices.borrow_mut() = devices;
        self.device_dd.set_selected(0);
        self.device_changed();
    }

    fn selected_device(&self) -> Option<DeviceChoice> {
        self.devices
            .borrow()
            .get(self.device_dd.selected() as usize)
            .cloned()
    }

    fn device_changed(&self) {
        let Some(d) = self.selected_device() else {
            return;
        };
        let ps: Vec<String> = pairs(d.channels)
            .iter()
            .map(|p| format!("{}–{}", p[0], p[1]))
            .collect();
        let refs: Vec<&str> = ps.iter().map(String::as_str).collect();
        self.pair_dd.set_model(Some(&gtk::StringList::new(&refs)));
        // Plato A llega hoy por 3–4.
        self.pair_dd.set_selected(if ps.len() > 1 { 1 } else { 0 });
        let rates: Vec<String> = d.rates.iter().map(|&r| khz(r)).collect();
        let refs: Vec<&str> = rates.iter().map(String::as_str).collect();
        self.rate_dd.set_model(Some(&gtk::StringList::new(&refs)));
        let best = d.rates.iter().position(|&r| r == 96_000).unwrap_or(0);
        self.rate_dd.set_selected(best as u32);
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Carpeta de los masters")
            .initial_folder(&gio::File::for_path(&*self.folder.borrow()))
            .build();
        let s = Rc::clone(self);
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                *s.folder.borrow_mut() = path;
                s.update_folder_label();
            }
        });
    }

    fn side(&self) -> Side {
        if self.side_b.is_active() {
            Side::B
        } else {
            Side::A
        }
    }

    fn set_armed_ui(&self, armed: bool) {
        for w in [
            self.device_dd.upcast_ref::<gtk::Widget>(),
            self.refresh_btn.upcast_ref(),
            self.pair_dd.upcast_ref(),
            self.rate_dd.upcast_ref(),
        ] {
            w.set_sensitive(!armed);
        }
        self.arm_btn
            .set_label(if armed { "Desarmar" } else { "Armar" });
        self.rec_btn.set_sensitive(armed);
        self.stop_btn.set_sensitive(false);
    }

    fn set_recording_ui(&self, recording: bool) {
        if !recording {
            self.status_lbl.remove_css_class("status-ok");
            self.status_lbl.remove_css_class("status-clip");
            self.status_lbl.add_css_class("dim-label");
        }
        self.arm_btn.set_sensitive(!recording);
        self.rec_btn.set_sensitive(!recording);
        self.stop_btn.set_sensitive(recording);
        for w in [
            self.side_a.upcast_ref::<gtk::Widget>(),
            self.side_b.upcast_ref(),
            self.disc_entry.upcast_ref(),
            self.notes_entry.upcast_ref(),
            self.folder_btn.upcast_ref(),
        ] {
            w.set_sensitive(!recording);
        }
        if recording {
            self.time_lbl.add_css_class("recording");
        } else {
            self.time_lbl.remove_css_class("recording");
        }
        self.recording_view.set(recording);
    }

    fn open_source(
        &self,
        d: &DeviceChoice,
        rate: u32,
    ) -> Result<Box<dyn CaptureSource>, AudioError> {
        Ok(match &d.hw {
            Some(hw) => Box::new(AlsaSource::open(&AlsaRequest::new(hw.clone(), rate, None))?),
            None => Box::new(
                SyntheticSource::new(StreamConfig {
                    rate,
                    format: SampleFormat::S32Le,
                    channels: SYNTH_CHANNELS,
                    period_frames: 1024,
                })
                .realtime(),
            ),
        })
    }

    fn toggle_arm(&self) {
        if self.closing.get() {
            return;
        }
        if let Some(s) = self.session.borrow_mut().take() {
            if let Err(e) = s.close() {
                self.alert("No se pudo soltar el dispositivo", &e.to_string());
            }
            self.set_armed_ui(false);
            self.clear_views();
            self.status_lbl.set_label("Desarmado.");
            return;
        }
        let Some(d) = self.selected_device() else {
            return;
        };
        let Some(&rate) = d.rates.get(self.rate_dd.selected() as usize) else {
            self.alert(
                "Frecuencia no disponible",
                "La tarjeta no ofrece 44,1, 48 ni 96 kHz.",
            );
            return;
        };
        let Some(pair) = pairs(d.channels)
            .get(self.pair_dd.selected() as usize)
            .copied()
        else {
            return;
        };
        let opened = self
            .open_source(&d, rate)
            .and_then(|src| Session::open(src, pair, EngineConfig::default()));
        match opened {
            Ok(s) => {
                let cfg = s.config();
                self.status_lbl.set_label(&format!(
                    "Armado: {} · {} · {} · {} canales · par {}–{}",
                    s.card_name(),
                    khz(cfg.rate),
                    cfg.format,
                    cfg.channels,
                    pair[0],
                    pair[1]
                ));
                *self.session.borrow_mut() = Some(s);
                self.clip_base.set([0, 0]);
                self.clipped.set([false, false]);
                self.set_armed_ui(true);
            }
            Err(e @ AudioError::Busy { .. }) => {
                self.alert("Dispositivo ocupado", &e.to_string());
            }
            Err(e) => self.alert("No se pudo abrir el dispositivo", &e.to_string()),
        }
    }

    fn start_recording(&self) {
        let mut guard = self.session.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return;
        };
        let folder = self.folder.borrow().clone();
        let path = master_path(&folder, &self.disc_entry.text(), self.side(), Path::exists);
        let notes = {
            let n = self.notes_entry.text().to_string();
            let side = format!("Cara {}", self.side().letter());
            if n.trim().is_empty() {
                side
            } else {
                format!("{n} · {side}")
            }
        };
        match session.start_recording(&path, &notes) {
            Ok(()) => {
                let lv = self.snap.borrow().levels;
                self.clip_base.set([lv[0].clip_events, lv[1].clip_events]);
                self.clipped.set([false, false]);
                self.set_recording_ui(true);
                self.status_lbl
                    .set_label(&format!("Grabando en {}", file_name(&path)));
                self.status_lbl
                    .set_tooltip_text(Some(&path.display().to_string()));
            }
            Err(e) => self.alert("No se pudo empezar a grabar", &e.to_string()),
        }
    }

    fn stop_recording(self: &Rc<Self>) {
        let Some(mut session) = self.session.borrow_mut().take() else {
            return;
        };
        self.stop_btn.set_sensitive(false);
        self.status_lbl.set_label("Cerrando el master…");
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let r = session.stop_recording();
            let _ = tx.send_blocking((session, r));
        });
        let s = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Ok((session, result)) = rx.recv().await else {
                return;
            };
            let finished = session.capture_finished();
            *s.session.borrow_mut() = Some(session);
            s.set_recording_ui(false);
            s.recording_done(result);
            if finished {
                s.capture_lost();
            } else if s.side() == Side::A {
                s.side_b.set_active(true);
            }
        });
    }

    fn recording_done(&self, result: Result<RecordingResult, AudioError>) {
        match result {
            Ok(r) => {
                let i = &r.info;
                let xr = if i.xruns.is_empty() {
                    "sin xruns".to_string()
                } else {
                    format!("{} xruns", i.xruns.len())
                };
                self.status_lbl
                    .set_tooltip_text(Some(&r.master.display().to_string()));
                self.status_lbl.set_label(&format!(
                    "Guardado {} ({}, {}, {}). Lista la cara siguiente.",
                    file_name(&r.master),
                    clock(i.duration_s),
                    i.container,
                    xr
                ));
                if !i.xruns.is_empty() || i.frames_lost_ring > 0 {
                    self.alert(
                        "La grabación tiene huecos",
                        &format!(
                            "{} xruns y {} frames perdidos. Los instantes están en {}.",
                            i.xruns.len(),
                            i.frames_lost_ring,
                            r.json.display()
                        ),
                    );
                }
            }
            Err(e) => self.alert("Error al cerrar el master", &e.to_string()),
        }
    }

    /// Medidores vacíos (sin dispositivo armado).
    /// Cambia de tema al momento: CSS de la ventana y todas las gráficas (T2.9e).
    fn set_theme(&self, index: usize) {
        let Some(t) = THEMES.get(index) else {
            return;
        };
        let e = t.effective();
        self.theme.set(e);
        self.css.load_from_string(&css_for(&e));
        for a in &self.areas {
            a.queue_draw();
        }
        self.config.borrow_mut().theme = t.id.into();
        self.config.borrow().save();
    }

    fn clear_views(&self) {
        self.spec.borrow_mut().reset();
        *self.snap.borrow_mut() = LiveSnapshot::default();
        self.clipped.set([false, false]);
        for a in &self.areas {
            a.queue_draw();
        }
    }

    /// La captura se ha parado sola (tarjeta desconectada o error de ALSA).
    fn capture_lost(&self) {
        if let Some(s) = self.session.borrow_mut().take() {
            let _ = s.close();
        }
        self.clear_views();
        self.set_recording_ui(false);
        self.set_armed_ui(false);
        self.alert(
            "La captura se ha detenido",
            "La tarjeta ha dejado de dar audio (¿desconectada?). Si se estaba grabando, el \
             master está guardado hasta ese punto.",
        );
    }

    fn update_free_space(&self) {
        let folder = self.folder.borrow();
        let Ok(st) = rustix::fs::statvfs(&*folder) else {
            self.free_lbl.set_label("");
            return;
        };
        let free = st.f_bavail.saturating_mul(st.f_frsize);
        let rate = self
            .session
            .try_borrow()
            .ok()
            .and_then(|s| s.as_ref().map(|s| s.config()))
            .map(|c| u64::from(c.rate) * 2 * c.format.bytes() as u64)
            .unwrap_or(96_000 * 2 * 4);
        self.free_lbl.set_label(&format!(
            "Libre: {} (≈ {} min)",
            bytes_human(free),
            minutes_left(free, rate)
        ));
    }

    fn tick(self: &Rc<Self>, frame_us: i64) {
        let dt = self
            .last_frame_us
            .replace(Some(frame_us))
            .map_or(1.0 / 60.0, |prev| {
                ((frame_us - prev) as f64 / 1e6).clamp(0.0, 0.5)
            });
        let (finished, status) = {
            let mut guard = self.session.borrow_mut();
            let Some(session) = guard.as_mut() else {
                return;
            };
            self.snap.borrow_mut().clone_from(session.snapshot());
            (session.capture_finished(), session.status())
        };
        self.spec.borrow_mut().update(&self.snap.borrow(), dt);
        let lv = self.snap.borrow().levels;
        let base = self.clip_base.get();
        let c = self.clipped.get();
        self.clipped.set([
            c[0] || lv[0].clip_events > base[0],
            c[1] || lv[1].clip_events > base[1],
        ]);
        for a in &self.areas {
            a.queue_draw();
        }
        let due = self
            .last_labels
            .get()
            .is_none_or(|t| t.elapsed() >= Duration::from_millis(250));
        if due {
            self.last_labels.set(Some(Instant::now()));
            self.time_lbl.set_label(&clock(status.seconds));
            self.update_free_space();
            if status.recording {
                // Estado «grabando sin xruns» en ok; con huecos, en clip.
                let bad = status.xruns > 0 || status.frames_lost > 0;
                self.status_lbl.remove_css_class("dim-label");
                self.status_lbl
                    .remove_css_class(if bad { "status-ok" } else { "status-clip" });
                self.status_lbl
                    .add_css_class(if bad { "status-clip" } else { "status-ok" });
                self.status_lbl.set_label(&format!(
                    "Grabando · xruns {} · perdidos {} · tiempo real: {}",
                    status.xruns,
                    status.frames_lost,
                    if status.realtime { "sí" } else { "no" }
                ));
            }
        }
        if finished {
            if status.recording {
                self.stop_recording();
            } else {
                self.capture_lost();
            }
        }
    }

    fn shutdown(&self) {
        self.closing.set(true);
        if let Some(s) = self.session.borrow_mut().take() {
            // Cierra el master si se estaba grabando: nunca se pierde lo grabado.
            if let Err(e) = s.close() {
                tracing::error!("al cerrar: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pares_de_canales() {
        assert_eq!(pairs(12).len(), 6);
        assert_eq!(pairs(12)[1], [3, 4]);
        assert_eq!(pairs(2), vec![[1, 2]]);
        assert!(pairs(1).is_empty());
    }

    #[test]
    fn siempre_hay_origen_sintetico() {
        let d = device_choices();
        assert!(
            d.last()
                .is_some_and(|d| d.hw.is_none() && d.channels == SYNTH_CHANNELS)
        );
    }
}
