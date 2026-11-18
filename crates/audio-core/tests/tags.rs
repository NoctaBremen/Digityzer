//! Etiquetas ID3 en el chunk `ID3 ` del AIFF.
//!
//! En Spector (`spector-convert/tests/tags.rs`) las etiquetas se leían de un FLAC/WAV/ALAC de
//! origen. En Digityzer los metadatos se escriben a mano (T5.3), así que estos tests construyen
//! `SourceTags` directamente y comprueban lo mismo: las etiquetas se releen con lofty, el PCM no
//! cambia y un AIFF con ID3 no admite un segundo chunk.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::borrow::Cow;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use audio_core::aiff::{AiffReader, AiffSpec, AiffWriter};
use audio_core::tags::{SourceTags, TagOptions, read_source_tags, write_aiff_id3};
use digityzer_testkit::{quantize, sine, temp_dir};
use lofty::config::ParseOptions;
use lofty::file::AudioFile;
use lofty::id3::v2::{FrameId, Id3v2Tag};
use lofty::iff::aiff::AiffFile;

const TITLE: &str = "Canción de prueba — ñandú 日本語";

fn pcm() -> Vec<i32> {
    quantize(&sine(1000.0, 96_000, 0.3, 0.5), 24)
        .into_iter()
        .flat_map(|s| [s, -s])
        .collect()
}

fn write_aiff(path: &Path) {
    let spec = AiffSpec {
        channels: 2,
        sample_rate: 96_000,
        bits: 24,
    };
    let mut w = AiffWriter::new(BufWriter::new(File::create(path).unwrap()), spec).unwrap();
    w.write_frames_i32(&pcm()).unwrap();
    w.finalize().unwrap();
}

fn vinyl_tags() -> SourceTags {
    SourceTags {
        title: Some(TITLE.into()),
        artist: Some("Artista Ñ".into()),
        album: Some("Álbum en vinilo".into()),
        year: Some("1994".into()),
        label: Some("Sello Records".into()),
        track_number: Some("2".into()),
        track_total: Some("4".into()),
        disc_number: Some("1".into()),
        comment: Some("Cara A".into()),
        bpm: Some("127.5".into()),
        extra: vec![("CATALOGNUMBER".into(), "CAT-001".into())],
        ..Default::default()
    }
}

fn read_id3(aiff: &Path) -> Id3v2Tag {
    let f = AiffFile::read_from(&mut File::open(aiff).unwrap(), ParseOptions::new()).unwrap();
    f.id3v2().cloned().expect("sin ID3")
}

fn text(t: &Id3v2Tag, id: &'static str) -> Option<String> {
    t.get_text(&FrameId::Valid(Cow::Borrowed(id)))
        .map(str::to_string)
}

fn id3_major(aiff: &Path) -> u8 {
    let b = std::fs::read(aiff).unwrap();
    let i = b.windows(4).position(|w| w == b"ID3 ").unwrap();
    assert_eq!(&b[i + 8..i + 11], b"ID3");
    b[i + 11]
}

#[test]
fn etiquetas_de_vinilo_en_id3v23_sin_tocar_el_pcm() {
    let dir = temp_dir();
    let aiff = dir.path().join("a.aiff");
    write_aiff(&aiff);
    let report = write_aiff_id3(&aiff, &vinyl_tags(), &TagOptions::default()).unwrap();
    assert!(report.written.contains(&"TIT2"), "{report:?}");

    assert_eq!(id3_major(&aiff), 3);
    let id3 = read_id3(&aiff);
    assert_eq!(text(&id3, "TIT2").as_deref(), Some(TITLE));
    assert_eq!(text(&id3, "TPE1").as_deref(), Some("Artista Ñ"));
    assert_eq!(text(&id3, "TPUB").as_deref(), Some("Sello Records"));
    assert_eq!(text(&id3, "TRCK").as_deref(), Some("2/4"));
    // v2.3: TBPM entero.
    assert_eq!(text(&id3, "TBPM").as_deref(), Some("128"));

    let mut r = AiffReader::open(File::open(&aiff).unwrap()).unwrap();
    assert_eq!(r.spec().bits, 24);
    assert_eq!(r.read_all_i32().unwrap(), pcm());

    // Lo que se escribe se puede volver a leer como origen.
    let back = read_source_tags(&aiff).unwrap();
    assert_eq!(back.title.as_deref(), Some(TITLE));
    assert_eq!(back.label.as_deref(), Some("Sello Records"));
}

#[test]
fn id3v24_conserva_el_bpm_decimal() {
    let dir = temp_dir();
    let aiff = dir.path().join("b.aiff");
    write_aiff(&aiff);
    write_aiff_id3(&aiff, &vinyl_tags(), &TagOptions { id3v23: false }).unwrap();
    assert_eq!(id3_major(&aiff), 4);
    assert_eq!(text(&read_id3(&aiff), "TBPM").as_deref(), Some("127.5"));
}

#[test]
fn sin_etiquetas_no_se_escribe_nada_y_no_hay_segundo_chunk() {
    let dir = temp_dir();
    let aiff = dir.path().join("c.aiff");
    write_aiff(&aiff);
    let before = std::fs::read(&aiff).unwrap();
    let report = write_aiff_id3(&aiff, &SourceTags::default(), &TagOptions::default()).unwrap();
    assert!(report.written.is_empty());
    assert_eq!(std::fs::read(&aiff).unwrap(), before);

    write_aiff_id3(&aiff, &vinyl_tags(), &TagOptions::default()).unwrap();
    assert!(write_aiff_id3(&aiff, &vinyl_tags(), &TagOptions::default()).is_err());
}
