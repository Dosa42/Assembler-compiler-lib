//! Hand-rolled UTC "now" as an ISO-8601 string, matching Python's
//! `datetime.now(timezone.utc).isoformat()` — used only for the CLI's
//! `--json` output's `generated_at` field (the schema's one deliberately
//! non-deterministic field; everything else in a `--json` document is a
//! pure function of the input `.s` bytes).
//!
//! Implemented against the standard library's `SystemTime` plus Howard
//! Hinnant's public-domain `civil_from_days` algorithm
//! (<http://howardhinnant.github.io/date_algorithms.html>) rather than
//! depending on `chrono`/`time`, continuing this crate's "stdlib only"
//! discipline. Verified against Python's `datetime` across leap-year
//! edge cases and both pre- and post-epoch dates before being ported
//! (see `tests` below).

use std::time::{SystemTime, UNIX_EPOCH};

fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if r != 0 && (r < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

/// Converts a day count since the Unix epoch (1970-01-01) into a
/// proleptic-Gregorian `(year, month, day)` triple.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = floor_div(z, 146097);
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SS.ffffff+00:00`, matching
/// Python's `datetime.now(timezone.utc).isoformat()` format.
pub fn utc_now_iso8601() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format_from_epoch(now.as_secs() as i64, now.subsec_micros())
}

fn format_from_epoch(secs: i64, micros: u32) -> String {
    let days = floor_div(secs, 86400);
    let time_of_day = secs - days * 86400; // 0..86400
    let (y, m, d) = civil_from_days(days);
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{micros:06}+00:00")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_python_datetime_across_edge_cases() {
        // (days_since_epoch, expected_y_m_d) -- generated from Python's
        // `(date - date(1970,1,1)).days` for each date, including a leap
        // day, two centurial non/leap years, and pre-epoch (negative z).
        let cases = [
            (0i64, (1970, 1, 1)),
            (1, (1970, 1, 2)),
            (-1, (1969, 12, 31)),
            (-3653, (1960, 1, 1)),
            (11016, (2000, 2, 29)),
            (20662, (2026, 7, 28)),
            (19782, (2024, 2, 29)),
            (47541, (2100, 3, 1)),
            (157113, (2400, 2, 29)),
            (-25508, (1900, 3, 1)),
        ];
        for (z, expected) in cases {
            assert_eq!(civil_from_days(z), expected, "mismatch for z={z}");
        }
    }

    #[test]
    fn format_matches_python_isoformat_shape() {
        // 2026-07-28T14:33:09.123456+00:00, cross-checked against
        // Python's `datetime(...).isoformat()` for the same instant.
        let secs = 1_785_249_189i64; // 2026-07-28T14:33:09 UTC, confirmed via Python datetime.timestamp()
        assert_eq!(format_from_epoch(secs, 123456), "2026-07-28T14:33:09.123456+00:00");
    }
}
