#[cfg(not(any(feature = "backend-perf", feature = "backend-powercap")))]
compile_error!(
    "the rapl source needs a backend: enable `backend-perf`, `backend-powercap`, or both"
);

#[cfg(feature = "backend-perf")]
mod counter;
pub mod domain;
pub mod error;

#[cfg(feature = "backend-perf")]
pub mod perf;

#[cfg(feature = "backend-powercap")]
pub mod powercap;

use std::collections::HashSet;

use joule_profiler_core::source::Source;
use serde::Deserialize;

use crate::domain::{EnergyUnit, RaplDomainType};
use crate::error::Result;

/// How the energy counters are read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Through the `power` PMU.
    #[cfg(feature = "backend-perf")]
    #[cfg_attr(feature = "backend-perf", default)]
    Perf,

    /// Through `/sys/class/powercap`, which needs no perf access.
    #[cfg(feature = "backend-powercap")]
    #[cfg_attr(not(feature = "backend-perf"), default)]
    Powercap,
}

/// Energy per socket and domain. Both the builder and the `[sources.rapl]` table.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rapl {
    backend: Backend,
    unit: EnergyUnit,
    sockets: Option<HashSet<u32>>,
    domains: Option<HashSet<RaplDomainType>>,
}

impl Rapl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    pub fn unit(mut self, unit: EnergyUnit) -> Self {
        self.unit = unit;
        self
    }

    pub fn sockets(mut self, sockets: impl IntoIterator<Item = u32>) -> Self {
        self.sockets = Some(sockets.into_iter().collect());
        self
    }

    pub fn domains(mut self, domains: impl IntoIterator<Item = RaplDomainType>) -> Self {
        self.domains = Some(domains.into_iter().collect());
        self
    }

    pub fn build(self) -> Result<Source> {
        let sockets = self.sockets.as_ref();
        let domains = self.domains.as_ref();

        match self.backend {
            #[cfg(feature = "backend-perf")]
            Backend::Perf => perf::build(self.unit, sockets, domains),

            #[cfg(feature = "backend-powercap")]
            Backend::Powercap => powercap::build(self.unit, sockets, domains),
        }
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
        assert_eq!(read(json!({})).unwrap().backend, Backend::Perf);
    }

    #[cfg(feature = "backend-powercap")]
    #[test]
    fn the_backend_field_names_the_one_to_read() {
        let rapl = read(json!({ "backend": "powercap", "unit": "millijoule" })).unwrap();

        assert_eq!(rapl.backend, Backend::Powercap);
        assert_eq!(rapl.unit, EnergyUnit::Millijoule);
    }

    #[test]
    fn a_backend_or_a_setting_that_does_not_exist_is_refused() {
        assert!(read(json!({ "backend": "rapl" })).is_err());
        assert!(read(json!({ "units": 3 })).is_err());
    }
}
