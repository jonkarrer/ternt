//! Time as the journal keeps it: µs since the epoch.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};

/// µs since the epoch: checkpoints and journal entries are both timed to the microsecond, so
/// a timeline can put them in the order they happened.
pub fn us(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

/// `2026-11-20 14:03:12.120Z` from µs since the epoch: UTC, to the millisecond.
pub fn clock(us: u64) -> String {
    let ms = us / 1000;
    let secs = ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Days since the epoch to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

/// `90s`, `30m`, `2h`, `7d` → µs.
pub fn duration(text: &str) -> Result<u64> {
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = number
        .parse()
        .with_context(|| format!("not a duration: {text}"))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => bail!("not a duration: {text} (use s, m, h or d)"),
    };
    Ok(n * seconds * 1_000_000)
}

/// µs since the epoch, now.
pub fn now() -> u64 {
    us(SystemTime::now())
}

/// A day, in µs.
pub const DAY: u64 = 86_400_000_000;

/// Ms since the epoch: how times leave tokidoki (`interface`).
pub fn ms(at: SystemTime) -> u64 {
    us(at) / 1000
}
