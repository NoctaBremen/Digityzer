# Cambios

## 1.2.0 — 18-11-2026

**Grabación a prueba de cortes (T1.6)**
- Mientras se graba, el master es `<nombre>.wav.partial`. Cada 2 s el hilo escritor (nunca el de
  captura) vuelca lo grabado, actualiza el tamaño en la cabecera WAV/RF64, hace `fdatasync` y
  guarda en `<nombre>.wav.partial.json` las muestras confirmadas. Al parar bien, se renombra al
  nombre final sin sobrescribir nada.
- Si la Xone se apaga o se desconecta, la grabación se cierra limpia con todo lo capturado y su
  ficha la marca como interrumpida (`interrupted`, `interruption`).
- Al abrir el programa (y al cambiar de carpeta) ofrece recuperar los `.partial` que queden: los
  corta en la última muestra confirmada, rehace la cabecera, calcula la huella y anota en la ficha
  que es recuperado y cuántos segundos se perdieron (`recovered`, `seconds_lost`). También
  `digityzer-cli recover [carpeta|archivo]`.
- Probado: matar el proceso con `kill -9` a mitad pierde como mucho 2 s, y lo recuperado es
  idéntico bit a bit a lo capturado hasta ese punto.
- Los avisos de la ventana tienen el botón «Cerrar» en castellano.

## 1.1.0 — 18-11-2026

**Capa de audio elegible**
- Nueva fila «Capa» en la ventana y `--backend` en `digityzer-cli record`: ALSA directo (por
  defecto, la única bit-perfect), PipeWire, PulseAudio o el predeterminado de ALSA.
- PipeWire, PulseAudio y `default` se abren por los plugins de ALSA del sistema (pipewire-alsa,
  pulseaudio-alsa): sin bibliotecas nuevas. Graban la fuente predeterminada del servidor en
  estéreo.
- Aviso «No bit-perfect» en ámbar al elegir la capa, al armar y mientras se graba. La ficha JSON
  del master guarda `audio_backend` y `bit_perfect` (formato versión 2; los JSON anteriores se
  siguen leyendo).
- `cli probe` lista qué capas están disponibles y qué instalar si falta alguna. La capa elegida
  se recuerda en `config.toml`.

**Robustez**
- Si el origen deja de dar audio (un servidor de sonido parado, una tarjeta colgada), parar ya no
  se queda esperando: cada lectura espera como mucho 200 ms y el hilo de captura vuelve a mirar la
  orden de parar. El master se cierra con lo que haya llegado.
- Aviso si una grabación termina sin ninguna muestra.

## 1.0.0 — 18-11-2026

Primera versión estable.

**Grabación bit-perfect**
- `hw:` en exclusiva con el formato nativo (S32_LE, S24_3LE o S16_LE) y la frecuencia exacta;
  sin `plughw`, remuestreo ni conversión.
- Hilo de captura `SCHED_FIFO` que solo llena ring buffers sin locks; xruns registrados con su
  posición.
- Master WAV (RF64 por encima de 4 GiB) con el par elegido byte a byte, cabecera refrescada cada
  segundo, huella blake3 por bloques y ficha JSON. Nunca se sobrescribe; queda en solo lectura.
- Aviso claro si otro programa (xwax) tiene la tarjeta.

**Interfaz GTK4**
- Armar, grabar, parar y pasar a la cara B sin soltar la tarjeta; tiempo y espacio libre.
- Medidores de pico y RMS con retención y aviso de clipping.
- Espectro: curva suave con capas instantánea, media y picos, suavizado de 1/3, 1/6 o 1/12 de
  octava, cursor con frecuencia, nota y cents; barras; cascada en vivo.
- Goniómetro con correlación de fase y forma de onda de los últimos 60 s.
- Diez temas de color con sus paletas oficiales, cambio en caliente y configuración guardada.
- Botones «Acerca de» (versión y créditos) y cerrar; al cerrar grabando, el master se cierra bien.
- Icono propio, con el mismo estilo que Crate Digger y Vinyl Lab.

**Línea de órdenes**
- `probe`, `record` (con Ctrl-C sin perder datos), `verify` y `--simulate` para probar sin mesa.
- `render-themes` genera capturas de referencia de los diez temas.

**Distribución**
- AppImage con los dos programas (`cli …` para la línea de órdenes); ALSA del sistema.

**Créditos:** dirección del proyecto, David Gimenez; programación, Claude (Anthropic).
