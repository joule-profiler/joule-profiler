//! Energy read from `/sys/class/powercap`, in microjoules.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::source::Source;
use joule_profiler_core::unit::MetricUnit;

use crate::domain::{EnergyUnit, RaplDomainType};
use crate::error::{RaplError, Result};

const POWERCAP: &str = "/sys/class/powercap";

/// One powercap zone.
#[derive(Debug, Clone)]
pub struct PowercapDomain {
    pub domain: RaplDomainType,
    pub socket: u32,
    path: PathBuf,

    /// The value the counter wraps at.
    max_uj: u64,

    /// From microjoules to the reported unit.
    factor: f64,
}

impl PowercapDomain {
    pub fn name(&self) -> String {
        self.domain.to_string_socket(self.socket)
    }

    fn energy_uj(&self) -> Result<u64> {
        read_energy(&self.path)
    }
}

pub type PowercapDomains = Arc<[PowercapDomain]>;

/// Every `intel-rapl:<socket>` zone and its subzones.
fn discover(unit: EnergyUnit) -> Result<Vec<PowercapDomain>> {
    let mut found = Vec::new();

    for package in sorted_dirs(Path::new(POWERCAP))? {
        let Some(socket) = package
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("intel-rapl:"))
            .and_then(|rest| rest.parse::<u32>().ok())
        else {
            continue;
        };

        for dir in std::iter::once(package.clone()).chain(sorted_dirs(&package)?) {
            if let Some(domain) = read_domain(&dir, socket, unit) {
                found.push(domain);
            }
        }
    }

    Ok(found)
}

fn read_energy(dir: &Path) -> Result<u64> {
    let path = dir.join("energy_uj");

    let content = fs::read_to_string(&path).map_err(|error| match error.kind() {
        ErrorKind::PermissionDenied => RaplError::PowercapDenied(path.clone()),
        _ => RaplError::Io(error),
    })?;

    content
        .trim()
        .parse()
        .map_err(|_| RaplError::NotANumber(path))
}

/// A zone with a known name and an `energy_uj` file. The file is not read here, so that
/// listing works without the rights to measure.
fn read_domain(dir: &Path, socket: u32, unit: EnergyUnit) -> Option<PowercapDomain> {
    let name = fs::read_to_string(dir.join("name")).ok()?;
    let domain = RaplDomainType::try_from(name.trim()).ok()?;

    if !dir.join("energy_uj").is_file() {
        return None;
    }

    let max_uj = fs::read_to_string(dir.join("max_energy_range_uj"))
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(u64::MAX);

    Some(PowercapDomain {
        domain,
        socket,
        path: dir.to_path_buf(),
        max_uj,
        factor: unit.per_joule() / 1e6,
    })
}

fn sorted_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .map_err(RaplError::Io)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    dirs.sort();
    Ok(dirs)
}

/// The increase of a counter that wraps at `max`.
fn moved(previous: u64, current: u64, max: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        (max - previous).saturating_add(current)
    }
}

pub struct RaplPowercapSensor {
    domains: PowercapDomains,
}

impl RaplPowercapSensor {
    fn new(domains: PowercapDomains) -> Self {
        Self { domains }
    }
}

impl Sensor for RaplPowercapSensor {
    /// One microjoule counter per domain.
    type Snapshot = Vec<u64>;

    type Error = RaplError;

    fn name(&self) -> &'static str {
        "rapl"
    }

    fn measure(&mut self) -> Result<Self::Snapshot> {
        self.domains.iter().map(PowercapDomain::energy_uj).collect()
    }
}

pub struct RaplPowercapProcessor {
    domains: PowercapDomains,
    unit: MetricUnit,
}

impl RaplPowercapProcessor {
    fn new(domains: PowercapDomains, unit: MetricUnit) -> Self {
        Self { domains, unit }
    }
}

impl Processor<RaplPowercapSensor> for RaplPowercapProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.domains
            .iter()
            .map(|domain| MetricInfo::new(domain.name(), self.unit))
            .collect()
    }

    fn info(&self) -> Info {
        let domains = InfoValue::list(self.domains.iter().map(PowercapDomain::name));

        Info::new()
            .with("backend", "powercap")
            .with("domains", domains)
            .with("unit", self.unit.to_string())
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "a phase never accumulates 2^52 microjoules"
    )]
    fn process(
        &mut self,
        previous: &Vec<u64>,
        current: &Vec<u64>,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        values.extend(self.domains.iter().zip(current.iter().zip(previous)).map(
            |(domain, (current, previous))| {
                MetricValue::F64(moved(*previous, *current, domain.max_uj) as f64 * domain.factor)
            },
        ));

        Ok(())
    }
}

/// PSYS has no socket, so only the domain filter applies to it.
pub(crate) fn build(
    unit: EnergyUnit,
    sockets: Option<&HashSet<u32>>,
    domains: Option<&HashSet<RaplDomainType>>,
) -> Result<Source> {
    let found = discover(unit)?;

    if let Some(unknown) = sockets.and_then(|wanted| {
        wanted
            .iter()
            .find(|socket| !found.iter().any(|domain| domain.socket == **socket))
    }) {
        return Err(RaplError::UnknownSocket(*unknown));
    }

    let domains: PowercapDomains = found
        .into_iter()
        .filter(|found| domains.is_none_or(|wanted| wanted.contains(&found.domain)))
        .filter(|found| {
            found.domain == RaplDomainType::Psys
                || sockets.is_none_or(|wanted| wanted.contains(&found.socket))
        })
        .collect();

    if domains.is_empty() {
        return Err(RaplError::NoDomain);
    }

    log::debug!("rapl: {} powercap domains discovered", domains.len());

    Ok(Source::new(
        RaplPowercapSensor::new(Arc::clone(&domains)),
        RaplPowercapProcessor::new(domains, unit.metric_unit()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_domains_and_the_unit_are_in_the_info() {
        let domains = Arc::new([PowercapDomain {
            domain: RaplDomainType::Package,
            socket: 0,
            path: PathBuf::from("/sys/class/powercap/intel-rapl:0"),
            max_uj: u64::MAX,
            factor: 1.0,
        }]);

        let info = RaplPowercapProcessor::new(domains, MetricUnit::MILLIJOULE).info();

        assert_eq!(info.get("backend"), Some(&"powercap".into()));
        assert_eq!(info.get("domains"), Some(&InfoValue::list(["PACKAGE-0"])));
        assert_eq!(info.get("unit"), Some(&"mJ".into()));
    }

    #[test]
    fn a_counter_that_wrapped_still_reads_as_what_it_moved() {
        assert_eq!(moved(100, 250, 1_000), 150);
        assert_eq!(moved(900, 100, 1_000), 200);
        assert_eq!(moved(900, 0, 1_000), 100);
    }
}
