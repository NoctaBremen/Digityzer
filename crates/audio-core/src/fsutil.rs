//! Utilidades de archivos: escritura atómica y copias de seguridad de archivos corruptos.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Prefijo de los temporales (en Spector, `scan::TEMP_PREFIX` = `.spector-tmp-`).
pub const TEMP_PREFIX: &str = ".digityzer-tmp-";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ruta temporal en el mismo directorio que `dest`: `.<nombre>.digityzer-tmp-<pid>-<n>`.
pub fn temp_path_for(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = format!(".{name}{TEMP_PREFIX}{}-{n}", std::process::id());
    dest.with_file_name(tmp)
}

/// Escribe `bytes` en `dest` de forma atómica: temporal en el mismo directorio → `fsync` →
/// rename (sustituye el archivo si existe) → `fsync` del directorio. Crea el directorio si no
/// existe. El temporal se borra si algo falla.
///
/// Para archivos de la aplicación (ajustes, biblioteca). La conversión de audio usa su propio
/// rename **sin sobrescribir** (`spector-convert`).
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = temp_path_for(dest);
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dest)?;
        // El fsync del directorio hace duradero el rename; si el sistema de archivos no lo
        // admite, no es un error.
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Aparta un archivo corrupto como `<nombre>.bak-<segundos unix>` y devuelve la ruta nueva.
pub fn backup_corrupt(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut bak = path.with_file_name(format!("{name}.bak-{secs}"));
    let mut n = 1;
    while bak.exists() {
        bak = path.with_file_name(format!("{name}.bak-{secs}-{n}"));
        n += 1;
    }
    fs::rename(path, &bak)?;
    Ok(bak)
}

/// Comprueba que se puede escribir en `dir`: lo crea si no existe, escribe un archivo de prueba
/// y lo borra (E/S: llamar desde un trabajador).
pub fn check_writable_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    if !fs::metadata(dir)?.is_dir() {
        return Err(io::Error::other("no es una carpeta"));
    }
    let probe = temp_path_for(&dir.join("prueba"));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?
        .write_all(b"digityzer")?;
    fs::remove_file(&probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escritura_atomica_y_copia() {
        let dir = digityzer_testkit::temp_dir();
        let p = dir.path().join("sub/a.txt");
        write_atomic(&p, b"uno").unwrap();
        write_atomic(&p, b"dos").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"dos");
        let names: Vec<_> = fs::read_dir(p.parent().unwrap()).unwrap().collect();
        assert_eq!(names.len(), 1, "no quedan temporales");
        let bak = backup_corrupt(&p).unwrap();
        assert!(!p.exists());
        assert!(bak.to_string_lossy().contains("a.txt.bak-"));
        check_writable_dir(&dir.path().join("nueva/sub")).unwrap();
        assert_eq!(
            fs::read_dir(dir.path().join("nueva/sub")).unwrap().count(),
            0
        );
        assert!(
            check_writable_dir(&bak).is_err(),
            "un archivo no es una carpeta"
        );
        let t = temp_path_for(&p);
        assert!(
            t.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".a.txt")
        );
    }
}
