use std::{ops::AddAssign, time::Duration};

use perf_event::CounterData;

/// A snapshot of a counter: its value and the times it was enabled and running, in nanoseconds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawCount {
    pub(crate) value: u64,
    pub(crate) enabled: u64,
    pub(crate) running: u64,
}

impl RawCount {
    /// Creates a snapshot. Missing times are set to 0.
    pub(crate) fn new(value: u64, enabled: Option<Duration>, running: Option<Duration>) -> Self {
        let nanos = |time: Option<Duration>| {
            time.map_or(0, |time| u64::try_from(time.as_nanos()).unwrap_or(u64::MAX))
        };

        Self {
            value,
            enabled: nanos(enabled),
            running: nanos(running),
        }
    }

    /// Returns the count since the previous snapshot, scaled if the counter was multiplexed,
    /// or `None` if it never ran.
    pub(crate) fn scaled_delta(&self, previous: &Self) -> Option<u64> {
        let value = self.value.wrapping_sub(previous.value);
        let enabled = self.enabled.saturating_sub(previous.enabled);
        let running = self.running.saturating_sub(previous.running);

        if running == enabled {
            return Some(value);
        }
        if running == 0 {
            return None;
        }

        let scaled = u128::from(value) * u128::from(enabled) / u128::from(running);
        Some(u64::try_from(scaled).unwrap_or(u64::MAX))
    }
}

impl From<CounterData> for RawCount {
    fn from(data: CounterData) -> Self {
        Self::new(data.count(), data.time_enabled(), data.time_running())
    }
}

impl AddAssign for RawCount {
    fn add_assign(&mut self, other: Self) {
        self.value = self.value.wrapping_add(other.value);
        self.enabled += other.enabled;
        self.running += other.running;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn raw_count(value: u64, enabled: u64, running: u64) -> RawCount {
        RawCount {
            value,
            enabled,
            running,
        }
    }

    #[test]
    fn a_counter_that_always_ran_is_not_scaled() {
        let delta = raw_count(350, 3000, 3000).scaled_delta(&raw_count(100, 1000, 1000));

        assert_eq!(delta, Some(250));
    }

    #[test]
    fn a_counter_that_ran_half_the_phase_is_doubled() {
        let delta = raw_count(600, 3000, 2000).scaled_delta(&raw_count(100, 1000, 1000));

        assert_eq!(delta, Some(1000));
    }

    #[test]
    fn the_counts_of_every_cpu_are_summed_before_being_scaled() {
        let mut total = RawCount::default();
        total += raw_count(100, 1000, 1000);
        total += raw_count(100, 1000, 500);

        assert_eq!(total, raw_count(200, 2000, 1500));
        assert_eq!(total.scaled_delta(&RawCount::default()), Some(266));
    }
}
