use amdsmi::types::EnergyCount;

/// Energy counter based on cumulative energy readings reported by AMD SMI.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnergyCounter {
    begin: Option<EnergyCount>,
    end: Option<EnergyCount>,
}

impl EnergyCounter {
    /// Records an energy measurement.
    ///
    /// The first value becomes the starting measurement. Subsequent values
    /// replace the ending measurement.
    pub fn update(&mut self, value: EnergyCount) {
        if self.begin.is_none() {
            self.begin = Some(value);
        } else {
            self.end = Some(value);
        }
    }

    /// Resets the counter by replacing begin by end.
    pub fn reset(&mut self) {
        self.begin = self.end;
    }

    /// Returns the energy consumed between the starting and ending
    /// measurements.
    ///
    /// Returns `None` if either measurement is missing.
    pub fn diff(&self) -> Option<u64> {
        match (self.begin, self.end) {
            (Some(begin), Some(end)) => Some(energy(&end).saturating_sub(energy(&begin))),
            _ => None,
        }
    }
}

impl From<EnergyCount> for EnergyCounter {
    fn from(value: EnergyCount) -> Self {
        Self {
            begin: Some(value),
            end: None,
        }
    }
}

/// Tracks the minimum and maximum VRAM usage observed during a measurement interval.
#[derive(Default, Clone, Copy)]
pub struct VramCounter {
    /// Lowest VRAM usage observed.
    pub min: Option<u64>,

    /// Highest VRAM usage observed.
    pub max: Option<u64>,
}

impl VramCounter {
    /// Updates the tracked minimum and maximum VRAM usage values.
    pub fn update(&mut self, value: u64) {
        self.min = Some(if let Some(min) = self.min {
            min.min(value)
        } else {
            value
        });

        self.max = Some(if let Some(max) = self.max {
            max.max(value)
        } else {
            value
        });
    }

    /// Resets the VRAM usage counters.
    pub fn reset(&mut self) {
        self.min = None;
        self.max = None;
    }
}

/// Instantaneous power measurement.
#[derive(Clone, Copy)]
pub struct PowerMeasurement {
    /// Timestamp in microseconds.
    pub timestamp: u128,

    /// Power draw in Watts.
    pub power: u32,
}

/// Collection of power samples used to estimate energy consumption based on power draws.
#[derive(Default, Clone)]
pub struct PowerCounter(Vec<PowerMeasurement>);

impl PowerCounter {
    /// Appends a power measurement.
    pub fn push(&mut self, value: PowerMeasurement) {
        self.0.push(value);
    }

    /// Removes all stored samples.
    pub fn reset(&mut self) {
        self.0 = Vec::new();
    }

    /// Estimates the consumed energy by integrating power samples using
    /// the trapezoidal rule (average power over each interval).
    ///
    /// The returned value is expressed in microjoules (µJ)
    pub fn compute_energy(&self) -> u64 {
        let mut energy: u64 = 0;
        for power in self.0.windows(2) {
            let p1 = power[0];
            let p2 = power[1];
            let duration_us = u64::try_from(p2.timestamp - p1.timestamp).unwrap_or(u64::MAX);

            let avg_power = u64::from(p1.power.midpoint(p2.power));
            energy += avg_power * duration_us;
        }
        energy
    }
}

/// Tracks the minimum and maximum GPU usage observed during a measurement interval.
#[derive(Default, Clone, Copy)]
pub struct UtilizationCounter {
    /// Lowest GPU usage observed.
    pub min: Option<u32>,

    /// Highest GPU usage observed.
    pub max: Option<u32>,
}

impl UtilizationCounter {
    /// Updates the tracked minimum and maximum GPU usage values.
    pub fn update(&mut self, value: u32) {
        self.min = Some(if let Some(min) = self.min {
            min.min(value)
        } else {
            value
        });

        self.max = Some(if let Some(max) = self.max {
            max.max(value)
        } else {
            value
        });
    }

    /// Resets the VRAM usage counters.
    pub fn reset(&mut self) {
        self.min = None;
        self.max = None;
    }
}

/// Available measurement counters for a device.
#[derive(Default, Clone)]
pub struct Counter {
    /// Energy counter based on hardware energy accumulation registers.
    pub energy: Option<EnergyCounter>,

    /// VRAM usage statistics.
    pub vram: Option<VramCounter>,

    /// Power draw samples.
    pub power: Option<PowerCounter>,

    /// GPU usage counter.
    pub utilization: Option<UtilizationCounter>,
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn energy(count: &EnergyCount) -> u64 {
    (count.energy_accumulator as f64 * f64::from(count.counter_resolution)) as u64
}
