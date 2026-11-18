# Procedencia de `audio-core`

Código copiado del repositorio [Spector](https://github.com/NoctaBremen/Spector) (de solo lectura
para Digityzer: Spector no recibe ningún cambio).

- **Commit de origen**: `49724e80e279386a3d68ed85e4c9637527d53171`
  («F5-T6: pulido, exportar PNG con ejes y versión 1.0.0», 18-11-2026). También está en
  `audio_core::SPECTOR_COMMIT`.
- **Licencia**: MIT en los dos proyectos.

Si se vuelve a copiar algo de Spector, actualiza el commit y esta tabla.

## Archivos

| Digityzer | Origen en Spector | Cambios |
|---|---|---|
| `src/cancel.rs` | `spector-core/src/cancel.rs` | Ninguno |
| `src/error.rs` | `spector-core/src/error.rs` | Añadido `TagError` (era `ConvertError::Tags`) |
| `src/fsutil.rs` | `spector-core/src/fsutil.rs` | `TEMP_PREFIX` local: `.digityzer-tmp-` (era `scan::TEMP_PREFIX`) |
| `src/pcm.rs` | `spector-core/src/decode.rs` (`PcmData`, `ChannelView`, `PcmBlock`) | Sin symphonia; añadido `PcmFormat` |
| `src/dsp/{mod,window,frame}.rs` | `spector-core/src/dsp/` | Ruta de importación de `pcm` |
| `src/spectrogram.rs` | `spector-core/src/spectrogram.rs` | `compute_spectrogram` recibe bloques + `PcmFormat` en lugar de una ruta; sin `compute_spectrogram_with_info` |
| `src/palette.rs`, `src/ticks.rs` | `spector-core/src/` | Ninguno |
| `src/analysis/{stats,thresholds,cutoff,upsampling}.rs` | `spector-core/src/analysis/` | Ninguno |
| `src/analysis/mod.rs` | `spector-core/src/analysis/mod.rs` | Solo `enbw_bins`, `frame_rms_db` y `analyze_spectrum` (la parte espectral de `analyze_raw`, con bloques). Sin profundidad de bits, clipping ni veredicto |
| `src/aiff/{error,extended,reader,writer}.rs` | `spector-aiff/src/` | Rutas de importación. Sin `writer_aifc` (float) |
| `src/tags.rs` | `spector-convert/src/tags.rs` | `ConvertError` → `TagError` |
| `src/gtk/spectrogram_view.rs` | `spector-gtk/src/spectrogram_view.rs` | Rutas de importación; `impl Default` (clippy lo pide en una biblioteca) |
| `src/gtk/export.rs` | `spector-gtk/src/export.rs` | Rutas de importación |
| `src/gtk/style.rs` | `spector-gtk/src/app.rs` (`CSS`, `load_css`) | Extraído a un módulo propio |
| `src/gtk/strings.rs` | `spector-gtk/src/strings.rs` | Solo `UNKNOWN_DURATION` |
| `tests/spectrogram.rs` | `spector-core/tests/spectrogram.rs` | Lee los WAV con `hound` (`tests/common`); sin `wav_y_flac_dan_lo_mismo` |
| `tests/detectors.rs` | `spector-core/tests/detectors.rs` | Solo cortes y sobremuestreo; el MP3 decodificado se guarda en WAV, no en FLAC |
| `tests/aiff_roundtrip.rs`, `tests/aiff_robustness.rs` | `spector-aiff/tests/` | Sin `aifc_float_bits_exactos` |
| `tests/tags.rs` | — | Nuevo: Spector probaba con etiquetas leídas de FLAC/ALAC; aquí se construyen a mano (metadatos de T5.3) |
| `../digityzer-testkit/src/{signals,spectral,wav,ffmpeg,corrupt}.rs` | `spector-testkit/src/` | Ninguno |

## No copiado

Decodificación con symphonia, información de archivo, veredicto, profundidad de bits, clipping,
escaneo, lotes, configuración, biblioteca, conversión y el resto de la interfaz de Spector.
Digityzer graba su propio master y no los necesita.

## Feature `gtk`

El widget, la exportación a PNG y el estilo están detrás del feature `gtk` para que
`digityzer-cli` y las bibliotecas no dependan de GTK.

## Código propio

`src/theme.rs` (temas de color, T2.9) no viene de Spector: es de Digityzer y está aquí para que
Spector pueda adoptarlo más adelante en su propio proyecto.
