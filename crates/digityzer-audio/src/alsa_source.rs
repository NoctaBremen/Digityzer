//! Captura directa en `hw:` (T1.2): dispositivo en exclusiva, todos los canales, formato nativo y
//! frecuencia exacta. Nunca `plughw`, nunca remuestreo ni conversión de formato.

use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};

use crate::backend::AudioBackend;
use crate::error::{AudioError, errno_of};
use crate::format::SampleFormat;
use crate::source::{CaptureSource, ReadOutcome, StreamConfig};

/// `errno` de xrun (EPIPE) y de suspensión (ESTRPIPE).
const EPIPE: i32 = 32;
const ESTRPIPE: i32 = 86;

/// Tope de espera de datos en cada lectura (ms).
pub const READ_WAIT_MS: u32 = 200;

/// Frames por período pedidos a ALSA (≈ 10 ms a 96 kHz).
pub const DEFAULT_PERIOD_FRAMES: usize = 1024;
/// Períodos en el búfer de ALSA (≈ 170 ms a 96 kHz): margen ante picos de latencia.
pub const DEFAULT_PERIODS: usize = 16;

/// Parte de tarjeta de un nombre `hw:` (`hw:CARD=USB1,DEV=0` → `USB1`, `hw:2,0` → `2`).
pub fn card_of(device: &str) -> Option<String> {
    let rest = device.strip_prefix("hw:")?;
    let first = rest.split(',').next()?;
    let card = first.strip_prefix("CARD=").unwrap_or(first);
    (!card.is_empty()).then(|| card.to_string())
}

/// Nombre legible de la tarjeta de un dispositivo `hw:`, o el propio dispositivo si no se sabe.
pub fn card_name(device: &str) -> String {
    card_of(device)
        .and_then(|c| alsa::Ctl::new(&format!("hw:{c}"), false).ok())
        .and_then(|ctl| {
            ctl.card_info()
                .ok()
                .and_then(|i| i.get_name().ok().map(str::to_string))
        })
        .unwrap_or_else(|| device.to_string())
}

/// Lo que se pide al abrir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlsaRequest {
    /// Capa de audio.
    pub backend: AudioBackend,
    /// Dispositivo; en ALSA directo debe empezar por `hw:`. En las demás capas no se usa.
    pub device: String,
    /// Frecuencia exacta (Hz).
    pub rate: u32,
    /// Formato; `None` = el mejor que admita la tarjeta de [`SampleFormat::PREFERENCE`].
    pub format: Option<SampleFormat>,
    /// Frames por período.
    pub period_frames: usize,
}

impl AlsaRequest {
    /// Petición con período por defecto.
    pub fn new(device: impl Into<String>, rate: u32, format: Option<SampleFormat>) -> Self {
        Self {
            backend: AudioBackend::Alsa,
            device: device.into(),
            rate,
            format,
            period_frames: DEFAULT_PERIOD_FRAMES,
        }
    }
}

impl AlsaRequest {
    /// Petición por una capa por plugin (PipeWire, PulseAudio o `default`).
    pub fn plugin(backend: AudioBackend, rate: u32, format: Option<SampleFormat>) -> Self {
        Self {
            backend,
            device: backend.pcm_name().unwrap_or("default").to_string(),
            rate,
            format,
            period_frames: DEFAULT_PERIOD_FRAMES,
        }
    }
}

/// Origen ALSA abierto y configurado.
pub struct AlsaSource {
    backend: AudioBackend,
    pcm: PCM,
    device: String,
    card_name: String,
    config: StreamConfig,
}

impl std::fmt::Debug for AlsaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlsaSource")
            .field("device", &self.device)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Formato elegido entre los que admite la tarjeta (`admite(f)`), o un error que los explica.
pub fn choose_format(
    requested: Option<SampleFormat>,
    supports: impl Fn(SampleFormat) -> bool,
    s24_le_available: bool,
) -> Result<SampleFormat, AudioError> {
    if let Some(f) = requested {
        return if supports(f) {
            Ok(f)
        } else {
            Err(AudioError::Unsupported(format!(
                "la tarjeta no admite {f}; sin conversión no se puede grabar en ese formato"
            )))
        };
    }
    SampleFormat::PREFERENCE
        .into_iter()
        .find(|&f| supports(f))
        .ok_or_else(|| {
            AudioError::Unsupported(if s24_le_available {
                "la tarjeta solo ofrece S24_LE (24 bits en la parte baja de 32), que no se puede \
                 guardar en WAV sin desplazar las muestras"
                    .into()
            } else {
                "la tarjeta no ofrece ningún formato entero que se pueda grabar sin convertir \
                 (S32_LE, S24_3LE o S16_LE)"
                    .into()
            })
        })
}

impl AlsaSource {
    /// Abre el dispositivo en exclusiva y lo configura. `EBUSY` → [`AudioError::Busy`] (T1.5).
    pub fn open(req: &AlsaRequest) -> Result<Self, AudioError> {
        let plugin = req.backend.pcm_name();
        if plugin.is_none() && !req.device.starts_with("hw:") {
            return Err(AudioError::Invalid(format!(
                "«{}» no es un dispositivo hw:; en ALSA directo solo se graba en hw: para ser \
                 bit-perfect",
                req.device
            )));
        }
        let device = plugin.map_or_else(|| req.device.clone(), str::to_string);
        let name = match plugin {
            None => card_name(&device),
            Some(_) => req.backend.source_name().to_string(),
        };
        let err = |ctx: &str| {
            let (d, n) = (device.clone(), name.clone());
            let ctx = ctx.to_string();
            move |e| AudioError::from_alsa(ctx, &d, &n, e)
        };
        let pcm = match PCM::new(&device, Direction::Capture, false) {
            Ok(p) => p,
            Err(e) if plugin.is_some() => {
                return Err(AudioError::Unsupported(format!(
                    "la capa {} no está disponible ({e}). Hace falta {}",
                    req.backend.label(),
                    req.backend.install_hint()
                )));
            }
            Err(e) => return Err(err("abrir")(e)),
        };
        let config = {
            let hwp = HwParams::any(&pcm).map_err(err("leer parámetros"))?;
            hwp.set_access(Access::RWInterleaved)
                .map_err(err("acceso entrelazado"))?;
            let format = choose_format(
                req.format,
                |f| hwp.test_format(f.to_alsa()).is_ok(),
                hwp.test_format(Format::S24LE).is_ok(),
            )?;
            hwp.set_format(format.to_alsa()).map_err(err("formato"))?;
            // ALSA directo: todos los canales de la interfaz. Por plugin: estéreo, la fuente
            // predeterminada del servidor.
            let channels = if plugin.is_some() && hwp.test_channels(2).is_ok() {
                2
            } else {
                hwp.get_channels_max().map_err(err("canales"))?
            };
            hwp.set_channels(channels).map_err(err("canales"))?;
            if hwp.test_rate(req.rate).is_err() {
                return Err(AudioError::Unsupported(format!(
                    "{name} no admite {} Hz de forma nativa y no se remuestrea",
                    req.rate
                )));
            }
            hwp.set_rate(req.rate, ValueOr::Nearest)
                .map_err(err("frecuencia"))?;
            hwp.set_period_size_near(req.period_frames as alsa::pcm::Frames, ValueOr::Nearest)
                .map_err(err("período"))?;
            hwp.set_buffer_size_near((req.period_frames * DEFAULT_PERIODS) as alsa::pcm::Frames)
                .map_err(err("búfer"))?;
            pcm.hw_params(&hwp).map_err(err("aplicar parámetros"))?;
            let cur = pcm.hw_params_current().map_err(err("leer parámetros"))?;
            let rate = cur.get_rate().map_err(err("frecuencia"))?;
            if rate != req.rate {
                return Err(AudioError::Unsupported(format!(
                    "{name} ha dado {rate} Hz en lugar de {} Hz",
                    req.rate
                )));
            }
            StreamConfig {
                rate,
                format,
                channels: u16::try_from(channels)
                    .map_err(|_| AudioError::Unsupported(format!("{channels} canales")))?,
                period_frames: usize::try_from(cur.get_period_size().map_err(err("período"))?)
                    .unwrap_or(req.period_frames)
                    .max(1),
            }
        };
        Ok(Self {
            backend: req.backend,
            pcm,
            device,
            card_name: name,
            config,
        })
    }
}

impl CaptureSource for AlsaSource {
    fn config(&self) -> StreamConfig {
        self.config
    }

    fn device(&self) -> &str {
        &self.device
    }

    fn card_name(&self) -> &str {
        &self.card_name
    }

    fn backend(&self) -> AudioBackend {
        self.backend
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.pcm.prepare().map_err(|e| AudioError::Alsa {
            context: "preparar".into(),
            source: e,
        })?;
        self.pcm.start().map_err(|e| AudioError::Alsa {
            context: "arrancar".into(),
            source: e,
        })
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<ReadOutcome, AudioError> {
        let fb = self.config.frame_bytes();
        let frames = (buf.len() / fb).min(self.config.period_frames);
        // Espera con tope (un `poll`, sin reservas): si el origen deja de dar datos (un servidor
        // de sonido parado, una tarjeta colgada), el hilo vuelve a mirar la orden de parar en vez
        // de quedarse bloqueado en `readi` para siempre.
        match self.pcm.wait(Some(READ_WAIT_MS)) {
            Ok(false) => return Ok(ReadOutcome::Frames(0)),
            Ok(true) => {}
            // Un xrun o una suspensión también despiertan a `wait`: los trata `readi` abajo.
            Err(e) if errno_of(&e) == EPIPE || errno_of(&e) == ESTRPIPE => {}
            Err(e) => {
                return Err(AudioError::Alsa {
                    context: "esperar datos".into(),
                    source: e,
                });
            }
        }
        let io = self.pcm.io_bytes();
        match io.readi(&mut buf[..frames * fb]) {
            Ok(n) => Ok(ReadOutcome::Frames(n)),
            Err(e) if errno_of(&e) == EPIPE || errno_of(&e) == ESTRPIPE => {
                // Xrun (o suspensión): se recupera y se sigue capturando; el hueco queda
                // registrado por el motor.
                self.pcm
                    .try_recover(e, true)
                    .and_then(|()| self.pcm.start())
                    .map_err(|e| AudioError::Alsa {
                        context: "recuperar tras un xrun".into(),
                        source: e,
                    })?;
                Ok(ReadOutcome::Xrun)
            }
            Err(e) => Err(AudioError::Alsa {
                context: "leer".into(),
                source: e,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarjeta_de_un_nombre_hw() {
        assert_eq!(card_of("hw:CARD=USB1,DEV=0").as_deref(), Some("USB1"));
        assert_eq!(card_of("hw:2,0").as_deref(), Some("2"));
        assert_eq!(card_of("hw:XONE96"), Some("XONE96".into()));
        assert_eq!(card_of("plughw:1,0"), None);
        assert_eq!(card_of("hw:"), None);
    }

    #[test]
    fn eleccion_de_formato() {
        use SampleFormat::*;
        assert_eq!(choose_format(None, |_| true, false).unwrap(), S32Le);
        assert_eq!(choose_format(None, |f| f != S32Le, false).unwrap(), S24_3Le);
        assert_eq!(choose_format(Some(S16Le), |_| true, false).unwrap(), S16Le);
        assert!(choose_format(Some(S16Le), |f| f == S32Le, false).is_err());
        let e = choose_format(None, |_| false, true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("S24_LE"), "{e}");
    }

    #[test]
    fn solo_hw() {
        let r = AlsaSource::open(&AlsaRequest::new("plughw:0,0", 96_000, None));
        assert!(matches!(r, Err(AudioError::Invalid(_))));
    }
}
