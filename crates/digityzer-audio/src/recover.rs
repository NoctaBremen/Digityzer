//! Recuperación de grabaciones cortadas de golpe (T1.6).
//!
//! Mientras se graba, el master es `<master>.partial` y su ficha de progreso
//! `<master>.partial.json` dice cuántos frames están confirmados en el disco (con `fdatasync`).
//! Si el programa muere (`kill -9`, corte de luz, cuelgue), al volver a abrirlo [`find_partials`]
//! los encuentra y [`recover`]:
//!
//! 1. corta el archivo en la última muestra confirmada (lo de después puede no haber llegado al
//!    disco entero),
//! 2. rehace la cabecera WAV/RF64 con ese tamaño y hace `fsync`,
//! 3. calcula la huella de lo que queda,
//! 4. lo renombra al nombre final sin sobrescribir nada (si ya existe, `… (recuperado).wav`),
//! 5. escribe la ficha del master marcada como interrumpida y recuperada, con una estimación de
//!    los segundos perdidos, y la deja en solo lectura junto con el master.
//!
//! Lo recuperado es idéntico bit a bit a lo capturado hasta la última confirmación: cada
//! [`crate::wav::SYNC_INTERVAL`] (2 s) como mucho se pierde.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use digityzer_project::{MasterInfo, PartialInfo, set_read_only, unix_now, utc_iso8601};

use crate::error::AudioError;
use crate::format::SampleFormat;
use crate::wav::{
    BlockHasher, HEADER_LEN, PARTIAL_SUFFIX, header_bytes, partial_path, rename_no_overwrite,
};

/// Resultado de recuperar un `.partial`.
#[derive(Debug, Clone)]
pub struct Recovered {
    /// Master final.
    pub master: PathBuf,
    /// Su ficha.
    pub json: PathBuf,
    /// La ficha escrita.
    pub info: MasterInfo,
}

/// Archivos `*.wav.partial` de una carpeta (no recursivo), ordenados.
pub fn find_partials(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(PARTIAL_SUFFIX) && !n.starts_with('.'))
        })
        .collect();
    v.sort();
    v
}

/// Nombre final libre para el master de `partial`: el suyo sin `.partial` o, si ya existe (o su
/// ficha), `<nombre> (recuperado).wav`, `(recuperado 2)`…
pub fn final_name(partial: &Path) -> PathBuf {
    let s = partial.to_string_lossy();
    let base = PathBuf::from(s.strip_suffix(PARTIAL_SUFFIX).unwrap_or(&s));
    let taken = |p: &Path| p.exists() || MasterInfo::path_for(p).exists();
    if !taken(&base) {
        return base;
    }
    let stem = base
        .file_stem()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = base
        .extension()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".into());
    let dir = base.parent().map(Path::to_path_buf).unwrap_or_default();
    (1..)
        .map(|n| {
            let suffix = if n == 1 {
                " (recuperado)".to_string()
            } else {
                format!(" (recuperado {n})")
            };
            dir.join(format!("{stem}{suffix}.{ext}"))
        })
        .find(|p| !taken(p))
        .unwrap_or(base)
}

/// Recupera un `.partial` (ver el módulo).
pub fn recover(partial: &Path) -> Result<Recovered, AudioError> {
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |e| AudioError::io(p, e)
    };
    let progress_path = PartialInfo::path_for(partial);
    let progress = PartialInfo::read(&progress_path)?;
    let format = SampleFormat::from_alsa_name(&progress.master.sample_format).ok_or_else(|| {
        AudioError::Invalid(format!(
            "formato desconocido en la ficha de progreso: {}",
            progress.master.sample_format
        ))
    })?;
    let block = u64::from(progress.block_align.max(1));
    let rate = progress.master.sample_rate.max(1);

    let meta = fs::metadata(partial).map_err(io(partial))?;
    let on_disk = meta.len().saturating_sub(HEADER_LEN);
    // Lo confirmado está entero en el disco; si el archivo fuese más corto (no debería), lo que
    // haya en frames completos.
    let data_bytes = (progress.confirmed_frames * block).min(on_disk / block * block);
    let frames = data_bytes / block;
    let bytes_per_s = f64::from(rate) * block as f64;
    // Lo perdido: lo escrito tras la confirmación que se descarta, o el tiempo entre la
    // confirmación y la última escritura del archivo, lo que sea mayor.
    let tail_s = (on_disk - data_bytes) as f64 / bytes_per_s;
    let mtime_gap = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| {
            (d.as_secs_f64() - progress.confirmed_unix).max(0.0)
        });
    let seconds_lost = tail_s.max(mtime_gap);

    {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(partial)
            .map_err(io(partial))?;
        f.set_len(HEADER_LEN + data_bytes).map_err(io(partial))?;
        f.seek(SeekFrom::Start(0)).map_err(io(partial))?;
        f.write_all(&header_bytes(format, rate, data_bytes, u64::from(u32::MAX)))
            .map_err(io(partial))?;
        f.sync_all().map_err(io(partial))?;
    }

    // Huella y bits bajos de lo que queda.
    let mut hasher = BlockHasher::new(progress.master.hash.block_bytes.max(1));
    let mut lsb_or = 0u32;
    {
        let mut f = File::open(partial).map_err(io(partial))?;
        f.seek(SeekFrom::Start(HEADER_LEN)).map_err(io(partial))?;
        let mut left = data_bytes;
        let mut buf = vec![0u8; 1 << 20];
        let sb = format.bytes();
        while left > 0 {
            let n = (buf.len() as u64).min(left) as usize;
            f.read_exact(&mut buf[..n]).map_err(io(partial))?;
            hasher.update(&buf[..n]);
            for s in buf[..n].chunks_exact(sb) {
                let mut le = [0u8; 4];
                le[..sb].copy_from_slice(s);
                lsb_or |= u32::from_le_bytes(le);
            }
            left -= n as u64;
        }
    }
    let (blocks, total) = hasher.finish();

    let master = final_name(partial);
    rename_no_overwrite(partial, &master).map_err(io(&master))?;
    let json = MasterInfo::path_for(&master);
    let mut info = progress.master.clone();
    info.frames = frames;
    info.duration_s = frames as f64 / f64::from(rate);
    info.data_bytes = data_bytes;
    info.container = if data_bytes + HEADER_LEN - 8 > u64::from(u32::MAX) {
        "RF64".into()
    } else {
        "WAV".into()
    };
    info.lsb_zero_bits = lsb_or.trailing_zeros().min(u32::from(format.bits()));
    info.finished_at = utc_iso8601(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(progress.confirmed_unix),
    );
    info.xruns.retain(|x| x.frame <= frames);
    info.hash.blocks = blocks;
    info.hash.total = total;
    info.interrupted = true;
    info.interruption = Some(format!(
        "cierre inesperado; recuperado el {} hasta la última confirmación",
        utc_iso8601(std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(unix_now()))
    ));
    info.recovered = true;
    info.seconds_lost = Some((seconds_lost * 1000.0).round() / 1000.0);
    info.write_new(&json)?;
    set_read_only(&master).map_err(io(&master))?;
    let _ = fs::remove_file(&progress_path);
    Ok(Recovered { master, json, info })
}

/// `true` si `path` es el archivo en curso de algún master (para no ofrecerlo como master).
pub fn is_partial(path: &Path) -> bool {
    path.to_string_lossy().ends_with(PARTIAL_SUFFIX)
}

/// Ruta del `.partial` de un master (reexportada por comodidad).
pub fn partial_of(master: &Path) -> PathBuf {
    partial_path(master)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SyntheticSource;
    use crate::source::tests::{config, drain};
    use crate::wav::MasterWriter;
    use crate::wav::tests::pair_bytes;

    fn progress_for(w: &MasterWriter, format: SampleFormat) -> PartialInfo {
        let json = format!(
            r#"{{"version":3,"digityzer_version":"t","device":"sintético","card_name":"x",
            "sample_rate":96000,"sample_format":"{}","container_bits":{},"lsb_zero_bits":0,
            "channels_opened":12,"channel_pair":[3,4],"container":"WAV","frames":0,
            "duration_s":0.0,"data_bytes":0,"started_at":"","finished_at":"","realtime":false,
            "xruns":[],"frames_lost_ring":0,"hash":{{"algorithm":"blake3","block_bytes":1048576,
            "blocks":[],"total":""}},"notes":"prueba"}}"#,
            format.alsa_name(),
            format.bits()
        );
        PartialInfo {
            master: serde_json::from_str(&json).unwrap(),
            block_align: 2 * format.bytes() as u16,
            confirmed_frames: w.confirmed_frames(),
            confirmed_unix: unix_now(),
        }
    }

    /// Simula un `kill -9`: se confirma a mitad, se sigue escribiendo y no se cierra nunca.
    #[test]
    fn corte_en_la_ultima_confirmacion_bit_a_bit() {
        let format = SampleFormat::S32Le;
        let all = drain(SyntheticSource::new(config(format, 12)).with_total_frames(48_000));
        let expected = pair_bytes(&all, format, 12, [2, 3]);
        let dir = digityzer_testkit::temp_dir();
        let master = dir.path().join("cara-a.wav");
        let mut w = MasterWriter::create(&master, format, 96_000, 12, [2, 3]).unwrap();
        let fb = 12 * 4;
        w.write_interleaved(&all[..30_000 * fb]).unwrap();
        assert_eq!(w.sync_point().unwrap(), 30_000);
        let p = progress_for(&w, format);
        p.save(&PartialInfo::path_for(w.partial())).unwrap();
        // Más audio tras la confirmación, que se pierde.
        w.write_interleaved(&all[30_000 * fb..]).unwrap();
        w.refresh_header().unwrap();
        std::mem::forget(w); // el proceso «muere»: ni finalize ni Drop
        let partial = partial_path(&master);
        assert_eq!(find_partials(dir.path()), vec![partial.clone()]);

        let r = recover(&partial).unwrap();
        assert_eq!(r.master, master);
        assert!(!partial.exists());
        assert!(!PartialInfo::path_for(&partial).exists());
        let bytes = fs::read(&master).unwrap();
        assert_eq!(&bytes[HEADER_LEN as usize..], &expected[..30_000 * 8]);
        assert_eq!(r.info.frames, 30_000);
        assert!(r.info.recovered && r.info.interrupted);
        assert!(r.info.seconds_lost.unwrap() >= 18_000.0 / 96_000.0 - 1e-6);
        assert_eq!(r.info.lsb_zero_bits, 8);
        assert!(crate::verify_master(&master).unwrap().ok());
        // Legible con hound: la cabecera está rehecha.
        let rd = hound::WavReader::open(&master).unwrap();
        assert_eq!(rd.duration(), 30_000);
        assert!(fs::metadata(&master).unwrap().permissions().readonly());
    }

    #[test]
    fn nombre_final_sin_sobrescribir() {
        let dir = digityzer_testkit::temp_dir();
        let partial = dir.path().join("x.wav.partial");
        assert_eq!(final_name(&partial), dir.path().join("x.wav"));
        fs::write(dir.path().join("x.wav"), b"ya").unwrap();
        assert_eq!(final_name(&partial), dir.path().join("x (recuperado).wav"));
        fs::write(dir.path().join("x (recuperado).wav.json"), b"{}").unwrap();
        assert_eq!(
            final_name(&partial),
            dir.path().join("x (recuperado 2).wav")
        );
        assert!(is_partial(&partial));
        assert_eq!(partial_of(&dir.path().join("x.wav")), partial);
    }

    #[test]
    fn sin_ficha_de_progreso_es_un_error_y_no_toca_nada() {
        let dir = digityzer_testkit::temp_dir();
        let partial = dir.path().join("y.wav.partial");
        fs::write(&partial, vec![0u8; 200]).unwrap();
        assert!(recover(&partial).is_err());
        assert_eq!(fs::read(&partial).unwrap().len(), 200);
    }
}
