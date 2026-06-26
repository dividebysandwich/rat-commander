//! Human-readable formatting helpers (sizes, times).

use std::time::{SystemTime, UNIX_EPOCH};

/// Format a byte count like `1.2K`, `345M`, `2.0G`. Mirrors the compact
/// columns used by Midnight Commander.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}{}", UNITS[0]);
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Format a `SystemTime` as `MMM DD HH:MM` (recent) or `MMM DD  YYYY` (old),
/// matching the `ls -l`/mc style. Uses a minimal local-agnostic civil-date
/// computation so we don't need a calendar crate.
pub fn format_time(time: SystemTime) -> String {
    let secs = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return "            ".to_string(),
    };
    let (year, month, day, hour, min) = civil_from_unix(secs);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS[(month - 1) as usize];

    // "Recent" = within ~6 months in the past or near future shows time,
    // otherwise shows the year. We approximate against the file's own epoch
    // distance from now.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(secs);
    let six_months = 60 * 60 * 24 * 182;
    if (now - secs).abs() < six_months {
        format!("{mon} {day:>2} {hour:02}:{min:02}")
    } else {
        format!("{mon} {day:>2}  {year}")
    }
}

/// Convert a Unix timestamp (UTC) into civil (year, month, day, hour, minute).
/// Based on Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_unix(secs: i64) -> (i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, hour, min)
}
