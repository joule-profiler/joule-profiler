use joule_profiler_core::{MetricInfo, MetricUnit, MetricValue, Processor};

use crate::{
    backend::RaplBackend,
    domain::{EnergyUnit, RaplDomain},
    error::Result,
    sensor::RaplSensor,
};

/// Computes the energy of each domain over a phase.
pub(crate) struct RaplProcessor {
    domains: Vec<RaplDomain>,

    /// The energy of one count of each domain, in the configured unit.
    energy_per_count: Vec<f64>,

    unit: EnergyUnit,
}

impl RaplProcessor {
    pub(crate) fn new(
        domains: Vec<RaplDomain>,
        joules_per_count: Vec<f64>,
        unit: EnergyUnit,
    ) -> Self {
        let energy_per_count = joules_per_count
            .into_iter()
            .map(|joules| joules * unit.per_joule())
            .collect();

        Self {
            domains,
            energy_per_count,
            unit,
        }
    }
}

impl<B: RaplBackend> Processor<RaplSensor<B>> for RaplProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.domains
            .iter()
            .map(|domain| MetricInfo::new(domain.name(), MetricUnit::from(self.unit)))
            .collect()
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "a phase never counts 2^52 ticks of energy"
    )]
    fn process(
        &mut self,
        previous: &Vec<u64>,
        current: &Vec<u64>,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        values.extend(
            self.energy_per_count
                .iter()
                .zip(current.iter().zip(previous))
                .map(|(energy_per_count, (current, previous))| {
                    self.unit
                        .value(current.wrapping_sub(*previous) as f64 * energy_per_count)
                }),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use joule_profiler_core::Info;

    use super::*;
    use crate::domain::RaplDomainType;

    struct MockBackend;

    impl RaplBackend for MockBackend {
        fn info(&self) -> Info {
            Info::new()
        }

        fn domains(&self) -> &[RaplDomain] {
            &[]
        }

        fn read(&mut self) -> Result<Vec<u64>> {
            Ok(Vec::new())
        }
    }

    fn energy(unit: EnergyUnit, previous: u64, current: u64) -> MetricValue {
        let domains = vec![RaplDomain {
            domain_type: RaplDomainType::Package,
            socket: 0,
            die: None,
        }];
        let mut processor = RaplProcessor::new(domains, vec![1e-6], unit);
        let mut values = Vec::new();

        <RaplProcessor as Processor<RaplSensor<MockBackend>>>::process(
            &mut processor,
            &vec![previous],
            &vec![current],
            &mut values,
        )
        .unwrap();

        values[0]
    }

    #[test]
    fn energy_unit() {
        assert_eq!(
            energy(EnergyUnit::Microjoule, 1_000, 9_545),
            MetricValue::U64(8_545)
        );
        assert_eq!(
            energy(EnergyUnit::Millijoule, 1_000, 9_545),
            MetricValue::U64(9)
        );
        assert!(matches!(
            energy(EnergyUnit::Joule, 1_000, 9_545),
            MetricValue::F64(joules) if (joules - 0.008_545).abs() < 1e-12
        ));
    }
}
