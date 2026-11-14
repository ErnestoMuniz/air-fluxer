//! Local wall-clock time: formatting message timestamps in the user's time zone.

use std::sync::OnceLock;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The local UTC offset in minutes, read once from `date +%z` (std has no time-zone API). UTC if unavailable.
fn offset_minutes() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
        out.and_then(|z| parse_offset(z.trim())).unwrap_or(0)
    })
}

fn parse_offset(z: &str) -> Option<i64> {
    let sign = match z.as_bytes().first()? { b'+' => 1, b'-' => -1, _ => return None };
    let digits = &z[1..];
    if digits.len() != 4 { return None }
    Some(sign * (digits[..2].parse::<i64>().ok()? * 60 + digits[2..].parse::<i64>().ok()?))
}

/// Days since 1970-01-01 in local time, plus minutes into that day.
fn local_parts(ms: i64) -> (i64, i64) {
    let minutes = ms.div_euclid(60_000) + offset_minutes();
    (minutes.div_euclid(1440), minutes.rem_euclid(1440))
}

pub fn day_of(ms: i64) -> i64 { local_parts(ms).0 }

/// (year, month, day) in local time.
pub fn ymd(ms: i64) -> (i64, i64, i64) { civil(day_of(ms)) }

pub fn hhmm(ms: i64) -> String {
    let m = local_parts(ms).1;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const MONTHS: [&str; 12] = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];

/// "Hoje", "Ontem" or "6 de novembro de 2026".
pub fn day_label(ms: i64, now: i64) -> String {
    let (d, today) = (day_of(ms), day_of(now));
    match today - d { 0 => "Hoje".into(), 1 => "Ontem".into(), _ => {
        let (y, m, day) = civil(d);
        format!("{day} de {} de {y}", MONTHS[(m - 1) as usize])
    } }
}

/// "hoje às 15:37", "ontem às 15:37" or "06/11/2026 15:37", for hover text on grouped messages.
pub fn full(ms: i64, now: i64) -> String {
    let d = day_of(ms);
    match day_of(now) - d {
        0 => format!("Hoje às {}", hhmm(ms)),
        1 => format!("Ontem às {}", hhmm(ms)),
        _ => { let (y, m, day) = civil(d); format!("{day:02}/{m:02}/{y} {}", hhmm(ms)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_parse() {
        assert_eq!(parse_offset("-0300"), Some(-180));
        assert_eq!(parse_offset("+0530"), Some(330));
        assert_eq!(parse_offset("x"), None);
    }

    #[test]
    fn civil_dates_round_trip_with_the_api_parser() {
        let ms = fluxer_client::api::parse_timestamp("2026-11-06T15:37:45Z").unwrap();
        assert_eq!(civil(ms.div_euclid(86_400_000)), (2026, 11, 6));
        assert_eq!(day_label(ms, ms), "Hoje");
        assert_eq!(day_label(ms, ms + 86_400_000), "Ontem");
        assert!(day_label(ms, ms + 5 * 86_400_000).starts_with("6 de novembro") || day_label(ms, ms + 5 * 86_400_000).starts_with("5 de novembro") || day_label(ms, ms + 5 * 86_400_000).starts_with("7 de novembro"));
    }
}
