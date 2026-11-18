# CLAUDE.md — Convenciones de Digityzer

El alcance de cada tarea está en el plan («Plan — Digityzer», tareas `T<fase>.<n>`). En estilo
manda este archivo.

## Qué es

Digitalizador de vinilo en Rust + GTK4 (gtk4-rs) para Arch Linux, Hyprland (Wayland) y kernel
PREEMPT_RT, **solo ALSA** (sin PipeWire, PulseAudio ni JACK). Captura desde la Xone:96 por USB
de forma bit-perfect, mide la captura y la cadena mecánica y exporta AIFF.

## Reglas que no se rompen

1. **Master intocable**: la captura se escribe tal cual llega de la tarjeta. Cortes, limpieza o
   conversión generan archivos nuevos. El master y sus JSON nunca se abren en escritura después
   de cerrar la grabación.
2. **Bit-perfect**: dispositivo `hw:` en exclusiva, nunca `plughw`, sin remuestreo ni conversión
   de formato en la captura. El formato nativo se detecta al abrir y no se convierte.
3. **Hilo de captura**: solo lee de ALSA y empuja al ring buffer (`rtrb`). Nada que bloquee
   (E/S, locks, asignaciones grandes, registro) corre en él. Si la interfaz se atasca se pierden
   fotogramas de los medidores, nunca muestras del master.
4. **Spector no se toca**: es de solo lectura. Lo compartido vive en `crates/audio-core`; si se
   vuelve a copiar algo, se anota en `crates/audio-core/PROVENANCE.md`.

## Flujo de trabajo

- Una tarea = una rama = un PR, con el identificador (`T1.2`) en el título y la plantilla de
  `.github/pull_request_template.md` rellena, incluida «Desviaciones del plan».
- Nada se fusiona sin el CI en verde. Antes de abrir el PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

- `Cargo.lock` se versiona y el CI usa `--locked`.
- Dependencias solo por `[workspace.dependencies]` (`nombre = { workspace = true }`).

## Estilo

- Edición 2024, MSRV 1.92, `max_width = 100`. GTK limitado a 4.14 (feature `v4_14`).
- Errores con `thiserror` en las bibliotecas y `anyhow` en los binarios; mensajes en castellano.
- Nunca `unwrap()`/`expect()` sobre datos externos (lints `deny`; permitidos en tests). Los
  tests de integración empiezan con `#![allow(clippy::unwrap_used, clippy::expect_used)]`.
- `unsafe` prohibido salvo justificación explícita en el PR.
- Registro con `tracing` (nivel con `DIGITYZER_LOG`); nada de `println!` en las bibliotecas.
- Doc-comments en toda API pública; lógica en funciones puras que se prueban sin GTK ni ALSA.

## Tests

- Las tareas de DSP se prueban con señales sintéticas de `digityzer-testkit` (senos, ruido,
  tonos con modulación de velocidad, frecuencia y nivel, clics, zumbido, diafonía, ruido rosa).
  Cada módulo de DSP usa el testkit en al menos un test.
- Prohibido versionar archivos de audio: todo se genera al vuelo en directorios temporales.
- Los tests que necesitan `ffmpeg` se saltan solos (`require_ffmpeg!()`).

## Mapa del workspace

| Crate | Responsabilidad |
|---|---|
| `audio-core` | Copiado de Spector: espectrograma, cortes y falsos hi-res, AIFF y etiquetas, widgets GTK (feature `gtk`) |
| `digityzer-audio` | Captura ALSA y escritura del master |
| `digityzer-dsp` | Medidores, calibración y diagnóstico |
| `digityzer-project` | Proyecto, marcadores y JSON del master |
| `digityzer-app` | Binarios `digityzer` (GUI) y `digityzer-cli` |
| `digityzer-testkit` | Señales sintéticas para los tests (solo dev) |
