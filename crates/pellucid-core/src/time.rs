//! Time helpers — every Pellucid timestamp is encoded as a signed i64 of
//! milliseconds since the Unix epoch. Negative values are forbidden in
//! envelope schemas but allowed at the type level so subtractions remain
//! ordinary arithmetic.

use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current Unix-epoch timestamp in milliseconds.
///
/// Negative clock skew (system time before 1970) is clamped to 0 because
/// all envelope and cache layers below assume monotonic non-negative
/// timestamps. Returning a sentinel rather than panicking keeps the
/// gateway's request path resilient to weird VM clocks.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Returns the duration between two epoch-millisecond timestamps, in
/// whole milliseconds. `from` after `to` returns a negative value so
/// callers can detect time-travel rather than silently swallowing it.
#[must_use]
pub fn duration_between_ms(from: i64, to: i64) -> i64 {
    to - from
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ms_is_after_2020_01_01() {
        // 2020-01-01T00:00:00Z = 1577836800000 ms.
        let ts = now_ms();
        assert!(
            ts > 1_577_836_800_000,
            "expected post-2020 timestamp, got {ts}"
        );
    }

    #[test]
    fn now_ms_is_before_year_2100() {
        // 2100-01-01T00:00:00Z = 4102444800000 ms.
        let ts = now_ms();
        assert!(
            ts < 4_102_444_800_000,
            "now_ms drifted past year 2100: {ts}"
        );
    }

    #[test]
    fn now_ms_is_monotonic_within_a_call() {
        let a = now_ms();
        let b = now_ms();
        assert!(b >= a, "now_ms went backwards: {a} → {b}");
    }

    #[test]
    fn duration_between_ms_is_signed() {
        assert_eq!(duration_between_ms(100, 250), 150);
        assert_eq!(duration_between_ms(250, 100), -150);
        assert_eq!(duration_between_ms(0, 0), 0);
    }
}
