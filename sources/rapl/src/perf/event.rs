use log::{debug, info, warn};
use perf_event::{Builder, Counter, Group, events::Dynamic};

use crate::{
    RaplError, Result,
    domain_type::RaplDomainType,
    perf::{
        domain::{PerfRaplDomain, build_group_for_socket},
        socket::{Socket, SocketInfo},
    },
};

#[derive(Debug)]
pub struct RaplEvent {
    pub counter: Counter,
    pub scale: f64,
}

impl RaplEvent {
    /// Creates a RAPL event counter for a domain on a socket, trying each CPU
    /// in the socket until one succeeds since some PMUs are only accessible
    /// through specific CPUs.
    ///
    /// Returns `DomainNotSupported` if the domain does not exist on the hardware,
    /// or `FailToOpenDomainCounter` if no CPU in the socket could open the counter.
    pub fn new(
        domain_type: RaplDomainType,
        socket: &SocketInfo,
        group: &mut Group,
    ) -> Result<Self> {
        let mut builder = Dynamic::builder("power")?;
        if let Err(err) = builder.event(domain_type.to_perf_event()) {
            return Err(match err.kind() {
                std::io::ErrorKind::NotFound => RaplError::DomainNotSupported(domain_type),
                _ => err.into(),
            });
        }

        let scale = builder.scale()?.ok_or(RaplError::RetrieveScaleError)?;

        for cpu in &socket.cpus {
            debug!("Trying to build RAPL event {domain_type:?} on CPU {cpu}");

            let counter_result = Builder::new(
                builder
                    .build()
                    .map_err(|err| RaplError::FailToOpenDomainCounter(err.to_string()))?,
            )
            .one_cpu(*cpu as usize)
            .any_pid()
            .include_hv()
            .include_kernel()
            .build_with_group(&mut *group);

            match counter_result {
                Ok(counter) => {
                    debug!("RAPL event {domain_type:?} successfully built on CPU {cpu}");
                    return Ok(Self { counter, scale });
                }
                Err(err) => {
                    debug!("Failed to build RAPL event {domain_type:?} on CPU {cpu}: {err:?}");
                }
            }
        }

        Err(RaplError::FailToOpenDomainCounter(format!(
            "unable to find a CPU associated with the {} socket's PMU for domain {:?}",
            socket.id, domain_type
        )))
    }
}

/// Opens perf counters for all discovered RAPL domains across sockets, and
/// enables them so they are active as soon as they're returned.
/// Sockets with no CPUs are skipped with a warning.
///
/// Returns an error if any counter fails to open for a non-empty socket.
pub fn open_counters(socket_topology: &[SocketInfo]) -> Result<Vec<Socket>> {
    let mut sockets = Vec::new();

    for socket_info in socket_topology {
        debug!(
            "Processing socket {} (cpus={:?})",
            socket_info.id, socket_info.cpus
        );

        if socket_info.cpus.is_empty() {
            warn!("Socket {} has no CPUs, skipping it", socket_info.id);
            continue;
        }

        let mut group = build_group_for_socket(socket_info)?;
        let domains = open_counters_for_socket(socket_info, &mut group)?;
        group.enable().map_err(RaplError::from)?;

        info!(
            "Opened {} RAPL domains for socket {}",
            domains.len(),
            socket_info.id
        );

        sockets.push(Socket {
            id: socket_info.id,
            group,
            domains,
        });
    }

    Ok(sockets)
}

static PER_SOCKET_DOMAIN_TYPES: &[RaplDomainType] = &[
    RaplDomainType::Package,
    RaplDomainType::Core,
    RaplDomainType::Uncore,
    RaplDomainType::Dram,
];

/// Opens available RAPL counters for a socket: PACKAGE, CORE, UNCORE and DRAM
/// for every socket, plus PSYS exclusively on socket 0. Unsupported domains
/// are silently skipped.
///
/// Returns an error if a supported domain fails to open for a reason other
/// than hardware unavailability.
pub fn open_counters_for_socket(
    socket: &SocketInfo,
    group: &mut Group,
) -> Result<Vec<PerfRaplDomain>> {
    let mut domains = Vec::new();

    for domain_type in PER_SOCKET_DOMAIN_TYPES {
        add_counter_to_domain_if_supported(*domain_type, socket, group, &mut domains)?;
    }

    if socket.id == 0
        && let Some(psys_domain) = open_psys_counter(socket, group)?
    {
        domains.push(psys_domain);
    }

    Ok(domains)
}

/// Tries to open the platform-wide PSYS counter, which measures total system
/// power and should only be opened once.
///
/// Returns None if Psys is not supported on the system, or an error if the
/// counter exists but could not be opened.
pub fn open_psys_counter(socket: &SocketInfo, group: &mut Group) -> Result<Option<PerfRaplDomain>> {
    match RaplEvent::new(RaplDomainType::Psys, socket, group) {
        Ok(counter) => {
            let domain = PerfRaplDomain::new(RaplDomainType::Psys, counter);
            Ok(Some(domain))
        }
        Err(RaplError::DomainNotSupported(_)) => {
            debug!("PSYS domain not supported on this system");
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

/// Adds a RAPL counter for a domain to the list. Unsupported domains are
/// silently ignored.
///
/// Returns an error if the domain is supported but the counter fails to open.
pub fn add_counter_to_domain_if_supported(
    domain_type: RaplDomainType,
    socket: &SocketInfo,
    group: &mut Group,
    domains: &mut Vec<PerfRaplDomain>,
) -> Result<()> {
    let counter = match RaplEvent::new(domain_type, socket, group) {
        Ok(counter) => counter,
        Err(err) => match err {
            RaplError::DomainNotSupported(_) => return Ok(()),
            _ => {
                return Err(err);
            }
        },
    };
    let domain = PerfRaplDomain::new(domain_type, counter);
    domains.push(domain);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket(id: u32, cpus: Vec<u32>) -> SocketInfo {
        SocketInfo { id, cpus }
    }

    #[test]
    fn open_counters_empty_topology_returns_empty() {
        let result = open_counters(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn open_counters_skips_socket_with_no_cpus() {
        let result = open_counters(&[socket(0, vec![])]).unwrap();
        assert!(result.is_empty());
    }
}
