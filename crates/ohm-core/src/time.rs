//! Time helpers. Everything on the wire is Unix milliseconds; humans get
//! RFC 3339 in logs.

use chrono::{DateTime, Local, TimeZone, Utc};

/// Current wall clock time in Unix milliseconds.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Current time as an RFC 3339 string in the local timezone.
pub fn now_rfc3339() -> String {
    unix_ms_to_rfc3339(now_ms())
}

/// Convert Unix milliseconds to a local RFC 3339 string.
pub fn unix_ms_to_rfc3339(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        None => ms.to_string(),
    }
}

/// Convert Unix milliseconds to a UTC `DateTime`.
pub fn unix_ms_to_utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_plausible() {
        // 2020-01-01 .. 2100-01-01
        let now = now_ms();
        assert!(now > 1_577_836_800_000, "clock looks wrong: {now}");
        assert!(now < 4_102_444_800_000, "clock looks wrong: {now}");
    }

    #[test]
    fn rfc3339_renders_local_time_for_the_same_instant() {
        // 1_700_000_000_000 ms == 2023-11-14T22:13:20Z, which can be either the
        // 14th or the 15th locally, so assert on the instant instead of the date.
        let utc = unix_ms_to_utc(1_700_000_000_000);
        assert_eq!(
            utc.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2023-11-14T22:13:20"
        );
        let local = unix_ms_to_rfc3339(1_700_000_000_000);
        assert!(local.starts_with("2023-11-1"), "unexpected: {local}");
        assert!(local.contains('T'), "unexpected: {local}");
    }
}
