# Arquitectura de Digityzer

Documento técnico para quien vaya a tocar el código. El uso está en [MANUAL.md](MANUAL.md) y las
convenciones en [CLAUDE.md](../CLAUDE.md).

## Principios

1. **Master intocable.** La captura se escribe tal cual llega; el master y su JSON no se vuelven a
   abrir en escritura (se crean con `create_new` y quedan en solo lectura).
2. **Bit-perfect.** En la capa ALSA directo: `hw:` en exclusiva, formato nativo, frecuencia
   exacta; nunca `plughw`, nunca remuestreo ni conversión. Lo que no se puede guardar sin
   convertir (S24_LE) se rechaza.

## Capas de audio (1.1)

`digityzer-audio/src/backend.rs`: `AudioBackend` = `Alsa` (bit-perfect), `PipeWire`, `Pulse` y
`Default`. Las tres últimas usan el mismo `AlsaSource` abriendo el PCM del plugin del sistema
(`pipewire`, `pulse`, `default`) en estéreo; no se enlaza libpipewire ni libpulse, así que el
AppImage no cambia y funciona con el ALSA del sistema. `available()` prueba a abrir el PCM. El
master guarda `audio_backend` y `bit_perfect` en su JSON (versión 2 del formato; los JSON de la
versión 1 se leen como ALSA bit-perfect).
3. **El hilo de captura no se bloquea.** Solo lee de ALSA y copia a ring buffers sin locks. Si
   algo va lento, se pierden fotogramas de los medidores, nunca muestras del master.
4. **Lógica pura y probada.** DSP, formatos, nombres y escalas son funciones sin GTK ni ALSA,
   probadas con señales sintéticas.

## Crates

| Crate | Responsabilidad |
|---|---|
| `audio-core` | Código compartido con Spector (espectrograma, cortes y falsos hi-res, AIFF y etiquetas, widgets GTK tras el feature `gtk`) y los temas de color (propios). Procedencia en `crates/audio-core/PROVENANCE.md` |
| `digityzer-audio` | `probe`, origen ALSA y sintético, motor de captura, escritor WAV/RF64, sesión y verificación |
| `digityzer-dsp` | Medidores, espectro en vivo (capas, suavizado, balística, notas), correlación, goniómetro y forma de onda |
| `digityzer-project` | JSON del master (`MasterInfo`) y fechas UTC |
| `digityzer-app` | Binarios `digityzer` (GTK4) y `digityzer-cli` |
| `digityzer-testkit` | Señales sintéticas deterministas para los tests (solo dev) |

## Hilos y flujo de datos

```text
              ┌───────────────────────────── hilo de captura (SCHED_FIFO) ──┐
 ALSA hw: ───▶│ readi(período) ──▶ ring «escritor» (10 s, rtrb)             │
              │                 └─▶ ring «análisis» (1 s, rtrb, con pérdidas)│
              │ xruns ──▶ ring de eventos · contadores atómicos             │
              └─────────────────────────────────────────────────────────────┘
                      │                                │
          hilo escritor (normal)              hilo de análisis (normal)
          · armado: vacía sin guardar         · decodifica el par a i32
          · grabando: par byte a byte          · LiveAnalyzer: medidores, espectro,
            → MasterWriter (WAV/RF64,            correlación, goniómetro, onda
            huella blake3, cabecera/1 s)       · instantánea cada 16 ms ──▶ triple buffer
          · al parar: finalize + JSON                                     │
                                                         hilo principal GTK (60 fps)
                                                         · lee la instantánea sin locks
                                                         · SpectrumView (suavizado,
                                                           balística, cascada) y dibujo
```

- **`Session`** (`digityzer-audio/src/recorder.rs`) mantiene el dispositivo abierto: armar,
  grabar, parar y volver a grabar sin soltar la tarjeta. La grabación empieza en el siguiente
  frame completo. Al parar con la captura terminada (Ctrl-C), primero se escribe todo lo
  pendiente.
- **`Recorder`** es la forma corta para el CLI (abre, graba, y al parar detiene la captura y
  drena).
- Cerrar el master (`fsync`) se hace fuera del hilo principal de GTK.

## Grabación a prueba de cortes (T1.6)

- `MasterWriter` escribe en `<master>.partial` con un búfer de 64 KiB y renombra al nombre final
  (enlace duro + borrado: nunca sobrescribe) al cerrar.
- El hilo escritor llama cada `SYNC_INTERVAL` (2 s) a `sync_point`: vuelca, reescribe la cabecera
  y hace `fdatasync`; después guarda `PartialInfo` (ficha del master a medias + frames
  confirmados + hora) en `<master>.partial.json` de forma atómica. El hilo de captura no toca
  nada de esto.
- Error de dispositivo en la captura → el hilo de captura termina con error → la sesión cierra el
  master con lo que haya y la ficha lleva `interrupted`/`interruption`.
- `recover.rs`: `find_partials`, `recover` (corta en lo confirmado, rehace la cabecera, huella,
  `final_name` sin sobrescribir, ficha con `recovered`/`seconds_lost`). La interfaz lo ofrece al
  abrir y al cambiar de carpeta; el CLI con `recover`.
- Formato de la ficha: versión 3 (las anteriores se siguen leyendo).

## Formatos

### Master WAV/RF64

Cabecera fija de 104 bytes escrita a mano:

```text
RIFF|RF64  tamaño  WAVE
JUNK|ds64  28 bytes (en RF64: tamaño RIFF, tamaño data y frames en u64)
fmt        40 bytes (WAVE_FORMAT_EXTENSIBLE, PCM, 2 canales FL+FR)
data       tamaño   … muestras del par, en el formato nativo …
```

Pasa a RF64 cuando el tamaño RIFF supera 4 GiB. La cabecera se reescribe cada segundo.

### Huella

blake3 de los bytes del chunk `data`, por bloques de 1 MiB y total. `verify_master` relee el
archivo y compara bloque a bloque.

## Espectro en vivo

- FFT Hann de `live_fft_size(rate)` (16.384 a 96 kHz, ≈ 5,9 Hz por bin) con el `FrameAnalyzer`
  de `audio-core`, sobre la mezcla del par.
- Tres capas en potencia por bin: instantánea (α = 0,6), media (τ = 1,5 s) y picos (caída de
  6 dB/s).
- Pantalla: `smooth_curve` integra la potencia interpolada linealmente entre bins en una ventana
  de 1/N octava (sin escalones en graves); `log_bands` para las barras; `Ballistics` (τ = 0,25 s)
  desliza las curvas a 60 fps.
- Cascada: 600 columnas × 256 filas en dB, repintadas por columna (o enteras al cambiar de tema),
  rango −90…−10 dBFS con respuesta cuadrática.

## Temas

`audio-core/src/theme.rs`: once roles por tema; `THEMES` con los valores del plan, test contra las
paletas oficiales, `effective()` con el ajuste de contraste (texto ≥ 4,5:1, tenue ≥ 3:1),
`gtk_css()` para la ventana y `waterfall_gradient()` para la cascada. `ui/draw.rs` no tiene ningún
color fijo (un test lo comprueba).

## Tests

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release -p digityzer-dsp -- --ignored --nocapture   # rendimiento del análisis
```

El CI (GitHub Actions) ejecuta fmt, clippy y tests en cada PR y push a `main`, más una
comprobación con Rust 1.92. El workflow de release construye y publica el AppImage al crear una
etiqueta `v*`. Las pruebas que necesitan la mesa están en
[PRUEBAS_HARDWARE.md](PRUEBAS_HARDWARE.md).
