// Gregorian UTC calendar arithmetic, independent of timezone and external crates.
use crate::types::Period;
pub fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yo = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    era * 146097 + yo * 365 + yo / 4 - yo / 100 + (153 * mp + 2) / 5 + d - 1 - 719468
}
pub fn civil(day: i64) -> (i64, i64, i64) {
    let z = day + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yo = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yo + era * 400;
    let doy = doe - (365 * yo + yo / 4 - yo / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m, d)
}
pub fn label(t: i64) -> String {
    let (y, m, d) = civil(t.div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}
pub fn parse(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20
        || !s.is_ascii()
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let n = |a, b| {
        let part = s.get(a..b)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok()
    };
    let (y, m, d, h, min, sec) = (
        n(0, 4)?,
        n(5, 7)?,
        n(8, 10)?,
        n(11, 13)?,
        n(14, 16)?,
        n(17, 19)?,
    );
    if !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
        || h > 23
        || min > 59
        || sec > 59
        || civil(days(y, m, d)) != (y, m, d)
    {
        return None;
    }
    let mut tail = &s[19..];
    if tail.starts_with('.') {
        tail = &tail[1..];
        let len = tail.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return None;
        }
        tail = &tail[len..];
    }
    let offset = if tail == "Z" {
        0
    } else {
        let b = tail.as_bytes();
        if b.len() != 6 || !matches!(b[0], b'+' | b'-') || b[3] != b':' {
            return None;
        }
        let h = tail[1..3].parse::<i64>().ok()?;
        let m = tail[4..6].parse::<i64>().ok()?;
        if !b[1..3].iter().chain(b[4..6].iter()).all(u8::is_ascii_digit) || h > 23 || m > 59 {
            return None;
        }
        (h * 3600 + m * 60) * if b[0] == b'+' { 1 } else { -1 }
    };
    Some(days(y, m, d) * 86400 + h * 3600 + min * 60 + sec - offset)
}
pub fn window(now: i64, per: Period) -> (i64, i64) {
    let day = now.div_euclid(86400);
    match per {
        Period::Week => {
            let start = day - (day + 3).rem_euclid(7);
            (start * 86400, (start + 7) * 86400)
        }
        Period::Month => {
            let (y, m, _) = civil(day);
            (
                days(y, m, 1) * 86400,
                days(y + i64::from(m == 12), if m == 12 { 1 } else { m + 1 }, 1) * 86400,
            )
        }
        Period::Day => (day * 86400, (day + 1) * 86400),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar() {
        assert_eq!(days(1970, 1, 1), 0);
        for d in -20000..30000 {
            let (y, m, day) = civil(d);
            assert_eq!(days(y, m, day), d);
        }
        assert!(parse("2025-02-29T00:00:00Z").is_none());
        assert_eq!(
            parse("2024-02-29T02:00:00+02:00"),
            parse("2024-02-29T00:00:00.123Z")
        );
        assert!(parse("2024-01-01T00:00:00Zjunk").is_none());
    }
    #[test]
    fn windows() {
        let t = parse("2024-02-29T12:00:00Z").unwrap();
        let (a, b) = window(t, Period::Month);
        assert_eq!(label(a), "2024-02-01");
        assert_eq!((b - a) / 86400, 29);
        let (a, b) = window(t, Period::Week);
        assert_eq!(label(a), "2024-02-26");
        assert_eq!((b - a) / 86400, 7);
        assert_eq!(window(t, Period::Day).1 - window(t, Period::Day).0, 86400);
    }
}
