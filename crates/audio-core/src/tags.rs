//! Etiquetas: se leen del original con lofty y se escriben como ID3v2 en el chunk `ID3 ` del
//! AIFF de destino (D9, P4).
//!
//! **Decisión (riesgo R4 del plan)**: lofty no escribe en el AIFF. Construimos un `Id3v2Tag`, lofty
//! lo serializa en memoria (`dump_to`, con `use_id3v23` para v2.3) y el chunk `ID3 ` lo añade
//! [`append_id3_chunk`] al final del archivo, parcheando solo el tamaño de `FORM`. Así `COMM` y
//! `SSND` no se tocan nunca y el identificador del chunk es siempre `ID3 `.

use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use lofty::TextEncoding;
use lofty::config::{ParseOptions, ParsingMode, WriteOptions};
use lofty::id3::v2::{
    AttachedPictureFrame, CommentFrame, ExtendedTextFrame, Frame, FrameId, Id3v2Tag,
    TextInformationFrame,
};
use lofty::picture::{MimeType, PictureType};
use lofty::prelude::{ItemKey, TagExt, TaggedFileExt};
use lofty::tag::{Tag, TagType};

use crate::error::TagError;

/// Una imagen (carátula u otra), con sus **bytes originales**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Tipo MIME (`image/jpeg`…), si se conoce.
    pub mime: Option<String>,
    /// `true` si es la portada.
    pub front_cover: bool,
    /// Descripción.
    pub description: Option<String>,
    /// Bytes de la imagen, tal cual.
    pub data: Vec<u8>,
}

/// Etiquetas del original, ya normalizadas.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceTags {
    /// Título.
    pub title: Option<String>,
    /// Artista.
    pub artist: Option<String>,
    /// Álbum.
    pub album: Option<String>,
    /// Año o fecha (texto original).
    pub year: Option<String>,
    /// Género.
    pub genre: Option<String>,
    /// BPM (texto original, puede ser decimal).
    pub bpm: Option<String>,
    /// Tonalidad (Camelot "8A" o musical "Am", tal cual).
    pub key: Option<String>,
    /// Comentario.
    pub comment: Option<String>,
    /// Artista del álbum.
    pub album_artist: Option<String>,
    /// Número de pista.
    pub track_number: Option<String>,
    /// Total de pistas.
    pub track_total: Option<String>,
    /// Número de disco.
    pub disc_number: Option<String>,
    /// Total de discos.
    pub disc_total: Option<String>,
    /// Compositor.
    pub composer: Option<String>,
    /// Sello.
    pub label: Option<String>,
    /// ISRC.
    pub isrc: Option<String>,
    /// Remezclador.
    pub remixer: Option<String>,
    /// Imágenes.
    pub pictures: Vec<Picture>,
    /// Campos sin equivalente directo (irán como `TXXX`).
    pub extra: Vec<(String, String)>,
    /// Problemas al leer (etiquetas dañadas…): no impiden convertir.
    pub warnings: Vec<String>,
}

impl SourceTags {
    /// `true` si no hay nada que escribir.
    pub fn is_empty(&self) -> bool {
        self.text_fields().iter().all(|(_, v)| v.is_none())
            && self.pictures.is_empty()
            && self.extra.is_empty()
    }

    fn text_fields(&self) -> [(&'static str, &Option<String>); 17] {
        [
            ("title", &self.title),
            ("artist", &self.artist),
            ("album", &self.album),
            ("year", &self.year),
            ("genre", &self.genre),
            ("bpm", &self.bpm),
            ("key", &self.key),
            ("comment", &self.comment),
            ("album_artist", &self.album_artist),
            ("track_number", &self.track_number),
            ("track_total", &self.track_total),
            ("disc_number", &self.disc_number),
            ("disc_total", &self.disc_total),
            ("composer", &self.composer),
            ("label", &self.label),
            ("isrc", &self.isrc),
            ("remixer", &self.remixer),
        ]
    }
}

/// Opciones de escritura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagOptions {
    /// ID3v2.3 (por defecto) en lugar de v2.4.
    pub id3v23: bool,
}

impl Default for TagOptions {
    fn default() -> Self {
        Self { id3v23: true }
    }
}

/// Qué se escribió y qué se perdió.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagReport {
    /// Frames escritos (`TIT2`, `APIC`…).
    pub written: Vec<&'static str>,
    /// Lo que no se pudo copiar, con el motivo.
    pub dropped: Vec<String>,
}

/// Prioridad de los tipos de etiqueta al combinar (la primera que tenga el campo gana). En un WAV
/// con `id3 ` e INFO manda ID3.
const PRIORITY: [TagType; 7] = [
    TagType::Id3v2,
    TagType::VorbisComments,
    TagType::Mp4Ilst,
    TagType::Ape,
    TagType::RiffInfo,
    TagType::AiffText,
    TagType::Id3v1,
];

/// Campos conocidos por lofty que no tienen frame propio en la tabla del plan: van como `TXXX`.
const EXTRA_KEYS: [(ItemKey, &str); 12] = [
    (ItemKey::CatalogNumber, "CATALOGNUMBER"),
    (ItemKey::Mood, "MOOD"),
    (ItemKey::Lyricist, "LYRICIST"),
    (ItemKey::Conductor, "CONDUCTOR"),
    (ItemKey::Producer, "PRODUCER"),
    (ItemKey::MixDj, "DJMIXER"),
    (ItemKey::Work, "WORK"),
    (ItemKey::ReleaseCountry, "RELEASECOUNTRY"),
    (ItemKey::CopyrightMessage, "COPYRIGHT"),
    (ItemKey::EncodedBy, "ENCODEDBY"),
    (ItemKey::ReplayGainTrackGain, "REPLAYGAIN_TRACK_GAIN"),
    (ItemKey::ReplayGainAlbumGain, "REPLAYGAIN_ALBUM_GAIN"),
];

/// Primer valor de texto no vacío de `keys`, buscando las etiquetas por orden de prioridad.
fn first_text(tags: &[&Tag], keys: &[ItemKey]) -> Option<String> {
    tags.iter().find_map(|t| {
        keys.iter().find_map(|k| {
            t.get_string(*k)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
    })
}

/// Lee las etiquetas del original (abierto en solo lectura). Si no se pueden leer, devuelve
/// etiquetas vacías con un aviso: las etiquetas nunca impiden convertir.
pub fn read_source_tags(path: &Path) -> Result<SourceTags, TagError> {
    let opts = ParseOptions::new()
        .read_properties(false)
        .parsing_mode(ParsingMode::BestAttempt);
    let tagged = match lofty::probe::Probe::open(path)
        .map_err(|e| e.to_string())
        .and_then(|p| p.options(opts).read().map_err(|e| e.to_string()))
    {
        Ok(t) => t,
        Err(e) => {
            return Ok(SourceTags {
                warnings: vec![format!("no se pudieron leer las etiquetas: {e}")],
                ..Default::default()
            });
        }
    };
    let mut ordered: Vec<&Tag> = Vec::new();
    for ty in PRIORITY {
        ordered.extend(tagged.tags().iter().filter(|t| t.tag_type() == ty));
    }
    ordered.extend(
        tagged
            .tags()
            .iter()
            .filter(|t| !PRIORITY.contains(&t.tag_type())),
    );
    Ok(tags_from(&ordered))
}

fn tags_from(tags: &[&Tag]) -> SourceTags {
    use ItemKey as K;
    let mut s = SourceTags {
        title: first_text(tags, &[K::TrackTitle]),
        artist: first_text(tags, &[K::TrackArtist]),
        album: first_text(tags, &[K::AlbumTitle]),
        year: first_text(tags, &[K::RecordingDate, K::Year, K::OriginalReleaseDate]),
        genre: first_text(tags, &[K::Genre]),
        bpm: first_text(tags, &[K::Bpm, K::IntegerBpm]),
        key: first_text(tags, &[K::InitialKey]),
        comment: first_text(tags, &[K::Comment]),
        album_artist: first_text(tags, &[K::AlbumArtist]),
        track_number: first_text(tags, &[K::TrackNumber]),
        track_total: first_text(tags, &[K::TrackTotal]),
        disc_number: first_text(tags, &[K::DiscNumber]),
        disc_total: first_text(tags, &[K::DiscTotal]),
        composer: first_text(tags, &[K::Composer]),
        label: first_text(tags, &[K::Label, K::Publisher]),
        isrc: first_text(tags, &[K::Isrc]),
        remixer: first_text(tags, &[K::Remixer]),
        ..Default::default()
    };
    // Imágenes: las de la etiqueta de más prioridad que tenga alguna.
    if let Some(t) = tags.iter().find(|t| !t.pictures().is_empty()) {
        s.pictures = t
            .pictures()
            .iter()
            .map(|p| Picture {
                mime: p.mime_type().map(|m| m.as_str().to_string()),
                front_cover: p.pic_type() == PictureType::CoverFront,
                description: p
                    .description()
                    .map(str::to_string)
                    .filter(|d| !d.is_empty()),
                data: p.data().to_vec(),
            })
            .collect();
    }
    // Otros campos útiles sin frame propio en nuestra tabla → TXXX con un nombre estable.
    // (lofty 0.25 no expone en la etiqueta genérica las claves que no conoce.)
    for (key, name) in EXTRA_KEYS {
        if let Some(v) = first_text(tags, &[key]) {
            s.extra.push((name.to_string(), v));
        }
    }
    s
}

/// BPM para v2.3: `TBPM` debe ser entero. Devuelve (valor de `TBPM`, ¿había que redondear?).
pub fn bpm_for_v23(bpm: &str) -> Option<(String, bool)> {
    let t = bpm.trim().replace(',', ".");
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        return Some((t.trim_start_matches('0').to_string().max("0".into()), false));
    }
    // Decimal: se redondea al entero más cercano (sin pasar por float: solo dígitos).
    let (int, frac) = t.split_once('.')?;
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut n: u64 = int.parse().ok()?;
    if frac.as_bytes().first().is_some_and(|&d| d >= b'5') {
        n += 1;
    }
    Some((n.to_string(), true))
}

/// Año de 4 cifras al principio de una fecha ("2024", "2024-05-01"…).
fn leading_year(s: &str) -> Option<&str> {
    let y = s.trim().get(..4)?;
    y.bytes().all(|b| b.is_ascii_digit()).then_some(y)
}

fn n_of(n: &Option<String>, total: &Option<String>) -> Option<String> {
    match (n, total) {
        (Some(n), Some(t)) => Some(format!("{n}/{t}")),
        (Some(n), None) => Some(n.clone()),
        (None, Some(t)) => Some(format!("0/{t}")),
        (None, None) => None,
    }
}

/// Construye la etiqueta ID3v2 serializada (cabecera ID3 + frames). Vacía si no hay etiquetas.
pub fn build_id3(tags: &SourceTags, opts: &TagOptions) -> Result<(Vec<u8>, TagReport), TagError> {
    let mut report = TagReport::default();
    if tags.is_empty() {
        return Ok((Vec::new(), report));
    }
    let enc = if opts.id3v23 {
        TextEncoding::UTF16
    } else {
        TextEncoding::UTF8
    };
    let mut id3 = Id3v2Tag::new();
    let text = |id3: &mut Id3v2Tag, report: &mut TagReport, id: &'static str, v: &str| {
        id3.insert(Frame::Text(TextInformationFrame::new(
            FrameId::Valid(Cow::Borrowed(id)),
            enc,
            v.to_string(),
        )));
        report.written.push(id);
    };
    let user = |id3: &mut Id3v2Tag, report: &mut TagReport, desc: &str, v: &str| {
        id3.insert(Frame::UserText(ExtendedTextFrame::new(
            enc,
            desc.to_string(),
            v.to_string(),
        )));
        if !report.written.contains(&"TXXX") {
            report.written.push("TXXX");
        }
    };
    let simple: [(&'static str, &Option<String>); 10] = [
        ("TIT2", &tags.title),
        ("TPE1", &tags.artist),
        ("TALB", &tags.album),
        ("TCON", &tags.genre),
        ("TKEY", &tags.key),
        ("TPE2", &tags.album_artist),
        ("TCOM", &tags.composer),
        ("TPUB", &tags.label),
        ("TSRC", &tags.isrc),
        ("TPE4", &tags.remixer),
    ];
    for (id, v) in simple {
        if let Some(v) = v {
            text(&mut id3, &mut report, id, v);
        }
    }
    if let Some(y) = &tags.year {
        if opts.id3v23 {
            match leading_year(y) {
                Some(year) => {
                    text(&mut id3, &mut report, "TYER", year);
                    if y.trim() != year {
                        user(&mut id3, &mut report, "DATE", y.trim());
                    }
                }
                None => user(&mut id3, &mut report, "DATE", y.trim()),
            }
        } else {
            text(&mut id3, &mut report, "TDRC", y.trim());
        }
    }
    if let Some(b) = &tags.bpm {
        if opts.id3v23 {
            match bpm_for_v23(b) {
                Some((int, rounded)) => {
                    text(&mut id3, &mut report, "TBPM", &int);
                    if rounded {
                        user(&mut id3, &mut report, "BPM", b.trim());
                    }
                }
                None => {
                    user(&mut id3, &mut report, "BPM", b.trim());
                    report
                        .dropped
                        .push(format!("BPM «{b}» no numérico: copiado en TXXX:BPM"));
                }
            }
        } else {
            text(&mut id3, &mut report, "TBPM", b.trim());
        }
    }
    if let Some(t) = n_of(&tags.track_number, &tags.track_total) {
        text(&mut id3, &mut report, "TRCK", &t);
    }
    if let Some(d) = n_of(&tags.disc_number, &tags.disc_total) {
        text(&mut id3, &mut report, "TPOS", &d);
    }
    if let Some(c) = &tags.comment {
        id3.insert(Frame::Comment(CommentFrame::new(
            enc,
            *b"eng",
            String::new(),
            c.clone(),
        )));
        report.written.push("COMM");
    }
    for (k, v) in &tags.extra {
        user(&mut id3, &mut report, k, v);
    }
    for (i, p) in tags.pictures.iter().enumerate() {
        let mut b = lofty::picture::Picture::unchecked(p.data.clone()).pic_type(if p.front_cover {
            PictureType::CoverFront
        } else {
            PictureType::Other
        });
        if let Some(m) = &p.mime {
            b = b.mime_type(MimeType::from_str(m));
        }
        // Dos imágenes del mismo tipo necesitan descripciones distintas en ID3.
        let desc = match &p.description {
            Some(d) => d.clone(),
            None if i == 0 => String::new(),
            None => format!("{}", i + 1),
        };
        b = b.description(desc);
        let before = id3.len();
        id3.insert(Frame::Picture(AttachedPictureFrame::new(enc, b.build())));
        if id3.len() == before {
            report.dropped.push(format!(
                "imagen {} duplicada (mismo tipo y descripción)",
                i + 1
            ));
        }
        if !report.written.contains(&"APIC") {
            report.written.push("APIC");
        }
    }
    let mut wo = WriteOptions::default().preferred_padding(0);
    let wo = wo.use_id3v23(opts.id3v23);
    let mut bytes = Vec::new();
    id3.dump_to(&mut bytes, wo)
        .map_err(|e| TagError::Tags(format!("no se pudo serializar el ID3: {e}")))?;
    Ok((bytes, report))
}

/// Añade un chunk `ID3 ` con `data` al final de un AIFF y actualiza el tamaño de `FORM`. No toca
/// ningún otro byte. Error si el archivo ya tiene un chunk ID3.
pub fn append_id3_chunk(aiff_path: &Path, data: &[u8]) -> Result<(), TagError> {
    let mut f = OpenOptions::new().read(true).write(true).open(aiff_path)?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head)?;
    if &head[..4] != b"FORM" || !(&head[8..] == b"AIFF" || &head[8..] == b"AIFC") {
        return Err(TagError::Tags("el destino no es un AIFF".into()));
    }
    // Recorre los chunks para comprobar que no hay ya un ID3.
    let len = f.seek(SeekFrom::End(0))?;
    let mut pos = 12u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos))?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch)?;
        if &ch[..4] == b"ID3 " || &ch[..4] == b"id3 " {
            return Err(TagError::Tags("el AIFF ya tiene etiquetas ID3".into()));
        }
        let size = u64::from(u32::from_be_bytes([ch[4], ch[5], ch[6], ch[7]]));
        pos += 8 + size + size % 2;
    }
    if pos != len {
        return Err(TagError::Tags(
            "los chunks del AIFF no llegan exactamente al final del archivo".into(),
        ));
    }
    let size =
        u32::try_from(data.len()).map_err(|_| TagError::Tags("ID3 demasiado grande".into()))?;
    let new_len = len + 8 + u64::from(size) + u64::from(size % 2);
    let form =
        u32::try_from(new_len - 8).map_err(|_| TagError::Tags("el AIFF superaría 4 GiB".into()))?;
    f.seek(SeekFrom::Start(len))?;
    f.write_all(b"ID3 ")?;
    f.write_all(&size.to_be_bytes())?;
    f.write_all(data)?;
    if size % 2 == 1 {
        f.write_all(&[0])?;
    }
    f.seek(SeekFrom::Start(4))?;
    f.write_all(&form.to_be_bytes())?;
    f.sync_all()?;
    Ok(())
}

/// Escribe `tags` como ID3v2 en el chunk `ID3 ` del AIFF. Sin etiquetas no se escribe nada.
pub fn write_aiff_id3(
    aiff_path: &Path,
    tags: &SourceTags,
    opts: &TagOptions,
) -> Result<TagReport, TagError> {
    let (bytes, mut report) = build_id3(tags, opts)?;
    report.dropped.extend(tags.warnings.iter().cloned());
    if !bytes.is_empty() {
        append_id3_chunk(aiff_path, &bytes)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_entero_y_decimal() {
        assert_eq!(bpm_for_v23("128"), Some(("128".into(), false)));
        assert_eq!(bpm_for_v23("0128"), Some(("128".into(), false)));
        assert_eq!(bpm_for_v23("127.5"), Some(("128".into(), true)));
        assert_eq!(bpm_for_v23("127,49"), Some(("127".into(), true)));
        assert_eq!(bpm_for_v23("rápido"), None);
        assert_eq!(bpm_for_v23("1.2.3"), None);
        assert_eq!(leading_year("2024-05-01"), Some("2024"));
        assert_eq!(leading_year("May 2024"), None);
        assert_eq!(n_of(&Some("3".into()), &Some("12".into())).unwrap(), "3/12");
    }

    #[test]
    fn sin_etiquetas_no_hay_bytes() {
        let (b, r) = build_id3(&SourceTags::default(), &TagOptions::default()).unwrap();
        assert!(b.is_empty());
        assert!(r.written.is_empty() && r.dropped.is_empty());
    }
}
