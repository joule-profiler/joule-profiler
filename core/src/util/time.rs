use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the microseconds since the Unix epoch.
pub fn timestamp_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_micros())
}

pub(crate) fn micros_between(start_us: u128, end_us: u128) -> u64 {
    u64::try_from(end_us.saturating_sub(start_us)).unwrap_or(u64::MAX)
}
