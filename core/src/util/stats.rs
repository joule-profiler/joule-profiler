/// The minimum, maximum and mean of the values polled during a phase.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MinMaxMean {
    min: Option<u64>,
    max: Option<u64>,
    sum: u128,
    count: u64,
}

impl MinMaxMean {
    pub fn add(&mut self, value: u64) {
        self.min = Some(self.min.map_or(value, |min| min.min(value)));
        self.max = Some(self.max.map_or(value, |max| max.max(value)));
        self.sum += u128::from(value);
        self.count += 1;
    }

    /// `(min, max, mean)`, all 0 when nothing was polled.
    pub fn min_max_mean(self) -> (u64, u64, u64) {
        let mean = u64::try_from(self.sum / u128::from(self.count.max(1))).unwrap_or(u64::MAX);

        (self.min.unwrap_or(0), self.max.unwrap_or(0), mean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lowest_the_highest_and_the_mean_of_the_values_are_kept() {
        let mut stats = MinMaxMean::default();
        for value in [200, 100, 600] {
            stats.add(value);
        }

        assert_eq!(stats.min_max_mean(), (100, 600, 300));
    }

    #[test]
    fn no_value_comes_to_zeros() {
        assert_eq!(MinMaxMean::default().min_max_mean(), (0, 0, 0));
    }
}
