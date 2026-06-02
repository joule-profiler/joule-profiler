use amdsmi::types::EnergyCount;

#[derive(Debug, Default, Clone, Copy)]
pub struct EnergyCounter {
    begin: Option<EnergyCount>,
    end: Option<EnergyCount>,
}

impl EnergyCounter {
    pub fn update(&mut self, value: EnergyCount) {
        if self.begin.is_none() {
            self.begin = Some(value);
        } else {
            self.end = Some(value);
        }
    }

    pub fn reset(&mut self) {
        self.begin = self.end;
    }

    pub fn diff(&self) -> Option<u64> {
        if let Some(begin) = &self.begin
            && let Some(end) = &self.end
        {
            let energy = ((end.energy_accumulator as f64 * f64::from(end.counter_resolution))
                as u64)
                .saturating_sub(
                    (begin.energy_accumulator as f64 * f64::from(begin.counter_resolution)) as u64,
                );
            Some(energy)
        } else {
            None
        }
    }
}

impl From<EnergyCount> for EnergyCounter {
    fn from(value: EnergyCount) -> Self {
        Self {
            begin: Some(value),
            ..Default::default()
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct VramCounter {
    pub min: Option<u64>,
    pub max: Option<u64>,
}

impl VramCounter {
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

    pub fn reset(&mut self) {
        self.min = None;
        self.max = None;
    }
}

#[derive(Clone, Copy)]
pub struct PowerMeasurement {
    pub timestamp: u128,
    pub power: u32,
}

#[derive(Default, Clone)]
pub struct PowerCounter(Vec<PowerMeasurement>);

impl PowerCounter {
    pub fn update(&mut self, value: PowerMeasurement) {
        self.0.push(value);
    }

    pub fn reset(&mut self) {
        self.0 = Vec::new();
    }

    pub fn compute_energy(&self) -> u64 {
        let mut energy: u64 = 0;
        for power in self.0.windows(2) {
            let p1 = power[0];
            let p2 = power[1];
            let duration_us = (p2.timestamp - p1.timestamp) as u64;

            let avg_power = u64::from(p1.power.midpoint(p2.power));
            energy += avg_power * duration_us;
        }
        energy
    }
}

#[derive(Default, Clone)]
pub struct Counter {
    pub energy: Option<EnergyCounter>,
    pub vram: Option<VramCounter>,
    pub power: Option<PowerCounter>,
}
