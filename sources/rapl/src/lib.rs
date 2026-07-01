// //! Intel RAPL metric source for Joule Profiler.
// //!
// //! This module provides several implementations of [`MetricReader`] for
// //! collecting energy metrics from Intel RAPL (Running Average Power Limit) domains.
// //!
// //! # Backends
// //!
// //! This module supports **two backends** for reading energy metrics:
// //! - [`powercap`] — uses the Linux `powercap` interface for energy readings.
// //! - [`perf`] — uses `perf_event` counters (`perf_event_open`) for RAPL domains.

// use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};

// mod domain_type;
// mod error;
// pub mod perf;
// pub mod powercap;
// mod snapshot;
// mod util;

// pub use error::RaplError;

// /// Custom result type for Rapl
// type Result<T> = std::result::Result<T, RaplError>;

const MICRO_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Joule,
};

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs;
use std::io::{self, ErrorKind};
use std::num::ParseIntError;

use joule_profiler_core::source::MetricSource;
use joule_profiler_core::source::Processor;
use joule_profiler_core::source::Sensor;
use joule_profiler_core::time::get_timestamp_micros;
use joule_profiler_core::types::{AvailableSensor, Metric, Metrics, Sensors};
use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};
use perf_event::events::{Dynamic, DynamicBuilder, Software};
use perf_event::{Builder, Counter, Group, GroupData, ReadFormat};
use thiserror::Error;
use tokio::task::JoinError;

const CPU_SYSFS_PATH: &str = "/sys/devices/system/cpu";
const ONLINE_CPU_SYSFS_PATH: &str = "/sys/devices/system/cpu/online";
const CPU_TOPOLOGY_SOCKET_ID: &str = "topology/physical_package_id";
const POWER_PMU_EVENTS_PATH: &str = "/sys/devices/power/events";

#[derive(Debug, Error)]
pub enum RaplError {
    #[error("failed to join tokio task")]
    Join(#[from] JoinError),

    #[error("reading {0}: {1}")]
    ReadOnlineCpus(&'static str, #[source] io::Error),

    #[error("invalid online cpu range: {0}")]
    InvalidCpuRange(String, #[source] ParseIntError),

    #[error("invalid online cpu id: {0}")]
    InvalidCpuId(String, #[source] ParseIntError),

    #[error("reading {0}: {1}")]
    ReadCpuDir(&'static str, #[source] io::Error),

    #[error("reading a cpu directory entry: {0}")]
    ReadCpuDirEntry(#[source] io::Error),

    #[error("reading {0}: {1}")]
    ReadPowerEvents(&'static str, #[source] io::Error),

    #[error("reading a power PMU events directory entry: {0}")]
    ReadPowerEventsEntry(#[source] io::Error),

    #[error("reading {0}: {1}")]
    ReadSocketId(String, #[source] io::Error),

    #[error("parsing socket id from {0}: {1}")]
    ParseSocketId(String, #[source] ParseIntError),

    #[error(
        "permission denied opening a perf group on cpu {0} (try: sudo sysctl kernel.perf_event_paranoid=0): {1}"
    )]
    PermissionDenied(u32, #[source] io::Error),

    #[error("opening a perf group on cpu {0}: {1}")]
    OpenGroup(u32, #[source] io::Error),

    #[error("opening the power PMU: {0}")]
    OpenPowerPmu(#[source] io::Error),

    #[error("configuring the {0} event: {1}")]
    ConfigureEvent(RaplDomain, #[source] io::Error),

    #[error("reading the scale for {0}: {1}")]
    ReadScale(RaplDomain, #[source] io::Error),

    #[error("no scale reported for {0}")]
    NoScale(RaplDomain),

    #[error("building the {0} event: {1}")]
    BuildEvent(RaplDomain, #[source] io::Error),

    #[error("opening the {0} counter on cpu {1}: {2}")]
    OpenCounter(RaplDomain, u32, #[source] io::Error),

    #[error("no RAPL domain available on this system")]
    NoRaplDomain,

    #[error("cannot disable socket {0} counter group: {1}")]
    CannotDisableGroup(usize, #[source] io::Error),

    #[error("enabling the perf group: {0}")]
    Enable(#[source] io::Error),

    #[error("reading the perf group: {0}")]
    Read(#[source] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RaplDomain {
    Package,
    Core,
    Uncore,
    Dram,
    Psys,
}

static PER_SOCKET_DOMAINS: &[RaplDomain] = &[
    RaplDomain::Package,
    RaplDomain::Core,
    RaplDomain::Uncore,
    RaplDomain::Dram,
];

impl RaplDomain {
    fn to_perf_event(self) -> &'static str {
        match self {
            RaplDomain::Package => "energy-pkg",
            RaplDomain::Core => "energy-cores",
            RaplDomain::Uncore => "energy-gpu",
            RaplDomain::Dram => "energy-ram",
            RaplDomain::Psys => "energy-psys",
        }
    }

    fn from_perf_event(name: &str) -> Option<Self> {
        match name {
            "energy-pkg" => Some(RaplDomain::Package),
            "energy-cores" => Some(RaplDomain::Core),
            "energy-gpu" => Some(RaplDomain::Uncore),
            "energy-ram" => Some(RaplDomain::Dram),
            "energy-psys" => Some(RaplDomain::Psys),
            _ => None,
        }
    }

    fn name(self, socket: u32) -> String {
        if matches!(self, RaplDomain::Psys) {
            "PSYS".into()
        } else {
            format!("{self}-{socket}")
        }
    }
}

impl Display for RaplDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RaplDomain::Package => "PACKAGE",
            RaplDomain::Core => "CORE",
            RaplDomain::Uncore => "UNCORE",
            RaplDomain::Dram => "DRAM",
            RaplDomain::Psys => "PSYS",
        })
    }
}

#[derive(Clone)]
struct SocketInfo {
    id: u32,
    cpu: u32,
}

struct DomainCounter {
    domain: RaplDomain,
    counter: Counter,
    scale: f64,
}

struct PerfSocket {
    id: u32,
    group: Group,
    domains: Vec<DomainCounter>,
}

#[derive(Debug, Clone, Copy)]
struct RaplCounter {
    raw: u64,
    scale: f64,
}

pub struct Snapshot {
    pub timestamp: u128,
    readings: HashMap<String, RaplCounter>,
}

pub struct RaplSensor {
    sockets: Vec<PerfSocket>,
}

impl Sensor<RaplSource> for RaplSensor {
    async fn pre_init(&mut self) -> Result<(), RaplError> {
        let mut sockets = tokio::task::spawn_blocking(|| {
            let topology = discover_sockets()?;
            open_counters(topology)
        })
        .await??;

        if sockets.iter().all(|s| s.domains.is_empty()) {
            return Err(RaplError::NoRaplDomain);
        }

        for socket in &mut sockets {
            socket.group.enable().map_err(RaplError::Enable)?;
        }

        self.sockets = sockets;
        Ok(())
    }

    async fn measure(&mut self) -> Result<Snapshot, RaplError> {
        let timestamp = get_timestamp_micros();
        let mut readings = HashMap::new();
        for socket in &mut self.sockets {
            let group_data: GroupData = socket.group.read().map_err(RaplError::Read)?;
            for domain in &socket.domains {
                let raw = group_data
                    .get(&domain.counter)
                    .map_or(0, |entry| entry.value());
                readings.insert(
                    domain.domain.name(socket.id),
                    RaplCounter {
                        raw,
                        scale: domain.scale,
                    },
                );
            }
        }
        Ok(Snapshot {
            timestamp,
            readings,
        })
    }

    async fn join(&mut self) -> Result<(), RaplError> {
        for (i, socket) in self.sockets.iter_mut().enumerate() {
            socket
                .group
                .disable()
                .map_err(|e| RaplError::CannotDisableGroup(i, e))?;
        }
        Ok(())
    }

    fn list_sensors(&self) -> Result<Sensors, RaplError> {
        let available_domains = scan_available_domains()?;
        let sockets_info = discover_sockets()?;

        let sensors = sockets_info
            .iter()
            .flat_map(|info| {
                domains_for_socket(info, &available_domains)
                    .into_iter()
                    .map(|domain| {
                        AvailableSensor::new(
                            domain.name(info.id),
                            MICRO_JOULE_UNIT,
                            RaplSource::get_name(),
                        )
                    })
            })
            .collect();
        Ok(sensors)
    }
}

#[derive(Default)]
pub struct RaplProcessor {
    baseline: Option<Snapshot>,
}

impl Processor<RaplSource> for RaplProcessor {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    async fn consume(&mut self, snapshot: Snapshot) -> Result<Option<Metrics>, RaplError> {
        let metrics = self.baseline.as_ref().map(|baseline| {
            snapshot
                .readings
                .iter()
                .filter_map(|(name, current)| {
                    let previous = baseline.readings.get(name)?;
                    let diff = current.raw.wrapping_sub(previous.raw);
                    let joules = ((diff as f64 * current.scale) * 1_000_000.0) as u64;
                    Some(Metric::new(
                        name.clone(),
                        joules,
                        RaplSource::get_name(),
                        baseline.timestamp,
                    ))
                })
                .collect()
        });

        self.baseline = Some(snapshot);
        Ok(metrics)
    }
}

pub struct RaplSource;

impl MetricSource for RaplSource {
    type Sensor = RaplSensor;
    type Processor = RaplProcessor;
    type Snapshot = Snapshot;
    type Error = RaplError;
    type Config = ();

    fn from_config(_config: ()) -> Result<(RaplSensor, RaplProcessor), RaplError> {
        Ok((
            RaplSensor {
                sockets: Vec::new(),
            },
            RaplProcessor::default(),
        ))
    }

    fn get_name() -> &'static str {
        "RAPL"
    }
}

fn build_group_for_socket(cpu: u32) -> Result<Group, RaplError> {
    Builder::new(Software::DUMMY)
        .read_format(ReadFormat::GROUP)
        .one_cpu(cpu as usize)
        .any_pid()
        .exclude_kernel(false)
        .exclude_hv(false)
        .build_group()
        .map_err(|e| match e.kind() {
            ErrorKind::PermissionDenied => RaplError::PermissionDenied(cpu, e),
            _ => RaplError::OpenGroup(cpu, e),
        })
}

fn open_domain_counter(
    builder: &mut DynamicBuilder,
    domain: RaplDomain,
    cpu: u32,
    group: &mut Group,
) -> Result<Option<DomainCounter>, RaplError> {
    if let Err(e) = builder.event(domain.to_perf_event()) {
        if e.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(RaplError::ConfigureEvent(domain, e));
    }
    let scale = builder
        .scale()
        .map_err(|e| RaplError::ReadScale(domain, e))?
        .ok_or(RaplError::NoScale(domain))?;

    let event = builder
        .build()
        .map_err(|e| RaplError::BuildEvent(domain, e.into()))?;
    let counter = Builder::new(event)
        .one_cpu(cpu as usize)
        .any_pid()
        .include_hv()
        .include_kernel()
        .build_with_group(&mut *group)
        .map_err(|e| RaplError::OpenCounter(domain, cpu, e))?;

    Ok(Some(DomainCounter {
        domain,
        counter,
        scale,
    }))
}

fn domains_for_socket(
    info: &SocketInfo,
    available_domains: &HashSet<RaplDomain>,
) -> Vec<RaplDomain> {
    let mut domains: Vec<RaplDomain> = PER_SOCKET_DOMAINS
        .iter()
        .copied()
        .filter(|domain| available_domains.contains(domain))
        .collect();

    if info.id == 0 && available_domains.contains(&RaplDomain::Psys) {
        domains.push(RaplDomain::Psys);
    }

    domains
}

fn open_counters_for_socket(
    info: &SocketInfo,
    group: &mut Group,
) -> Result<Vec<DomainCounter>, RaplError> {
    let available_domains = scan_available_domains()?;
    let mut builder = Dynamic::builder("power").map_err(RaplError::OpenPowerPmu)?;

    let mut domains = Vec::new();
    for domain in domains_for_socket(info, &available_domains) {
        if let Some(counter) = open_domain_counter(&mut builder, domain, info.cpu, group)? {
            domains.push(counter);
        }
    }
    Ok(domains)
}

fn open_counters(sockets_info: Vec<SocketInfo>) -> Result<Vec<PerfSocket>, RaplError> {
    let mut sockets = Vec::new();
    for info in sockets_info {
        let mut group = build_group_for_socket(info.cpu)?;
        let domains = open_counters_for_socket(&info, &mut group)?;
        sockets.push(PerfSocket {
            id: info.id,
            group,
            domains,
        });
    }
    Ok(sockets)
}

fn scan_available_domains() -> Result<HashSet<RaplDomain>, RaplError> {
    let entries = fs::read_dir(POWER_PMU_EVENTS_PATH)
        .map_err(|e| RaplError::ReadPowerEvents(POWER_PMU_EVENTS_PATH, e))?;

    let mut domains = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(RaplError::ReadPowerEventsEntry)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains('.') {
            continue;
        }
        if let Some(domain) = RaplDomain::from_perf_event(&name) {
            domains.insert(domain);
        }
    }
    Ok(domains)
}

fn read_online_cpus() -> Result<HashSet<u32>, RaplError> {
    let content = fs::read_to_string(ONLINE_CPU_SYSFS_PATH)
        .map_err(|e| RaplError::ReadOnlineCpus(ONLINE_CPU_SYSFS_PATH, e))?;
    let mut cpus = HashSet::new();
    for part in content.trim().split(',') {
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: u32 = start
                .parse()
                .map_err(|e| RaplError::InvalidCpuRange(part.to_string(), e))?;
            let end: u32 = end
                .parse()
                .map_err(|e| RaplError::InvalidCpuRange(part.to_string(), e))?;
            cpus.extend(start..=end);
        } else {
            cpus.insert(
                part.parse()
                    .map_err(|e| RaplError::InvalidCpuId(part.to_string(), e))?,
            );
        }
    }
    Ok(cpus)
}

fn discover_sockets() -> Result<Vec<SocketInfo>, RaplError> {
    let online = read_online_cpus()?;
    let mut sockets: HashMap<u32, u32> = HashMap::new();

    let entries =
        fs::read_dir(CPU_SYSFS_PATH).map_err(|e| RaplError::ReadCpuDir(CPU_SYSFS_PATH, e))?;
    for entry in entries {
        let entry = entry.map_err(RaplError::ReadCpuDirEntry)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix("cpu") else {
            continue;
        };
        let Ok(cpu_id) = suffix.parse::<u32>() else {
            continue;
        };
        if !online.contains(&cpu_id) {
            continue;
        }

        let socket_path = format!("{CPU_SYSFS_PATH}/{name}/{CPU_TOPOLOGY_SOCKET_ID}");
        let socket_id: u32 = fs::read_to_string(&socket_path)
            .map_err(|e| RaplError::ReadSocketId(socket_path.clone(), e))?
            .trim()
            .parse()
            .map_err(|e| RaplError::ParseSocketId(socket_path.clone(), e))?;

        sockets
            .entry(socket_id)
            .and_modify(|cpu| *cpu = (*cpu).min(cpu_id))
            .or_insert(cpu_id);
    }

    let mut sockets: Vec<SocketInfo> = sockets
        .into_iter()
        .map(|(id, cpu)| SocketInfo { id, cpu })
        .collect();
    sockets.sort_unstable_by_key(|s| s.id);
    Ok(sockets)
}
