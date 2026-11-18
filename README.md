# Digityzer

Digitalizador de vinilo en Rust + GTK4: captura bit-perfect desde la Xone:96 por ALSA, mide la
calidad de la captura y de la cadena mecánica y exporta las pistas a AIFF. El master crudo nunca
se modifica.

Entorno: Arch Linux, Hyprland (Wayland), kernel PREEMPT_RT, solo ALSA (sin PipeWire, PulseAudio
ni JACK).

## Workspace

| Crate | Responsabilidad |
|---|---|
| `crates/audio-core` | Código compartido con Spector (espectrograma, cortes y falsos hi-res, AIFF con etiquetas, widgets GTK4). Ver `PROVENANCE.md` |
| `crates/digityzer-testkit` | Señales sintéticas deterministas para los tests (solo dev) |

## Comandos

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Para compilar hacen falta las cabeceras de GTK 4.14 y ALSA (`gtk4` y `alsa-lib` en Arch).
