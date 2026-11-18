//! Escritor del master (T1.3): WAV con el formato nativo, o RF64 si pasa de 4 GiB, escrito a
//! mano. Del flujo entrelazado de todos los canales se copian **los bytes** del par elegido, sin
//! tocarlos; la huella blake3 por bloques se calcula sobre esos mismos bytes.
//!
//! Disposición fija de la cabecera (104 bytes):
//!
//! ```text
//! RIFF|RF64  tamaño  WAVE
//! JUNK|ds64  28 bytes (reservados; en RF64: tamaño RIFF, tamaño data y frames en u64)
//! fmt        40 bytes (WAVE_FORMAT_EXTENSIBLE, PCM, 2 canales FL+FR)
//! data       tamaño   … muestras …
//! ```
//!
//! A prueba de cortes (T1.6): mientras se graba, el archivo es `<master>.partial`. El hilo
//! escritor (nunca el de captura) llama cada [`SYNC_INTERVAL`] a [`MasterWriter::sync_point`]:
//! vuelca el búfer, reescribe la cabecera con el tamaño y hace `fdatasync`, y esos frames quedan
//! **confirmados** en el disco. Al cerrar bien, se renombra al nombre final sin sobrescribir nada.
//! Si el proceso muere, `recover` corta el `.partial` en la última muestra confirmada.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::AudioError;
use crate::format::SampleFormat;

/// Longitud de la cabecera.
pub const HEADER_LEN: u64 = 104;
/// Bytes de la cabecera que cuentan en el tamaño RIFF además de `data`.
const RIFF_OVERHEAD: u64 = HEADER_LEN - 8;
/// Bloque de la huella.
pub const HASH_BLOCK_BYTES: u64 = 1 << 20;
/// Cada cuánto se confirma lo escrito en el disco (cabecera + `fdatasync`).
pub const SYNC_INTERVAL: Duration = Duration::from_secs(2);

/// Búfer de escritura del master (bytes).
pub const WRITE_BUFFER: usize = 64 * 1024;

/// Extensión del archivo mientras se graba.
pub const PARTIAL_SUFFIX: &str = ".partial";

/// Ruta del archivo en curso de un master: `<master>.partial`.
pub fn partial_path(master: &Path) -> PathBuf {
    let mut s = master.as_os_str().to_owned();
    s.push(PARTIAL_SUFFIX);
    PathBuf::from(s)
}

/// Cabecera de [`HEADER_LEN`] bytes para `data_bytes` de audio de un par en `format` a `rate`.
/// Pasa a RF64 si el tamaño RIFF supera `rf64_threshold`.
pub fn header_bytes(
    format: SampleFormat,
    rate: u32,
    data_bytes: u64,
    rf64_threshold: u64,
) -> [u8; HEADER_LEN as usize] {
    let mut h = [0u8; HEADER_LEN as usize];
    let riff = data_bytes + RIFF_OVERHEAD;
    let rf64 = riff > rf64_threshold;
    let bits = format.bits();
    let block_align = 2 * format.bytes() as u16;
    h[0..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    h[4..8].copy_from_slice(&(if rf64 { u32::MAX } else { riff as u32 }).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(if rf64 { b"ds64" } else { b"JUNK" });
    h[16..20].copy_from_slice(&28u32.to_le_bytes());
    if rf64 {
        h[20..28].copy_from_slice(&riff.to_le_bytes());
        h[28..36].copy_from_slice(&data_bytes.to_le_bytes());
        h[36..44].copy_from_slice(&(data_bytes / u64::from(block_align)).to_le_bytes());
        // 44..48: tabla de chunks vacía.
    }
    h[48..52].copy_from_slice(b"fmt ");
    h[52..56].copy_from_slice(&40u32.to_le_bytes());
    h[56..58].copy_from_slice(&0xFFFEu16.to_le_bytes());
    h[58..60].copy_from_slice(&2u16.to_le_bytes());
    h[60..64].copy_from_slice(&rate.to_le_bytes());
    h[64..68].copy_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    h[68..70].copy_from_slice(&block_align.to_le_bytes());
    h[70..72].copy_from_slice(&bits.to_le_bytes());
    h[72..74].copy_from_slice(&22u16.to_le_bytes());
    h[74..76].copy_from_slice(&bits.to_le_bytes());
    h[76..80].copy_from_slice(&3u32.to_le_bytes());
    h[80..96].copy_from_slice(&PCM_GUID);
    h[96..100].copy_from_slice(b"data");
    let data = if rf64 { u32::MAX } else { data_bytes as u32 };
    h[100..104].copy_from_slice(&data.to_le_bytes());
    h
}

/// Renombra `from` a `to` sin sobrescribir nunca: enlace duro (falla si `to` existe) y después
/// borra `from`.
pub fn rename_no_overwrite(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}
/// GUID de `KSDATAFORMAT_SUBTYPE_PCM`.
const PCM_GUID: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// Huella blake3 por bloques fijos de un flujo de bytes.
#[derive(Debug, Clone)]
pub struct BlockHasher {
    block: u64,
    in_block: u64,
    current: blake3::Hasher,
    total: blake3::Hasher,
    blocks: Vec<String>,
}

impl BlockHasher {
    /// Huella con bloques de `block` bytes.
    pub fn new(block: u64) -> Self {
        Self {
            block: block.max(1),
            in_block: 0,
            current: blake3::Hasher::new(),
            total: blake3::Hasher::new(),
            blocks: Vec::new(),
        }
    }

    /// Añade bytes.
    pub fn update(&mut self, mut bytes: &[u8]) {
        self.total.update(bytes);
        while !bytes.is_empty() {
            let room = (self.block - self.in_block) as usize;
            let (now, rest) = bytes.split_at(room.min(bytes.len()));
            self.current.update(now);
            self.in_block += now.len() as u64;
            if self.in_block == self.block {
                self.blocks
                    .push(self.current.finalize().to_hex().to_string());
                self.current = blake3::Hasher::new();
                self.in_block = 0;
            }
            bytes = rest;
        }
    }

    /// Termina: (huellas de los bloques, huella total).
    pub fn finish(mut self) -> (Vec<String>, String) {
        if self.in_block > 0 {
            self.blocks
                .push(self.current.finalize().to_hex().to_string());
        }
        (self.blocks, self.total.finalize().to_hex().to_string())
    }
}

/// Resumen de un master cerrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterSummary {
    /// Ruta.
    pub path: PathBuf,
    /// Frames escritos.
    pub frames: u64,
    /// Bytes de audio.
    pub data_bytes: u64,
    /// `true` si es RF64.
    pub rf64: bool,
    /// Huellas por bloque.
    pub block_hashes: Vec<String>,
    /// Huella total.
    pub total_hash: String,
    /// Bits bajos a cero en todas las muestras.
    pub lsb_zero_bits: u32,
}

/// Escritor del master.
#[derive(Debug)]
pub struct MasterWriter {
    file: BufWriter<File>,
    path: PathBuf,
    format: SampleFormat,
    rate: u32,
    in_frame: usize,
    pair_offsets: [usize; 2],
    carry: Vec<u8>,
    out: Vec<u8>,
    data_bytes: u64,
    hasher: BlockHasher,
    lsb_or: u32,
    rf64_threshold: u64,
    final_path: PathBuf,
    confirmed_frames: u64,
}

impl MasterWriter {
    /// Empieza el master `path`: escribe en `path.partial` y lo renombra a `path` al cerrar. Nunca
    /// sobrescribe: falla si existe cualquiera de los dos. `channels` es el número de canales del
    /// flujo entrelazado y `pair` el par grabado, desde 0.
    pub fn create(
        path: &Path,
        format: SampleFormat,
        rate: u32,
        channels: u16,
        pair: [u16; 2],
    ) -> Result<Self, AudioError> {
        if pair[0] == pair[1] || pair.iter().any(|&c| c >= channels) {
            return Err(AudioError::Invalid(format!(
                "par de canales {}–{} no válido: la interfaz tiene {channels} canales",
                pair[0] + 1,
                pair[1] + 1
            )));
        }
        if path.exists() {
            return Err(AudioError::io(
                path,
                std::io::Error::new(std::io::ErrorKind::AlreadyExists, "el master ya existe"),
            ));
        }
        let final_path = path.to_path_buf();
        let path = &partial_path(path);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| AudioError::io(path, e))?;
        let bytes = format.bytes();
        let mut w = Self {
            // Búfer pequeño (≈ 80 ms a 96 kHz en 32 bits): el audio llega al archivo casi al
            // momento, así que tras un cierre de golpe se sabe cuánto se capturó después de la
            // última confirmación.
            file: BufWriter::with_capacity(WRITE_BUFFER, file),
            path: path.to_path_buf(),
            format,
            rate,
            in_frame: bytes * usize::from(channels),
            pair_offsets: [usize::from(pair[0]) * bytes, usize::from(pair[1]) * bytes],
            carry: Vec::new(),
            out: Vec::new(),
            data_bytes: 0,
            hasher: BlockHasher::new(HASH_BLOCK_BYTES),
            lsb_or: 0,
            rf64_threshold: u64::from(u32::MAX),
            final_path,
            confirmed_frames: 0,
        };
        let header = w.header();
        w.file
            .write_all(&header)
            .map_err(|e| AudioError::io(path, e))?;
        Ok(w)
    }

    /// Cambia el tamaño RIFF a partir del cual se escribe RF64 (solo para los tests).
    #[doc(hidden)]
    pub fn with_rf64_threshold(mut self, bytes: u64) -> Self {
        self.rf64_threshold = bytes;
        self
    }

    /// Frames escritos hasta ahora.
    pub fn frames(&self) -> u64 {
        self.data_bytes / (2 * self.format.bytes()) as u64
    }

    /// Añade bytes entrelazados del flujo completo (pueden cortar frames: el resto se guarda).
    pub fn write_interleaved(&mut self, mut bytes: &[u8]) -> Result<(), AudioError> {
        let fb = self.in_frame;
        let sb = self.format.bytes();
        self.out.clear();
        if !self.carry.is_empty() {
            let need = fb - self.carry.len();
            let take = need.min(bytes.len());
            self.carry.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.carry.len() == fb {
                let frame = std::mem::take(&mut self.carry);
                self.extract(&frame, sb);
            }
        }
        let whole = bytes.len() / fb * fb;
        let (frames, rest) = bytes.split_at(whole);
        for frame in frames.chunks_exact(fb) {
            self.extract(frame, sb);
        }
        self.carry.extend_from_slice(rest);
        if !self.out.is_empty() {
            let out = std::mem::take(&mut self.out);
            self.hasher.update(&out);
            self.file
                .write_all(&out)
                .map_err(|e| AudioError::io(&self.path, e))?;
            self.data_bytes += out.len() as u64;
            self.out = out;
        }
        Ok(())
    }

    fn extract(&mut self, frame: &[u8], sb: usize) {
        for &o in &self.pair_offsets {
            let s = &frame[o..o + sb];
            self.out.extend_from_slice(s);
            let mut le = [0u8; 4];
            le[..sb].copy_from_slice(s);
            self.lsb_or |= u32::from_le_bytes(le);
        }
    }

    fn is_rf64(&self) -> bool {
        self.data_bytes + RIFF_OVERHEAD > self.rf64_threshold
    }

    /// Ruta del archivo en curso (`.partial`).
    pub fn partial(&self) -> &Path {
        &self.path
    }

    /// Ruta final del master.
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// Frames confirmados en el disco en el último [`Self::sync_point`].
    pub fn confirmed_frames(&self) -> u64 {
        self.confirmed_frames
    }

    /// Punto de confirmación (T1.6): vuelca el búfer, reescribe la cabecera con el tamaño actual
    /// y hace `fdatasync`. Devuelve los frames que quedan confirmados en el disco. Solo desde el
    /// hilo escritor.
    pub fn sync_point(&mut self) -> Result<u64, AudioError> {
        self.refresh_header()?;
        self.file
            .get_ref()
            .sync_data()
            .map_err(|e| AudioError::io(&self.path, e))?;
        self.confirmed_frames = self.frames();
        Ok(self.confirmed_frames)
    }

    fn header(&self) -> [u8; HEADER_LEN as usize] {
        header_bytes(self.format, self.rate, self.data_bytes, self.rf64_threshold)
    }

    /// Reescribe la cabecera con los tamaños actuales.
    pub fn refresh_header(&mut self) -> Result<(), AudioError> {
        let h = self.header();
        let path = self.path.clone();
        let io = |e| AudioError::io(&path, e);
        self.file.flush().map_err(io)?;
        let f = self.file.get_mut();
        f.seek(SeekFrom::Start(0)).map_err(io)?;
        f.write_all(&h).map_err(io)?;
        f.seek(SeekFrom::End(0)).map_err(io)?;
        Ok(())
    }

    /// Cierra el master: cabecera definitiva, `fsync`, renombrado de `.partial` al nombre final
    /// (sin sobrescribir) y solo lectura. Un frame a medias al final (no debería pasar) se
    /// descarta.
    pub fn finalize(mut self) -> Result<MasterSummary, AudioError> {
        self.refresh_header()?;
        let partial = self.path.clone();
        let io = |e| AudioError::io(&partial, e);
        self.file.flush().map_err(io)?;
        self.file.get_ref().sync_all().map_err(io)?;
        let path = self.final_path.clone();
        rename_no_overwrite(&partial, &path).map_err(|e| AudioError::io(&path, e))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            // El renombrado también debe llegar al disco.
            let _ = File::open(dir).and_then(|d| d.sync_all());
        }
        digityzer_project::set_read_only(&path).map_err(|e| AudioError::io(&path, e))?;
        let rf64 = self.is_rf64();
        let frames = self.frames();
        let bits = u32::from(self.format.bits());
        let lsb_zero_bits = self.lsb_or.trailing_zeros().min(bits);
        let (block_hashes, total_hash) = self.hasher.finish();
        Ok(MasterSummary {
            path,
            frames,
            data_bytes: self.data_bytes,
            rf64,
            block_hashes,
            total_hash,
            lsb_zero_bits,
        })
    }
}

/// Cabecera leída de un master escrito por [`MasterWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    /// `true` si es RF64.
    pub rf64: bool,
    /// Canales.
    pub channels: u16,
    /// Frecuencia.
    pub rate: u32,
    /// Bits por muestra.
    pub bits: u16,
    /// Posición del audio.
    pub data_offset: u64,
    /// Bytes de audio.
    pub data_bytes: u64,
}

/// Lee la cabecera de un WAV o RF64 con la disposición de [`MasterWriter`].
pub fn read_header(r: &mut impl Read) -> std::io::Result<WavInfo> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
    let mut h = [0u8; HEADER_LEN as usize];
    r.read_exact(&mut h)?;
    let rf64 = match &h[0..4] {
        b"RIFF" => false,
        b"RF64" => true,
        _ => return Err(bad("no es un WAV")),
    };
    if &h[8..12] != b"WAVE" || &h[48..52] != b"fmt " || &h[96..100] != b"data" {
        return Err(bad("cabecera WAV con una disposición inesperada"));
    }
    if rf64 && &h[12..16] != b"ds64" {
        return Err(bad("RF64 sin ds64"));
    }
    let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    let data_bytes = if rf64 {
        u64::from_le_bytes(h[28..36].try_into().map_err(|_| bad("ds64"))?)
    } else {
        u64::from(u32_at(100))
    };
    Ok(WavInfo {
        rf64,
        channels: u16_at(58),
        rate: u32_at(60),
        bits: u16_at(70),
        data_offset: HEADER_LEN,
        data_bytes,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source::tests::{config, drain};
    use crate::source::{CaptureSource, SyntheticSource};

    /// Bytes del par `pair` (desde 0) de un flujo entrelazado.
    pub(crate) fn pair_bytes(all: &[u8], format: SampleFormat, ch: u16, pair: [u16; 2]) -> Vec<u8> {
        let sb = format.bytes();
        let fb = sb * usize::from(ch);
        all.chunks_exact(fb)
            .flat_map(|f| {
                pair.iter()
                    .flat_map(move |&c| f[usize::from(c) * sb..usize::from(c) * sb + sb].to_vec())
            })
            .collect()
    }

    #[test]
    fn huella_por_bloques_independiente_del_troceado() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut a = BlockHasher::new(1000);
        a.update(&data);
        let mut b = BlockHasher::new(1000);
        for c in data.chunks(333) {
            b.update(c);
        }
        let (ba, ta) = a.finish();
        let (bb, tb) = b.finish();
        assert_eq!((ba.len(), &ba, &ta), (10, &bb, &tb));
        assert_eq!(ta, blake3::hash(&data).to_hex().to_string());
        assert_eq!(ba[3], blake3::hash(&data[3000..4000]).to_hex().to_string());
    }

    #[test]
    fn el_par_elegido_byte_a_byte_y_legible_con_hound() {
        for format in SampleFormat::PREFERENCE {
            let c = config(format, 12);
            let src = SyntheticSource::new(c).with_total_frames(9_600);
            let all = drain(src);
            let dir = digityzer_testkit::temp_dir();
            let path = dir.path().join("m.wav");
            let mut w = MasterWriter::create(&path, format, 96_000, 12, [2, 3]).unwrap();
            // Trozos que cortan frames a propósito.
            for chunk in all.chunks(1001) {
                w.write_interleaved(chunk).unwrap();
            }
            let s = w.finalize().unwrap();
            let expected = pair_bytes(&all, format, 12, [2, 3]);
            assert_eq!(s.frames, 9_600);
            assert!(!s.rf64);
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(&bytes[HEADER_LEN as usize..], &expected[..], "{format}");
            assert_eq!(s.total_hash, blake3::hash(&expected).to_hex().to_string());
            assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
            assert_eq!(
                s.lsb_zero_bits,
                if format == SampleFormat::S32Le { 8 } else { 0 }
            );

            let mut r = hound::WavReader::open(&path).unwrap();
            let spec = r.spec();
            assert_eq!((spec.channels, spec.sample_rate), (2, 96_000));
            assert_eq!(spec.bits_per_sample, format.bits());
            let samples: Vec<i32> = r.samples::<i32>().map(Result::unwrap).collect();
            let decoded: Vec<i32> = expected
                .chunks_exact(format.bytes())
                .map(|b| format.decode(b))
                .collect();
            assert_eq!(samples, decoded, "{format}");
        }
    }

    #[test]
    fn rf64_por_encima_del_umbral() {
        let c = config(SampleFormat::S24_3Le, 2);
        let all = drain(SyntheticSource::new(c).with_total_frames(5_000));
        let dir = digityzer_testkit::temp_dir();
        let path = dir.path().join("grande.wav");
        let mut w = MasterWriter::create(&path, SampleFormat::S24_3Le, 96_000, 2, [0, 1])
            .unwrap()
            .with_rf64_threshold(10_000);
        w.write_interleaved(&all).unwrap();
        let s = w.finalize().unwrap();
        assert!(s.rf64);
        let mut f = File::open(&path).unwrap();
        let info = read_header(&mut f).unwrap();
        assert!(info.rf64);
        assert_eq!(info.data_bytes, 30_000);
        assert_eq!((info.channels, info.rate, info.bits), (2, 96_000, 24));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[4..8], &[0xFF; 4]);
        assert_eq!(&bytes[100..104], &[0xFF; 4]);
        assert_eq!(&bytes[HEADER_LEN as usize..], &all[..]);
    }

    #[test]
    fn nunca_sobrescribe_y_valida_el_par() {
        let dir = digityzer_testkit::temp_dir();
        let path = dir.path().join("x.wav");
        std::fs::write(&path, b"no tocar").unwrap();
        assert!(matches!(
            MasterWriter::create(&path, SampleFormat::S32Le, 96_000, 12, [2, 3]),
            Err(AudioError::Io { .. })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"no tocar");
        let p2 = dir.path().join("y.wav");
        assert!(matches!(
            MasterWriter::create(&p2, SampleFormat::S32Le, 96_000, 4, [3, 4]),
            Err(AudioError::Invalid(_))
        ));
        assert!(MasterWriter::create(&p2, SampleFormat::S32Le, 96_000, 4, [1, 1]).is_err());
        assert!(!p2.exists());
        let _ = SyntheticSource::new(config(SampleFormat::S16Le, 2)).config();
    }
}
