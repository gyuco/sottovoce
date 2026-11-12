//! Date RFC 3339 senza dipendenze: quanto basta per le API dei calendari
//! (`timeMin`/`timeMax` in UTC, `dateTime` degli eventi con offset).

const MS_PER_DAY: i64 = 86_400_000;

/// Giorni dal 1970-01-01 per una data del calendario gregoriano.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Data del calendario gregoriano per i giorni dal 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-11-12T13:30:00Z` (UTC, secondi interi).
pub fn format_utc(ms: i64) -> String {
    let days = ms.div_euclid(MS_PER_DAY);
    let secs = ms.rem_euclid(MS_PER_DAY) / 1000;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn num(s: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let part = s.get(range)?;
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` → ms epoch.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 20 || !matches!(s.as_bytes()[10], b'T' | b't' | b' ') {
        return None;
    }
    let (y, mo, d) = (num(s, 0..4)?, num(s, 5..7)?, num(s, 8..10)?);
    let (h, mi, se) = (num(s, 11..13)?, num(s, 14..16)?, num(s, 17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut frac_ms = 0;
    if let Some(after) = rest.strip_prefix('.') {
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let ms: String = after[..digits]
            .chars()
            .chain("000".chars())
            .take(3)
            .collect();
        frac_ms = ms.parse::<i64>().ok()?;
        rest = &after[digits..];
    }
    let offset_min = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && matches!(rest.as_bytes()[0], b'+' | b'-') => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (num(rest, 1..3)? * 60 + num(rest, 4..6)?)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    let local = days * MS_PER_DAY + ((h * 60 + mi) * 60 + se) * 1000 + frac_ms;
    Some(local - offset_min * 60_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_offsets() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        let ms = parse_rfc3339("2026-11-12T14:30:00+01:00").unwrap();
        assert_eq!(format_utc(ms), "2026-11-12T13:30:00Z");
        assert_eq!(parse_rfc3339("2026-11-12T13:30:00Z"), Some(ms));
        assert_eq!(
            parse_rfc3339("2026-11-12T08:30:00.250-05:00"),
            Some(ms + 250)
        );
        assert_eq!(
            format_utc(parse_rfc3339("2024-02-29T23:59:59Z").unwrap()),
            "2024-02-29T23:59:59Z"
        );
        assert_eq!(format_utc(-1000), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "2026-11-12",
            "2026-11-12T14:30:00",
            "2026-13-01T00:00:00Z",
            "2026-11-12T14:30:00+0100",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }
}
