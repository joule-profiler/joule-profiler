//! Energy read from `/sys/class/powercap`.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use joule_profiler_core::{Info, util::poller::Poller};

use crate::{
    backend::RaplBackend,
    domain::{RaplDomain, RaplDomainType},
    error::{RaplError, Result},
};

pub(crate) const POWERCAP_PATH: &str = "/sys/class/powercap";

/// `energy_uj` counts microjoules.
pub(crate) const JOULES_PER_COUNT: f64 = 1e-6;

/// The energy counter of a powercap zone.
pub(crate) struct PowercapZone {
    energy_file: PathBuf,

    /// The range given in `max_energy_range_uj`, which wraps to 0 when it reaches it.
    energy_range: u64,
}

impl PowercapZone {
    fn read_energy(&self) -> Result<u64> {
        let content =
            fs::read_to_string(&self.energy_file).map_err(|error| match error.kind() {
                ErrorKind::PermissionDenied => {
                    RaplError::PowercapPermissionDenied(self.energy_file.clone())
                }
                _ => RaplError::Read(self.energy_file.clone(), error),
            })?;

        content
            .trim()
            .parse()
            .map_err(|error| RaplError::NotANumber(self.energy_file.clone(), error))
    }
}

/// The energy of a counter, wraps at when an overflow occured at `range`.
struct EnergyCounter {
    range: u64,
    last_reading: u64,
    total: u64,
}

impl EnergyCounter {
    fn new(range: u64, first_reading: u64) -> Self {
        Self {
            range,
            last_reading: first_reading,
            total: 0,
        }
    }

    /// Adds the energy since the last reading to the total, across a wrap of the counter.
    fn update(&mut self, reading: u64) {
        let energy = if reading >= self.last_reading {
            reading - self.last_reading
        } else {
            (self.range - self.last_reading).saturating_add(reading)
        };

        self.total = self.total.wrapping_add(energy);
        self.last_reading = reading;
    }
}

/// Reads each domain from the `energy_uj` file of its zone, polled to follow its wraps.
pub(crate) struct PowercapBackend {
    domains: Vec<RaplDomain>,
    zones: Arc<[PowercapZone]>,
    poll_interval: Duration,
    counters: Arc<Mutex<Vec<EnergyCounter>>>,
    poller: Option<Poller<RaplError>>,
}

impl PowercapBackend {
    pub(crate) fn new(domains: Vec<(RaplDomain, PowercapZone)>, poll_interval: Duration) -> Self {
        let (domains, zones): (Vec<RaplDomain>, Vec<PowercapZone>) = domains.into_iter().unzip();

        Self {
            domains,
            zones: zones.into(),
            poll_interval,
            counters: Arc::default(),
            poller: None,
        }
    }
}

impl RaplBackend for PowercapBackend {
    fn info(&self) -> Info {
        Info::new()
            .with("backend", "powercap")
            .with("poll_interval", self.poll_interval)
    }

    fn domains(&self) -> &[RaplDomain] {
        &self.domains
    }

    fn open(&mut self) -> Result<()> {
        let counters = self
            .zones
            .iter()
            .map(|zone| Ok(EnergyCounter::new(zone.energy_range, zone.read_energy()?)))
            .collect::<Result<_>>()?;
        self.counters = Arc::new(Mutex::new(counters));

        let zones = Arc::clone(&self.zones);
        let counters = Arc::clone(&self.counters);
        self.poller = Some(Poller::start(self.poll_interval, move || {
            measure_zones(&zones, &counters)
        }));

        Ok(())
    }

    fn read(&mut self) -> Result<Vec<u64>> {
        measure_zones(&self.zones, &self.counters)?;
        let counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(counters.iter().map(|counter| counter.total).collect())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(poller) = self.poller.take() {
            poller.stop()?;
        }
        Ok(())
    }
}

/// Reads every zone and adds its energy to its counter.
fn measure_zones(zones: &[PowercapZone], counters: &Mutex<Vec<EnergyCounter>>) -> Result<()> {
    let mut counters = counters.lock().unwrap_or_else(PoisonError::into_inner);

    for (zone, counter) in zones.iter().zip(counters.iter_mut()) {
        counter.update(zone.read_energy()?);
    }
    Ok(())
}

/// Returns the domain of every RAPL zone under `root` and of its subzones.
pub(crate) fn discover_domains(root: &Path) -> Result<Vec<(RaplDomain, PowercapZone)>> {
    let mut domains = Vec::new();

    for package_dir in sorted_dirs(root)? {
        let is_package_zone = package_dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("intel-rapl:"))
            .is_some_and(|index| index.parse::<u32>().is_ok());
        if !is_package_zone {
            continue;
        }

        let Some(package_name) = read_domain_name(&package_dir) else {
            continue;
        };
        let Some((socket, die)) = parse_socket_and_die(&package_name) else {
            continue;
        };

        for zone_dir in std::iter::once(package_dir.clone()).chain(sorted_dirs(&package_dir)?) {
            let Some(domain_type) = read_domain_name(&zone_dir).and_then(|name| domain_type(&name))
            else {
                continue;
            };
            let Some(zone) = read_zone(&zone_dir) else {
                continue;
            };

            let domain = RaplDomain {
                domain_type,
                socket,
                die,
            };
            domains.push((domain, zone));
        }
    }

    Ok(domains)
}

/// Returns the socket and die of a powercap top-level zone, named `package-0`, `package-0-die-1`, or
/// `psys`.
fn parse_socket_and_die(name: &str) -> Option<(u32, Option<u32>)> {
    if name == "psys" {
        return Some((0, None));
    }

    let package = name.strip_prefix("package-")?;

    match package.split_once("-die-") {
        Some((socket, die)) => Some((socket.parse().ok()?, Some(die.parse().ok()?))),
        None => Some((package.parse().ok()?, None)),
    }
}

/// Returns the domain type of a powercap zone from its name.
fn domain_type(name: &str) -> Option<RaplDomainType> {
    match name {
        "core" => Some(RaplDomainType::Core),
        "uncore" => Some(RaplDomainType::Uncore),
        "dram" => Some(RaplDomainType::Dram),
        "psys" => Some(RaplDomainType::Psys),
        _ if name.starts_with("package-") => Some(RaplDomainType::Package),
        _ => None,
    }
}

fn read_domain_name(zone_dir: &Path) -> Option<String> {
    let name = fs::read_to_string(zone_dir.join("name")).ok()?;
    Some(name.trim().to_owned())
}

/// Reads the zone in `zone_dir`, if it has an `energy_uj` file.
fn read_zone(zone_dir: &Path) -> Option<PowercapZone> {
    let energy_file = zone_dir.join("energy_uj");
    if !energy_file.is_file() {
        return None;
    }

    let energy_range = fs::read_to_string(zone_dir.join("max_energy_range_uj"))
        .ok()
        .and_then(|range| range.trim().parse().ok())
        .unwrap_or(u64::MAX);

    Some(PowercapZone {
        energy_file,
        energy_range,
    })
}

fn sorted_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|error| RaplError::Read(root.into(), error))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    fn fake_zone(dir: &Path, name: &str, energy: u64) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("name"), format!("{name}\n")).unwrap();
        fs::write(dir.join("energy_uj"), format!("{energy}\n")).unwrap();
        fs::write(dir.join("max_energy_range_uj"), "1000\n").unwrap();
    }

    fn fake_root(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("joule-powercap-{test}-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn names(domains: &[(RaplDomain, PowercapZone)]) -> Vec<String> {
        domains.iter().map(|(domain, _)| domain.name()).collect()
    }

    #[test]
    fn discover_zones() {
        let root = fake_root("discover");
        fake_zone(&root.join("intel-rapl:0"), "package-0", 0);
        fake_zone(&root.join("intel-rapl:0/intel-rapl:0:0"), "core", 0);
        fake_zone(&root.join("intel-rapl:0:0"), "core", 0);
        fake_zone(&root.join("intel-rapl:1"), "psys", 0);
        fake_zone(&root.join("intel-rapl-mmio:0"), "package-0", 0);

        let domains = discover_domains(&root);
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(names(&domains.unwrap()), ["PACKAGE-0", "CORE-0", "PSYS"]);
    }

    #[test]
    fn die_from_zone_name() {
        let root = fake_root("dies");
        fake_zone(&root.join("intel-rapl:0"), "package-0-die-0", 0);
        fake_zone(&root.join("intel-rapl:1"), "package-0-die-1", 0);
        fake_zone(&root.join("intel-rapl:1/intel-rapl:1:0"), "dram", 0);

        let domains = discover_domains(&root);
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(
            names(&domains.unwrap()),
            ["PACKAGE-0-DIE-0", "PACKAGE-0-DIE-1", "DRAM-0-DIE-1"]
        );
    }

    #[test]
    fn socket_and_die_from_name() {
        assert_eq!(parse_socket_and_die("package-1"), Some((1, None)));
        assert_eq!(parse_socket_and_die("package-0-die-1"), Some((0, Some(1))));
        assert_eq!(parse_socket_and_die("psys"), Some((0, None)));
    }

    #[test]
    fn malformed_zone_ignored() {
        for name in ["package-x", "package-0-die-", "package-", "core", ""] {
            assert_eq!(parse_socket_and_die(name), None, "{name}");
        }
    }

    #[test]
    fn counter_wrap() {
        let mut counter = EnergyCounter::new(1000, 900);

        counter.update(950);
        counter.update(100);

        assert_eq!(counter.total, 200);
    }
}
