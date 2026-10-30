//! Energy read from the `power` PMU.

use joule_profiler_core::{
    Info,
    util::sys::{Socket, socket_topology},
};
use perf_event::{Builder, Counter, events::Dynamic};

use crate::{
    backend::RaplBackend,
    domain::{RaplDomain, RaplDomainType},
    error::{RaplError, Result},
};

const PMU_NAME: &str = "power";

const DOMAINS: [RaplDomainType; 5] = [
    RaplDomainType::Package,
    RaplDomainType::Core,
    RaplDomainType::Uncore,
    RaplDomainType::Dram,
    RaplDomainType::Psys,
];

/// What the counter of a domain is opened with.
#[derive(Clone, Copy)]
pub(crate) struct CounterSpec {
    cpu: u32,
    event: Dynamic,

    /// The energy of one count, in joules.
    pub(crate) scale: f64,
}

/// Reads each domain with its own `perf_event` counter.
pub(crate) struct PerfBackend {
    domains: Vec<RaplDomain>,
    configs: Vec<CounterSpec>,
    counters: Vec<Counter>,
}

impl PerfBackend {
    pub(crate) fn new(domains: Vec<(RaplDomain, CounterSpec)>) -> Self {
        let (domains, configs) = domains.into_iter().unzip();

        Self {
            domains,
            configs,
            counters: Vec::new(),
        }
    }
}

impl RaplBackend for PerfBackend {
    fn info(&self) -> Info {
        Info::new().with("backend", "perf").with("pmu", PMU_NAME)
    }

    fn domains(&self) -> &[RaplDomain] {
        &self.domains
    }

    fn open(&mut self) -> Result<()> {
        self.counters = self
            .domains
            .iter()
            .zip(&self.configs)
            .map(|(domain, config)| {
                Builder::new(config.event)
                    .any_pid()
                    .one_cpu(config.cpu as usize)
                    .exclude_kernel(false)
                    .exclude_hv(false)
                    .enabled(true)
                    .build()
                    .map_err(|error| RaplError::OpenCounter(domain.name(), error))
            })
            .collect::<Result<_>>()?;

        Ok(())
    }

    fn read(&mut self) -> Result<Vec<u64>> {
        self.domains
            .iter()
            .zip(&mut self.counters)
            .map(|(domain, counter)| {
                counter
                    .read()
                    .map_err(|error| RaplError::ReadCounter(domain.name(), error))
            })
            .collect()
    }

    fn close(&mut self) -> Result<()> {
        self.counters.clear();
        Ok(())
    }
}

/// Returns every domain the `power` PMU can count, on the first CPU of each die.
pub(crate) fn discover_domains() -> Result<Vec<(RaplDomain, CounterSpec)>> {
    let mut events = Vec::new();
    for domain_type in DOMAINS {
        if let Some(event) = event(domain_type)? {
            events.push((domain_type, event));
        }
    }

    let topology = socket_topology().map_err(RaplError::Topology)?;

    Ok(domains_per_die(&topology, &events)
        .into_iter()
        .map(|(domain, cpu, (event, scale))| (domain, CounterSpec { cpu, event, scale }))
        .collect())
}

/// Returns a domain for each event on each die, with the first CPU of the die.
/// The die is only named when its socket has several.
fn domains_per_die<Event: Copy>(
    topology: &[Socket],
    events: &[(RaplDomainType, Event)],
) -> Vec<(RaplDomain, u32, Event)> {
    let mut domains = Vec::new();

    for socket in topology {
        let has_several_dies = socket.dies.len() > 1;

        for die in &socket.dies {
            let Some(&cpu) = die.cpus.first() else {
                continue;
            };

            for &(domain_type, event) in events {
                let domain = RaplDomain {
                    domain_type,
                    socket: socket.id,
                    die: has_several_dies.then_some(die.id),
                };

                domains.push((domain, cpu, event));
            }
        }
    }

    domains
}

/// Returns the event of `domain_type` and its scale, or `None` if the PMU has no such event.
fn event(domain_type: RaplDomainType) -> Result<Option<(Dynamic, f64)>> {
    let mut builder = Dynamic::builder(PMU_NAME).map_err(RaplError::Pmu)?;

    if builder.event(event_name(domain_type)).is_err() {
        return Ok(None);
    }

    let Some(scale) = builder.scale().map_err(RaplError::Pmu)? else {
        return Ok(None);
    };

    let event = builder.build().map_err(RaplError::Event)?;
    Ok(Some((event, scale)))
}

/// Returns the name of the event of `domain_type` in the `power` PMU.
fn event_name(domain_type: RaplDomainType) -> &'static str {
    match domain_type {
        RaplDomainType::Package => "energy-pkg",
        RaplDomainType::Core => "energy-cores",
        RaplDomainType::Uncore => "energy-gpu",
        RaplDomainType::Dram => "energy-ram",
        RaplDomainType::Psys => "energy-psys",
    }
}

#[cfg(test)]
mod tests {
    use joule_profiler_core::util::sys::Die;

    use super::*;

    fn die(id: u32, cpus: &[u32]) -> Die {
        Die {
            id,
            cpus: cpus.to_vec(),
        }
    }

    #[test]
    fn first_cpu_of_each_die() {
        let topology = [
            Socket {
                id: 0,
                cpus: vec![0, 1, 2, 3],
                dies: vec![die(0, &[0, 2]), die(1, &[1, 3])],
            },
            Socket {
                id: 1,
                cpus: vec![4, 5],
                dies: vec![die(0, &[4, 5])],
            },
        ];

        let domains: Vec<(String, u32)> =
            domains_per_die(&topology, &[(RaplDomainType::Package, ())])
                .into_iter()
                .map(|(domain, cpu, ())| (domain.name(), cpu))
                .collect();

        assert_eq!(
            domains,
            [
                ("PACKAGE-0-DIE-0".to_owned(), 0),
                ("PACKAGE-0-DIE-1".to_owned(), 1),
                ("PACKAGE-1".to_owned(), 4),
            ]
        );
    }
}
