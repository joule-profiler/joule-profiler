use std::fmt::{self, Display};

use joule_profiler_core::metric::MetricValue;
use joule_profiler_core::unit::MetricUnit;
use serde::Deserialize;

/// The unit the energy is reported in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnergyUnit {
    Joule,
    Millijoule,
    #[default]
    Microjoule,
}

impl EnergyUnit {
    /// Returns the multiplicator for each unit.
    pub(crate) fn per_joule(self) -> f64 {
        match self {
            Self::Joule => 1.0,
            Self::Millijoule => 1e3,
            Self::Microjoule => 1e6,
        }
    }

    /// Returns `energy` as unsigned integer in millijoules or microjoules, or as fractional joules.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an energy is positive, and a phase never accumulates 2^64 microjoules"
    )]
    pub(crate) fn value(self, energy: f64) -> MetricValue {
        match self {
            Self::Joule => MetricValue::F64(energy),
            Self::Millijoule | Self::Microjoule => MetricValue::U64(energy.round() as u64),
        }
    }
}

impl From<EnergyUnit> for MetricUnit {
    fn from(value: EnergyUnit) -> Self {
        match value {
            EnergyUnit::Joule => MetricUnit::JOULE,
            EnergyUnit::Millijoule => MetricUnit::MILLIJOULE,
            EnergyUnit::Microjoule => MetricUnit::MICROJOULE,
        }
    }
}

/// A RAPL energy domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RaplDomainType {
    /// The whole socket (PKG).
    Package,

    /// The cores (PP0).
    Core,

    /// The integrated GPU (PP1).
    Uncore,

    /// The memory attached to the socket.
    Dram,

    /// The whole platform, not tied to a socket.
    Psys,
}

impl Display for RaplDomainType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Package => "PACKAGE",
            Self::Core => "CORE",
            Self::Uncore => "UNCORE",
            Self::Dram => "DRAM",
            Self::Psys => "PSYS",
        })
    }
}

/// A domain on one socket, and on one die when the socket has several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RaplDomain {
    pub(crate) domain_type: RaplDomainType,
    pub(crate) socket: u32,
    pub(crate) die: Option<u32>,
}

impl RaplDomain {
    /// Returns the metric name of the domain, such as `PACKAGE-0`, `PACKAGE-0-DIE-1` or `PSYS`.
    pub(crate) fn name(&self) -> String {
        match (self.domain_type, self.die) {
            (RaplDomainType::Psys, _) => self.domain_type.to_string(),
            (_, Some(die)) => format!("{}-{}-DIE-{die}", self.domain_type, self.socket),
            (_, None) => format!("{}-{}", self.domain_type, self.socket),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millijoules_and_microjoules_are_rounded_and_joules_are_not() {
        assert_eq!(
            EnergyUnit::Microjoule.value(706_726.4),
            MetricValue::U64(706_726)
        );
        assert_eq!(EnergyUnit::Millijoule.value(8.545), MetricValue::U64(9));
        assert_eq!(
            EnergyUnit::Joule.value(0.706_726),
            MetricValue::F64(0.706_726)
        );
    }

    #[test]
    fn a_metric_is_named_after_its_socket_and_die_except_for_psys() {
        let domain = |domain_type, die| RaplDomain {
            domain_type,
            socket: 1,
            die,
        };

        assert_eq!(domain(RaplDomainType::Dram, None).name(), "DRAM-1");
        assert_eq!(domain(RaplDomainType::Dram, Some(2)).name(), "DRAM-1-DIE-2");
        assert_eq!(domain(RaplDomainType::Psys, Some(2)).name(), "PSYS");
    }
}
