# Pruebas con la mesa (fase 1)

Lo que los tests automáticos no pueden comprobar: necesitan la Xone:96 conectada. Todo lo demás
(motor, escritor, JSON, huella, Ctrl-C, xruns simulados) lo cubre `cargo test`.

## Preparación (Arch Linux)

```bash
sudo pacman -S --needed rust gtk4 alsa-lib alsa-utils realtime-privileges
sudo usermod -aG realtime "$USER"     # SCHED_FIFO sin root; cerrar sesión y volver a entrar
cargo build --release
```

`realtime-privileges` da al grupo `realtime` permiso de `rtprio`. Sin él la grabación funciona
igual, pero el programa avisa de que no tiene prioridad de tiempo real.

## T1.1 — `probe`

```bash
./target/release/digityzer-cli probe
```

Acepta: aparecen XONE 96 USB 1 y USB 2 con su formato nativo (previsiblemente `S32_LE` o
`S24_3LE`), sus frecuencias y sus canales. Copia de aquí el nombre `hw:CARD=…,DEV=…` para
grabar.

## T1.5 — dispositivo ocupado

Con xwax abierto sobre USB 1:

```bash
./target/release/digityzer-cli record --device hw:CARD=<USB1>,DEV=0 --pair 3-4 prueba.wav
```

Acepta: mensaje «… está en uso por otro programa (¿xwax abierto?)…» y código de salida 2.
`probe` muestra ese dispositivo como «en uso».

## T1.4 — grabación de 20 minutos

Plato A (entradas 3–4), conmutador USB del canal en PHONO, pitch a 0:

```bash
./target/release/digityzer-cli record --device hw:CARD=<USB1>,DEV=0 --pair 3-4 \
    --notes "Sello, catálogo, cara A" cara-a.wav
```

Para con Ctrl-C cuando acabe la cara. Acepta: `cara-a.wav` abre bien en Audacity y en Spek;
`cara-a.wav.json` existe; `digityzer-cli verify cara-a.wav` dice «correcto».

## T1.2 — 60 minutos sin xruns

```bash
./target/release/digityzer-cli record --device hw:CARD=<USB1>,DEV=0 --pair 3-4 \
    --duration 3600 hora.wav
```

Acepta: el resumen final dice `0 xruns` y `0 frames perdidos`, y `"realtime": true` en el JSON.
Para el xrun provocado: durante una grabación, `kill -STOP <pid>` unos segundos y `kill -CONT
<pid>`; el JSON debe listar el xrun con su frame y su segundo.

## T1.3 — huella

`digityzer-cli verify <master>` después de cada prueba. Debe decir «correcto».
