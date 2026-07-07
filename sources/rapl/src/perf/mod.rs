use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
};

use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, Metrics},
};
use log::{info, trace};
use perf_event::GroupData;

use crate::{
    MICRO_JOULE_UNIT, Result,
    error::{PerfParanoidError, RaplError},
    perf::{
        compute::{compute_measurement_from_snapshots, joules_to_micro_joules},
        domain::discover_domains,
        event::open_counters,
        socket::{Socket, SocketInfo},
    },
    snapshot::{Phase, Snapshot},
    util::check_os,
};

mod compute;
mod config;
mod domain;
mod event;
mod socket;

pub use config::RaplConfig;

/// Default sysfs path for perf RAPL counters.
const PERF_RAPL_PATH: &str = "/sys/bus/event_source/devices/power";

/// perf RAPL source name for this metric reader.
const PERF_SOURCE_NAME: &str = "RAPL (perf_event)";

/// Path to check the kernel `perf_event_paranoid` level.
const PERF_PARANOID_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";

/// RAPL energy measurement source using Linux perf events.
///
/// Provides energy readings per RAPL domain (e.g., package, core, dram).
pub struct Rapl {
    /// Discovered socket topology, not yet opened. Consumed by [`MetricReader::pre_init`].
    socket_topology: Vec<SocketInfo>,

    /// Opened sockets with their associated counters. Empty until [`MetricReader::pre_init`] runs.
    sockets: Vec<Socket>,

    /// Begin snapshot of the current phase.
    begin_snapshot: Option<Snapshot>,

    /// End snapshot of the current phase.
    end_snapshot: Option<Snapshot>,
}

impl Rapl {
    /// Discovers a new RAPL reader with the specified sockets specification.
    pub fn new(sockets_spec: Option<&HashSet<u32>>) -> Result<Self> {
        check_os()?;

        trace!("Attempting to initialize RAPL reader: sockets={sockets_spec:?}");

        let socket_topology = discover_domains(sockets_spec)?;

        info!("Discovered {} socket(s)", socket_topology.len());

        Ok(Self {
            socket_topology,
            sockets: Vec::new(),
            begin_snapshot: None,
            end_snapshot: None,
        })
    }

    /// Check if perf events access is allowed.
    pub fn check_perf_access() -> Result<()> {
        read_pmu_type()?;
        Ok(())
    }

    /// Read the current counter values for all domains.
    fn read_domains_counter(&mut self) -> Result<Snapshot> {
        let mut metrics = HashMap::new();

        for socket in &mut self.sockets {
            let group_data: GroupData = socket.group.read()?;

            for domain in &socket.domains {
                let domain_index = (domain.domain_type, socket.id);

                let value = group_data
                    .get(&domain.event_counter.counter)
                    .map_or(0, |d| d.value());

                metrics.insert(domain_index, value);
            }
        }

        Ok(Snapshot { metrics })
    }
}

impl MetricReader for Rapl {
    type Type = Phase;
    type Error = RaplError;
    type Config = RaplConfig;

    fn from_config(config: Self::Config) -> Result<Self> {
        Self::new(config.sockets_spec.as_ref())
    }

    async fn pre_init(&mut self) -> Result<()> {
        let paranoid_level = read_paranoid_level()?;

        self.sockets = open_counters(&self.socket_topology).map_err(|err| {
            if let RaplError::PerfParanoid(_) = err
                && paranoid_level > 0
            {
                PerfParanoidError::ParanoidLevelTooHigh(paranoid_level).into()
            } else {
                err
            }
        })?;

        Ok(())
    }

    /// Perform a measurement and accumulate metrics.
    ///
    /// Computes delta between last and current snapshot.
    async fn measure(&mut self) -> Result<()> {
        let new_snapshot = self.read_domains_counter()?;
        if self.begin_snapshot.is_none() {
            self.begin_snapshot = Some(new_snapshot);
        } else {
            self.end_snapshot = Some(new_snapshot);
        }
        Ok(())
    }

    /// Retrieve accumulated metrics since the last reset.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        trace!("Retrieving RAPL counters");

        if let Some(begin) = self.begin_snapshot.take()
            && let Some(end) = self.end_snapshot.take()
        {
            self.begin_snapshot = Some(end.clone());
            Ok(Phase { begin, end })
        } else {
            Err(RaplError::NotEnoughSamples)
        }
    }

    /// Return the list of available sensors for this source.
    ///
    /// If perf counters have already been opened (see [`MetricReader::pre_init`]),
    /// the actually opened domains are listed. Otherwise (e.g. `list-sensors`,
    /// which never calls `pre_init`), this falls back to the domain types
    /// discovered at construction time (see [`discover_domains`]), so no
    /// sysfs rescan or perf counter is needed here.
    fn get_sensors(&self) -> Result<Sensors> {
        trace!("Building RAPL sensor list");

        if !self.sockets.is_empty() {
            return Ok(self
                .sockets
                .iter()
                .flat_map(|socket| {
                    socket.domains.iter().map(|domain| {
                        let domain_name = domain.get_name(socket.id);
                        trace!("Registering sensor: {domain_name}");
                        Sensor::new(domain_name, MICRO_JOULE_UNIT, Self::get_name())
                    })
                })
                .collect());
        }

        let sensors = self
            .socket_topology
            .iter()
            .flat_map(|socket| {
                socket.domain_types.iter().map(|domain_type| {
                    let domain_name = domain_type.to_string_socket(socket.id);
                    trace!("Registering sensor: {domain_name}");
                    Sensor::new(domain_name, MICRO_JOULE_UNIT, Self::get_name())
                })
            })
            .collect();

        Ok(sensors)
    }

    fn to_metrics(&self, snapshot: Self::Type) -> Result<Metrics> {
        let diff =
            compute_measurement_from_snapshots(&self.sockets, &snapshot.begin, &snapshot.end)?;
        let result = self
            .sockets
            .iter()
            .flat_map(|socket| {
                socket.domains.iter().filter_map(|domain| {
                    let domain_index = (domain.domain_type, socket.id);
                    if let Some(metric) = diff.get(&domain_index) {
                        let joules = domain.compute_scale(*metric);
                        let micro_joules = joules_to_micro_joules(joules);
                        Some(Metric::new(
                            domain.domain_type.to_string_socket(socket.id),
                            micro_joules,
                            MICRO_JOULE_UNIT,
                            Self::get_name(),
                        ))
                    } else {
                        None
                    }
                })
            })
            .collect();
        Ok(result)
    }

    fn get_name() -> &'static str {
        PERF_SOURCE_NAME
    }

    fn get_id() -> &'static str {
        "rapl"
    }
}

/// Read the PMU type from sysfs.
///
/// Returns an `RaplError` if the file cannot be read or parsed.
fn read_pmu_type() -> Result<u32> {
    read_pmu_type_from_path(PERF_RAPL_PATH)
}

/// Read the `perf_event_paranoid` level from `/proc`.
///
/// Returns a `PerfParanoidError` if the file cannot be read or parsed.
fn read_paranoid_level() -> Result<i8> {
    read_paranoid_level_from_path(PERF_PARANOID_PATH)
}

fn read_pmu_type_from_path(path: &str) -> Result<u32> {
    let type_path = format!("{path}/type");
    fs::read_to_string(type_path)
        .map_err(|err| RaplError::RaplNotAvailable(format!("Failed to read perf PMU type {err}")))
        .map(|s| s.trim().parse::<u32>())?
        .map_err(Into::into)
}

fn read_paranoid_level_from_path(path: &str) -> Result<i8> {
    fs::read_to_string(path)
        .map_err(|err| match err.kind() {
            ErrorKind::NotFound => PerfParanoidError::NotFound,
            ErrorKind::PermissionDenied => PerfParanoidError::PermissionDenied(err.to_string()),
            _ => PerfParanoidError::IoError(err),
        })
        .map(|s| s.trim().parse::<i8>())?
        .map_err(|err| PerfParanoidError::ParseParanoidLevelError(err).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) -> String {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn read_paranoid_level_valid_value() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "paranoid", "1\n");
        assert_eq!(read_paranoid_level_from_path(&path).unwrap(), 1);
    }

    #[test]
    fn read_paranoid_level_negative() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "paranoid", "-1\n");
        assert_eq!(read_paranoid_level_from_path(&path).unwrap(), -1);
    }

    #[test]
    fn read_paranoid_level_missing_file_returns_not_found() {
        let result = read_paranoid_level_from_path("/nonexistent/paranoid");
        assert!(matches!(
            result,
            Err(RaplError::PerfParanoid(PerfParanoidError::NotFound))
        ));
    }

    #[test]
    fn read_paranoid_level_invalid_content_returns_parse_error() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "paranoid", "not_a_number\n");
        assert!(matches!(
            read_paranoid_level_from_path(&path),
            Err(RaplError::PerfParanoid(
                PerfParanoidError::ParseParanoidLevelError(_)
            ))
        ));
    }

    #[test]
    fn read_paranoid_level_empty_file_returns_parse_error() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "paranoid", "");
        assert!(matches!(
            read_paranoid_level_from_path(&path),
            Err(RaplError::PerfParanoid(
                PerfParanoidError::ParseParanoidLevelError(_)
            ))
        ));
    }

    #[test]
    fn read_pmu_type_valid_value() {
        let dir = TempDir::new().unwrap();
        let type_dir = dir.path().join("type");
        fs::write(&type_dir, "").unwrap(); // create parent structure
        let base = TempDir::new().unwrap();
        fs::create_dir_all(base.path()).unwrap();
        fs::write(base.path().join("type"), "7\n").unwrap();

        let result = read_pmu_type_from_path(base.path().to_str().unwrap()).unwrap();
        assert_eq!(result, 7);
    }

    #[test]
    fn read_pmu_type_missing_file_returns_rapl_not_available() {
        let result = read_pmu_type_from_path("/nonexistent");
        assert!(matches!(result, Err(RaplError::RaplNotAvailable(_))));
    }

    #[test]
    fn read_pmu_type_invalid_content_returns_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("type"), "garbage\n").unwrap();
        let result = read_pmu_type_from_path(dir.path().to_str().unwrap());
        assert!(result.is_err());
    }
}
