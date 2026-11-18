# Cambios

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
