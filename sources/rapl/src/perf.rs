use std::collections::HashSet;
use std::sync::Arc;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::source::Source;
use perf_event::events::Software;
use perf_event::{Builder, Counter, Group, ReadFormat};

use crate::counter::build_counter;
use crate::domain::{EnergyUnit, RaplDomain, RaplDomainType, discover_domains};
use crate::error::{RaplError, Result};

pub(crate) const PMU: &str = "power";

pub(crate) type RaplDomains = Arc<[RaplDomain]>;

/// Builds the source on the `power` PMU, for the sockets and domains asked for.
pub(crate) fn build(
    unit: EnergyUnit,
    sockets: Option<&HashSet<u32>>,
    domains: Option<&HashSet<RaplDomainType>>,
) -> Result<Source> {
    let domains: RaplDomains = discover_domains(sockets, domains, unit)?.into();

    if domains.is_empty() {
        return Err(RaplError::NoDomain);
    }

    log::debug!("rapl: {} perf domains discovered", domains.len());

    Ok(Source::new(
        RaplPerfSensor::new(Arc::clone(&domains)),
        RaplProcessor::new(domains, unit),
    ))
}

struct SocketGroup {
    cpu: u32,
    group: Group,
    counters: Vec<(Counter, usize)>,
}

impl SocketGroup {
    fn new(cpu: u32) -> Result<Self> {
        let group = Builder::new(Software::DUMMY)
            .read_format(ReadFormat::GROUP | ReadFormat::ID)
            .any_pid()
            .one_cpu(cpu as usize)
            .exclude_kernel(false)
            .exclude_hv(false)
            .build_group()
            .map_err(|e| RaplError::OpenGroup(cpu, e))?;

        Ok(Self {
            cpu,
            group,
            counters: Vec::new(),
        })
    }
}

pub struct RaplPerfSensor {
    domains: RaplDomains,
    sockets: Vec<SocketGroup>,
}

impl RaplPerfSensor {
    fn new(domains: RaplDomains) -> Self {
        Self {
            domains,
            sockets: Vec::new(),
        }
    }
}

impl Sensor for RaplPerfSensor {
    type Snapshot = Vec<u64>;

    type Error = RaplError;

    fn name(&self) -> &'static str {
        "rapl"
    }

    fn init(&mut self) -> Result<()> {
        for (slot, domain) in self.domains.iter().enumerate() {
            let index = if let Some(index) = self.sockets.iter().position(|s| s.cpu == domain.cpu) {
                index
            } else {
                self.sockets.push(SocketGroup::new(domain.cpu)?);
                self.sockets.len() - 1
            };

            let socket = &mut self.sockets[index];
            let counter = build_counter(domain)?
                .build_with_group(&mut socket.group)
                .map_err(|e| RaplError::OpenCounter(domain.name(), e))?;

            socket.counters.push((counter, slot));
        }

        for socket in &mut self.sockets {
            socket.group.enable().map_err(RaplError::Io)?;
        }

        log::debug!("rapl: {} socket groups opened", self.sockets.len());
        Ok(())
    }

    fn measure(&mut self) -> Result<Self::Snapshot> {
        let mut snapshot = vec![0; self.domains.len()];

        for socket in &mut self.sockets {
            let data = socket.group.read().map_err(RaplError::Io)?;

            for (counter, slot) in &socket.counters {
                snapshot[*slot] = data.get(counter).ok_or(RaplError::MissingCounter)?.value();
            }
        }

        Ok(snapshot)
    }

    fn close(&mut self) -> Result<()> {
        for socket in &mut self.sockets {
            socket.group.disable().map_err(RaplError::Io)?;
        }
        Ok(())
    }
}

pub struct RaplProcessor {
    domains: RaplDomains,
    unit: EnergyUnit,
}

impl RaplProcessor {
    fn new(domains: RaplDomains, unit: EnergyUnit) -> Self {
        Self { domains, unit }
    }
}

impl Processor<RaplPerfSensor> for RaplProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.domains
            .iter()
            .map(|domain| MetricInfo::new(domain.name(), self.unit.metric_unit()))
            .collect()
    }

    fn info(&self) -> Info {
        Info::new()
            .with("backend", "perf")
            .with("pmu", PMU)
            .with(
                "domains",
                InfoValue::list(self.domains.iter().map(RaplDomain::name)),
            )
            .with("unit", self.unit.metric_unit().to_string())
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
        values.extend(self.domains.iter().zip(current.iter().zip(previous)).map(
            |(domain, (current, previous))| {
                self.unit
                    .value(current.wrapping_sub(*previous) as f64 * domain.scale)
            },
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pmu_the_domains_and_the_unit_are_in_the_info() {
        let domain = |domain, socket| RaplDomain {
            domain,
            socket,
            cpu: 0,
            scale: 1.0,
        };
        let processor = RaplProcessor::new(
            Arc::new([
                domain(RaplDomainType::Package, 0),
                domain(RaplDomainType::Dram, 1),
            ]),
            EnergyUnit::Microjoule,
        );

        let info = processor.info();

        assert_eq!(info.get("pmu"), Some(&PMU.into()));
        assert_eq!(
            info.get("domains"),
            Some(&InfoValue::list(["PACKAGE-0", "DRAM-1"]))
        );
        assert_eq!(info.get("unit"), Some(&"µJ".into()));
    }
}
