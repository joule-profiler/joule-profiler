//! RAPL source for Joule Profiler.
//!
//! Measures the energy consumed by the processor and the memory with RAPL (Running Average Power
//! Limit), the energy counters of Intel and AMD processors.
//!
//! # Backends
//!
//! - [`BackendType::Perf`] (default) reads the `power` PMU with the `perf_event` interface.
//!   Requires `kernel.perf_event_paranoid` at most 0 or `CAP_PERFMON` capability.
//! - [`BackendType::Powercap`] uses the `Powercap` framework using the `/sys/class/powercap` Linux vfs.
//!   Requires root access since Linux 5.10.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_source_rapl::{EnergyUnit, Rapl, RaplDomainType};
//!
//! let source = Rapl::new()
//!     .unit(EnergyUnit::Millijoule)
//!     .domains([RaplDomainType::Package, RaplDomainType::Dram])
//!     .build()
//!     .unwrap();
//! ```

#[cfg(not(any(feature = "backend-perf", feature = "backend-powercap")))]
compile_error!(
    "the RAPL source needs a backend: enable `backend-perf`, `backend-powercap`, or both"
);

mod backend;
mod config;
mod domain;
mod error;
mod processor;
mod sensor;

use std::collections::HashSet;
#[cfg(feature = "backend-powercap")]
use std::{path::PathBuf, time::Duration};

use joule_profiler_core::info::InfoValue;
use joule_profiler_core::source::Source;
use joule_profiler_core::unit::MetricUnit;
#[cfg(all(feature = "backend-perf", feature = "backend-powercap"))]
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

pub use config::BackendType;
#[cfg(feature = "backend-powercap")]
pub use config::Powercap;
pub use domain::{EnergyUnit, RaplDomainType};
pub use error::RaplError;

use crate::backend::RaplBackend;
#[cfg(feature = "backend-perf")]
use crate::backend::perf::{self, PerfBackend};
#[cfg(feature = "backend-powercap")]
use crate::backend::powercap::{self, JOULES_PER_COUNT, POWERCAP_PATH, PowercapBackend};
use crate::domain::RaplDomain;
use crate::error::Result;
use crate::processor::RaplProcessor;
use crate::sensor::RaplSensor;

/// Configuration of the RAPL source.
#[derive(Debug, Clone, Default)]
pub struct Rapl {
    backend: BackendType,
    unit: EnergyUnit,
    sockets: Option<HashSet<u32>>,
    domains: Option<HashSet<RaplDomainType>>,
}

impl Rapl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the backend that reads the counters.
    pub fn backend(mut self, backend: impl Into<BackendType>) -> Self {
        self.backend = backend.into();
        self
    }

    /// Sets the unit of the reported energy. Defaults to microjoules.
    pub fn unit(mut self, unit: EnergyUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Sets the sockets to measure. Defaults to every socket. PSYS, which has no socket, is not
    /// affected.
    pub fn sockets(mut self, sockets: impl IntoIterator<Item = u32>) -> Self {
        self.sockets = Some(sockets.into_iter().collect());
        self
    }

    /// Sets the domains to measure. Defaults to every domain the CPU supports.
    pub fn domains(mut self, domains: impl IntoIterator<Item = RaplDomainType>) -> Self {
        self.domains = Some(domains.into_iter().collect());
        self
    }

    /// Builds the source from the requested domains the CPU supports.
    ///
    /// # Errors
    ///
    /// Returns [`RaplError::UnknownSocket`] if a requested socket does not exist, and
    /// [`RaplError::NoDomain`] if none of the requested domains is available.
    pub fn build(self) -> Result<Source> {
        let sockets = self.sockets.as_ref();
        let domains = self.domains.as_ref();

        match self.backend {
            #[cfg(feature = "backend-perf")]
            BackendType::Perf => {
                let domains = filter_domains(perf::discover_domains()?, sockets, domains)?;
                let joules_per_count = domains.iter().map(|(_, config)| config.scale).collect();
                build_source(PerfBackend::new(domains), joules_per_count, self.unit)
            }

            #[cfg(feature = "backend-powercap")]
            BackendType::Powercap(powercap) => {
                let root = powercap.root.unwrap_or_else(|| POWERCAP_PATH.into());
                let domains = filter_domains(powercap::discover_domains(&root)?, sockets, domains)?;
                let joules_per_count = vec![JOULES_PER_COUNT; domains.len()];
                build_source(
                    PowercapBackend::new(domains, powercap.poll_interval),
                    joules_per_count,
                    self.unit,
                )
            }
        }
    }
}

fn build_source<B: RaplBackend>(
    backend: B,
    joules_per_count: Vec<f64>,
    unit: EnergyUnit,
) -> Result<Source> {
    let domains = backend.domains().to_vec();

    if domains.is_empty() {
        return Err(RaplError::NoDomain);
    }

    log::debug!("rapl: {} domains found", domains.len());

    let info = backend
        .info()
        .with(
            "domains",
            InfoValue::list(domains.iter().map(RaplDomain::name)),
        )
        .with("unit", MetricUnit::from(unit).to_string());

    Ok(Source::new(
        RaplSensor::new(backend),
        RaplProcessor::new(domains, joules_per_count, unit),
    )
    .with_info(info))
}

/// Filters the domains found by the backend with the requested sockets and domains.
fn filter_domains<BackendData>(
    available_domains: Vec<(RaplDomain, BackendData)>,
    sockets: Option<&HashSet<u32>>,
    domains: Option<&HashSet<RaplDomainType>>,
) -> Result<Vec<(RaplDomain, BackendData)>> {
    if let Some(&unknown_socket) = sockets.into_iter().flatten().find(|socket_id| {
        !available_domains
            .iter()
            .any(|(domain, _)| domain.socket == **socket_id)
    }) {
        return Err(RaplError::UnknownSocket(unknown_socket));
    }

    let mut filtered_domains: Vec<(RaplDomain, BackendData)> = Vec::new();

    for (domain, backend_data) in available_domains {
        if domains.is_some_and(|domain_types| !domain_types.contains(&domain.domain_type)) {
            continue;
        }

        if domain.domain_type == RaplDomainType::Psys {
            if filtered_domains
                .iter()
                .any(|(filtered_domain, _)| filtered_domain.domain_type == RaplDomainType::Psys)
            {
                continue;
            }
        } else if sockets.is_some_and(|socket_ids| !socket_ids.contains(&domain.socket)) {
            continue;
        }

        filtered_domains.push((domain, backend_data));
    }

    Ok(filtered_domains)
}

/// Deserializes a table with `backend` (`perf` by default), `unit`, `sockets` and `domains`. The
/// `powercap` backend also takes `root` and `poll_interval`.
impl<'de> Deserialize<'de> for Rapl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Backend {
            #[cfg(feature = "backend-perf")]
            #[cfg_attr(feature = "backend-perf", default)]
            Perf,

            #[cfg(feature = "backend-powercap")]
            #[cfg_attr(not(feature = "backend-perf"), default)]
            Powercap,
        }

        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Table {
            backend: Backend,
            unit: EnergyUnit,
            sockets: Option<HashSet<u32>>,
            domains: Option<HashSet<RaplDomainType>>,
            #[cfg(feature = "backend-powercap")]
            root: Option<PathBuf>,
            #[cfg(feature = "backend-powercap")]
            #[serde(with = "humantime_serde")]
            poll_interval: Option<Duration>,
        }

        let table = Table::deserialize(deserializer)?;

        let backend = match table.backend {
            #[cfg(feature = "backend-perf")]
            Backend::Perf => {
                #[cfg(feature = "backend-powercap")]
                {
                    let powercap_keys = [
                        ("root", table.root.is_some()),
                        ("poll_interval", table.poll_interval.is_some()),
                    ];
                    if let Some((key, _)) = powercap_keys.iter().find(|(_, set)| *set) {
                        return Err(D::Error::custom(format!(
                            "`{key}` needs `backend = \"powercap\"`"
                        )));
                    }
                }
                BackendType::Perf
            }

            #[cfg(feature = "backend-powercap")]
            Backend::Powercap => {
                let mut powercap = Powercap::new();
                powercap.root = table.root;
                if let Some(interval) = table.poll_interval {
                    powercap.poll_interval = interval;
                }
                BackendType::Powercap(powercap)
            }
        };

        Ok(Self {
            backend,
            unit: table.unit,
            sockets: table.sockets,
            domains: table.domains,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read(table: serde_json::Value) -> serde_json::Result<Rapl> {
        serde_json::from_value(table)
    }

    #[cfg(feature = "backend-perf")]
    #[test]
    fn a_table_that_names_no_backend_reads_through_perf() {
        assert!(matches!(
            read(json!({})).unwrap().backend,
            BackendType::Perf
        ));
    }

    #[cfg(feature = "backend-powercap")]
    #[cfg_attr(not(feature = "backend-perf"), allow(irrefutable_let_patterns))]
    #[test]
    fn the_backend_field_names_the_one_to_read() {
        let rapl = read(json!({
            "backend": "powercap",
            "unit": "millijoule",
            "root": "/elsewhere",
            "poll_interval": "500ms",
        }))
        .unwrap();

        let BackendType::Powercap(powercap) = rapl.backend else {
            panic!("a powercap backend");
        };
        assert_eq!(powercap.root, Some(PathBuf::from("/elsewhere")));
        assert_eq!(powercap.poll_interval, Duration::from_millis(500));
        assert_eq!(rapl.unit, EnergyUnit::Millijoule);
    }

    #[cfg(all(feature = "backend-perf", feature = "backend-powercap"))]
    #[test]
    fn the_keys_of_powercap_need_the_powercap_backend() {
        for key in ["root", "poll_interval"] {
            let value = if key == "root" { "/elsewhere" } else { "500ms" };
            let error = read(json!({ key: value })).unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains(&format!("`{key}` needs `backend = \"powercap\"`")),
                "{error}"
            );
        }
    }

    fn domain(domain_type: RaplDomainType, socket: u32) -> (RaplDomain, ()) {
        let domain = RaplDomain {
            domain_type,
            socket,
            die: None,
        };
        (domain, ())
    }

    fn names(domains: &[(RaplDomain, ())]) -> Vec<String> {
        domains.iter().map(|(domain, ())| domain.name()).collect()
    }

    #[test]
    fn psys_is_kept_once_whatever_the_sockets() {
        let available_domains = vec![
            domain(RaplDomainType::Package, 0),
            domain(RaplDomainType::Psys, 0),
            domain(RaplDomainType::Package, 1),
            domain(RaplDomainType::Psys, 1),
        ];

        let domains = filter_domains(available_domains, Some(&HashSet::from([1])), None).unwrap();

        assert_eq!(names(&domains), ["PSYS", "PACKAGE-1"]);
    }

    #[test]
    fn only_the_requested_domain_types_are_kept() {
        let available_domains = vec![
            domain(RaplDomainType::Package, 0),
            domain(RaplDomainType::Dram, 0),
        ];

        let domains = filter_domains(
            available_domains,
            None,
            Some(&HashSet::from([RaplDomainType::Dram])),
        )
        .unwrap();

        assert_eq!(names(&domains), ["DRAM-0"]);
    }

    #[test]
    fn a_socket_that_does_not_exist_is_refused() {
        let available_domains = vec![domain(RaplDomainType::Package, 0)];

        assert!(matches!(
            filter_domains(available_domains, Some(&HashSet::from([3])), None),
            Err(RaplError::UnknownSocket(3))
        ));
    }
}
