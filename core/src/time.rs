// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since 1970, as the index stores every time.
pub(crate) fn now() -> i64 {
    seconds(SystemTime::now())
}

pub(crate) fn seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Nanoseconds since 1970: what tells a file changed since the last scan.
pub(crate) fn nanos(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64,
        Err(e) => -(e.duration().as_nanos() as i64),
    }
}

/// `2026-11-07 16:47 UTC`.
pub fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        of_day / 3_600,
        of_day % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_are_civil() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(super::format_utc(1_794_070_020), "2026-11-07 16:47 UTC");
        assert_eq!(super::format_utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}
