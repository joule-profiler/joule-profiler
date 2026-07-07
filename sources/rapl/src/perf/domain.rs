use std::{collections::HashSet, io::ErrorKind};

use log::debug;
use perf_event::{Builder, Group, ReadFormat, events::Software};

use crate::{
    Result,
    domain_type::RaplDomainType,
    error::{PerfParanoidError, RaplError},
    perf::{
        event::RaplEvent,
        socket::{SocketInfo, discover_socket_topology},
    },
};

/// Represents a RAPL domain with its `perf_event` counter.
///
/// Each domain corresponds to a measurable component like `PACKAGE-0`, `DRAM-0`, etc.
#[derive(Debug)]
pub struct PerfRaplDomain {
    /// Domain type (package, dram, core, etc.).
    pub domain_type: RaplDomainType,

    pub event_counter: RaplEvent,
}

impl PerfRaplDomain {
    /// Create a new `PerfRaplDomain`.
    pub fn new(domain_type: RaplDomainType, event_counter: RaplEvent) -> Self {
        Self {
            domain_type,
            event_counter,
        }
    }

    /// Apply the domain-specific scaling factor to convert raw value to joules.
    #[allow(clippy::cast_precision_loss)]
    pub fn compute_scale(&self, value: u64) -> f64 {
        value as f64 * self.event_counter.scale
    }

    /// Return the domain name including socket, e.g., `PACKAGE-0`.
    pub fn get_name(&self, socket: u32) -> String {
        self.domain_type.to_string_socket(socket)
    }
}

/// Attempts to create a `perf_event` group for a given socket by trying each CPU
/// in the list of CPUs until one succeeds.
///
/// Returns the group if it has been successfully initialized, else returns an `RaplError::FailToOpenDomainCounter`.
///
/// # Notes
///
/// * Some PMUs, such as RAPL energy counters, are only accessible via a
///   specific CPU on the socket (often the first CPU). This function ensures
///   portability by trying all CPUs in the socket.
pub fn build_group_for_socket(socket: &SocketInfo) -> Result<Group> {
    for cpu in &socket.cpus {
        debug!("Trying to build perf group on CPU {cpu}");

        match Builder::new(Software::DUMMY)
            .read_format(ReadFormat::GROUP)
            .one_cpu(*cpu as usize)
            .any_pid()
            .exclude_kernel(false)
            .exclude_hv(false)
            .build_group()
        {
            Ok(group) => {
                debug!("Perf group successfully built on CPU {cpu}");
                return Ok(group);
            }
            Err(err) => {
                debug!("Failed to build group on CPU {cpu}: {err:?}");
                if err.kind() == ErrorKind::PermissionDenied {
                    return Err(PerfParanoidError::IoError(err).into());
                }
            }
        }
    }

    Err(RaplError::FailToOpenDomainCounter(format!(
        "unable to find a CPU associate to the {} socket's PMU",
        socket.id
    )))
}

/// Discover the system's socket topology. The returned sockets aren't opened
/// yet (see [`crate::perf::event::open_counters`]).
pub fn discover_domains(domains_to_discover: Option<&HashSet<u32>>) -> Result<Vec<SocketInfo>> {
    let socket_topology = discover_socket_topology(domains_to_discover)?;
    debug!("Discovered {} socket(s)", socket_topology.len());
    Ok(socket_topology)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::domain_type::RaplDomainType;
    use crate::perf::event::RaplEvent;
    use std::mem::ManuallyDrop;

    fn mock_domain(domain_type: RaplDomainType, scale: f64) -> ManuallyDrop<PerfRaplDomain> {
        let event = RaplEvent {
            scale,
            counter: unsafe { std::mem::zeroed() },
        };
        ManuallyDrop::new(PerfRaplDomain::new(domain_type, event))
    }

    #[test]
    fn compute_scale_multiplies_raw_value_by_scale() {
        let d = mock_domain(RaplDomainType::Package, 2.5);
        assert_eq!(d.compute_scale(4), 10.0);
    }

    #[test]
    fn compute_scale_zero_value_returns_zero() {
        let d = mock_domain(RaplDomainType::Package, 2.5);
        assert_eq!(d.compute_scale(0), 0.0);
    }

    #[test]
    fn compute_scale_scale_of_one_is_identity() {
        let d = mock_domain(RaplDomainType::Dram, 1.0);
        assert_eq!(d.compute_scale(42), 42.0);
    }

    #[test]
    fn get_name_includes_socket_for_regular_domains() {
        assert_eq!(
            mock_domain(RaplDomainType::Package, 1.0).get_name(0),
            "PACKAGE-0"
        );
        assert_eq!(
            mock_domain(RaplDomainType::Package, 1.0).get_name(1),
            "PACKAGE-1"
        );
        assert_eq!(mock_domain(RaplDomainType::Core, 1.0).get_name(0), "CORE-0");
        assert_eq!(mock_domain(RaplDomainType::Dram, 1.0).get_name(2), "DRAM-2");
        assert_eq!(
            mock_domain(RaplDomainType::Uncore, 1.0).get_name(0),
            "UNCORE-0"
        );
    }

    #[test]
    fn get_name_psys_omits_socket_number() {
        assert_eq!(mock_domain(RaplDomainType::Psys, 1.0).get_name(0), "PSYS");
        assert_eq!(mock_domain(RaplDomainType::Psys, 1.0).get_name(99), "PSYS");
    }
}
