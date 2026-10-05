use std::time::{SystemTime, UNIX_EPOCH};

/// Microseconds since the Unix epoch.
pub fn get_timestamp_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_micros())
}

/// Milliseconds between two timestamps in microseconds.
pub(crate) fn millis_between(start_us: u128, end_us: u128) -> u64 {
    u64::try_from(end_us.saturating_sub(start_us) / 1000).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliseconds_are_whole_and_never_negative() {
        assert_eq!(millis_between(1_000, 3_999), 2);
        assert_eq!(millis_between(5_000, 1_000), 0);
    }
}
