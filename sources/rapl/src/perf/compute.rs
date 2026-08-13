use std::collections::HashMap;

use log::{debug, error, info, trace};

use crate::{
    Result, domain_type::RaplDomainIndex, error::RaplError, perf::socket::Socket,
    snapshot::Snapshot,
};

/// Compute one measurement from two energy snapshots.
pub fn compute_measurement_from_snapshots(
    sockets: &[Socket],
    begin: &Snapshot,
    end: &Snapshot,
) -> Result<HashMap<RaplDomainIndex, u64>> {
    trace!(
        "Computing measurement from snapshots for {} sockets",
        sockets.len()
    );

    let mut per_domain_energy: HashMap<RaplDomainIndex, u64> = HashMap::new();

    for socket in sockets {
        trace!(
            "Computing measurement from snapshots for {} domains for socket {}",
            socket.domains.len(),
            socket.id
        );

        for domain in &socket.domains {
            let domain_name = domain.get_name(socket.id);
            trace!("Processing domain '{domain_name}'");

            let domain_index = (domain.domain_type, socket.id);

            let start_value = if let Some(v) = begin.metrics.get(&domain_index) {
                *v
            } else {
                error!("Missing start energy snapshot for domain '{domain_name}'");
                return Err(RaplError::RaplReadError(format!(
                    "Missing start energy snapshot for domain '{domain_name}'"
                )));
            };

            let end_value = if let Some(v) = end.metrics.get(&domain_index) {
                *v
            } else {
                error!("Missing end energy snapshot for domain '{domain_name}'");
                return Err(RaplError::RaplReadError(format!(
                    "Missing end energy snapshot for domain '{domain_name}'"
                )));
            };

            let diff = energy_diff(start_value, end_value);
            debug!(
                "Domain '{domain_name}': start={start_value} µJ, end={end_value} µJ, diff={diff} µJ",
            );

            per_domain_energy
                .entry(domain_index)
                .and_modify(|v| *v += diff)
                .or_insert(diff);
        }
    }

    info!(
        "Computed energy measurement for {} domains",
        per_domain_energy.len()
    );

    Ok(per_domain_energy)
}

/// Compute difference between two u64, wrapping them if there is an overflow.
fn energy_diff(start: u64, end: u64) -> u64 {
    if end >= start {
        end - start
    } else {
        (u64::MAX - start) + end
    }
}

/// Convert joules to microjoules.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
pub fn joules_to_micro_joules(joules: f64) -> u64 {
    (joules * 1_000_000.0) as u64
}

#[cfg(test)]
mod tests {
    use std::mem::ManuallyDrop;

    use super::*;
    use crate::{
        domain_type::RaplDomainType,
        perf::{domain::PerfRaplDomain, socket::Socket},
        snapshot::Snapshot,
    };

    fn snapshot(metrics: Vec<((RaplDomainType, u32), u64)>) -> Snapshot {
        Snapshot {
            metrics: metrics.into_iter().collect(),
        }
    }

    fn socket_with_domains(id: u32, domain_types: Vec<RaplDomainType>) -> ManuallyDrop<Socket> {
        let domains = domain_types
            .into_iter()
            .map(|dt| PerfRaplDomain::new(dt, unsafe { std::mem::zeroed() }))
            .collect();
        ManuallyDrop::new(Socket {
            id,
            domains,
            group: unsafe { std::mem::zeroed() },
        })
    }

    fn as_sockets(v: &[ManuallyDrop<Socket>]) -> &[Socket] {
        unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<Socket>(), v.len()) }
    }

    #[test]
    fn energy_diff_no_overflow() {
        assert_eq!(energy_diff(10, 50), 40);
    }

    #[test]
    fn energy_diff_equal_values_returns_zero() {
        assert_eq!(energy_diff(42, 42), 0);
    }

    #[test]
    fn energy_diff_wraps_on_overflow() {
        assert_eq!(energy_diff(u64::MAX - 5, 10), 15);
    }

    #[test]
    fn energy_diff_start_zero() {
        assert_eq!(energy_diff(0, 100), 100);
    }

    #[test]
    fn energy_diff_end_zero_wraps() {
        assert_eq!(energy_diff(1, 0), u64::MAX - 1);
    }

    #[test]
    fn joules_to_micro_joules_one_joule() {
        assert_eq!(joules_to_micro_joules(1.0), 1_000_000);
    }

    #[test]
    fn joules_to_micro_joules_zero() {
        assert_eq!(joules_to_micro_joules(0.0), 0);
    }

    #[test]
    fn joules_to_micro_joules_fractional() {
        assert_eq!(joules_to_micro_joules(0.5), 500_000);
    }

    #[test]
    fn joules_to_micro_joules_small_value() {
        assert_eq!(joules_to_micro_joules(0.000_001), 1);
    }

    #[test]
    fn compute_measurement_no_sockets_returns_empty() {
        let begin = snapshot(vec![]);
        let end = snapshot(vec![]);
        let result = compute_measurement_from_snapshots(&[], &begin, &end).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn compute_measurement_single_domain() {
        let key = (RaplDomainType::Package, 0);
        let begin = snapshot(vec![(key, 100)]);
        let end = snapshot(vec![(key, 350)]);
        let sockets = vec![socket_with_domains(0, vec![RaplDomainType::Package])];

        let result =
            compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end).unwrap();

        assert_eq!(result[&key], 250);
    }

    #[test]
    fn compute_measurement_multiple_domains() {
        let pkg = (RaplDomainType::Package, 0);
        let dram = (RaplDomainType::Dram, 0);
        let begin = snapshot(vec![(pkg, 0), (dram, 1000)]);
        let end = snapshot(vec![(pkg, 500), (dram, 1200)]);
        let sockets = vec![socket_with_domains(
            0,
            vec![RaplDomainType::Package, RaplDomainType::Dram],
        )];

        let result =
            compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end).unwrap();

        assert_eq!(result[&pkg], 500);
        assert_eq!(result[&dram], 200);
    }

    #[test]
    fn compute_measurement_counter_overflow_is_handled() {
        let key = (RaplDomainType::Package, 0);
        let begin = snapshot(vec![(key, u64::MAX - 10)]);
        let end = snapshot(vec![(key, 5)]);
        let sockets = vec![socket_with_domains(0, vec![RaplDomainType::Package])];

        let result =
            compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end).unwrap();

        assert_eq!(result[&key], 15);
    }

    #[test]
    fn compute_measurement_missing_start_returns_error() {
        let key = (RaplDomainType::Package, 0);
        let begin = snapshot(vec![]);
        let end = snapshot(vec![(key, 100)]);
        let sockets = vec![socket_with_domains(0, vec![RaplDomainType::Package])];

        let result = compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end);

        assert!(matches!(result, Err(RaplError::RaplReadError(_))));
    }

    #[test]
    fn compute_measurement_missing_end_returns_error() {
        let key = (RaplDomainType::Package, 0);
        let begin = snapshot(vec![(key, 100)]);
        let end = snapshot(vec![]);
        let sockets = vec![socket_with_domains(0, vec![RaplDomainType::Package])];

        let result = compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end);

        assert!(matches!(result, Err(RaplError::RaplReadError(_))));
    }

    #[test]
    fn compute_measurement_multiple_sockets() {
        let pkg0 = (RaplDomainType::Package, 0);
        let pkg1 = (RaplDomainType::Package, 1);
        let begin = snapshot(vec![(pkg0, 0), (pkg1, 0)]);
        let end = snapshot(vec![(pkg0, 300), (pkg1, 700)]);
        let sockets = vec![
            socket_with_domains(0, vec![RaplDomainType::Package]),
            socket_with_domains(1, vec![RaplDomainType::Package]),
        ];

        let result =
            compute_measurement_from_snapshots(as_sockets(&sockets), &begin, &end).unwrap();

        assert_eq!(result[&pkg0], 300);
        assert_eq!(result[&pkg1], 700);
    }
}
