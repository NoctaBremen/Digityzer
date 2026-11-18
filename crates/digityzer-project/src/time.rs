//! Fechas UTC en ISO 8601 sin dependencias (para el JSON del master).

use std::time::{SystemTime, UNIX_EPOCH};

/// Formatea un instante como `AAAA-MM-DDTHH:MM:SSZ` (UTC, sin fracciones). Los instantes
/// anteriores a 1970 se dan como la época.
pub fn utc_iso8601(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Días desde 1970-01-01 → (año, mes, día) en el calendario gregoriano proléptico
/// (algoritmo de Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fechas_conocidas() {
        assert_eq!(utc_iso8601(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400); // 29-02-2000
        assert_eq!(utc_iso8601(t), "2000-02-29T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_794_960_000 + 3_723);
        assert_eq!(utc_iso8601(t), "2026-11-18T01:02:03Z");
    }
}
