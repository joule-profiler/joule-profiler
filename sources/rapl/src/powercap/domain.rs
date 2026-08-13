use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::Result;
use crate::domain_type::RaplDomainType;
use crate::error::RaplError;
use crate::powercap::socket::parse_or_all_sockets;
use log::{debug, error, info, trace, warn};

/// Represents a RAPL (Running Average Power Limit) energy domain.
#[derive(Debug, Clone)]
pub struct RaplDomain {
    /// Path to the `energy_uj` file for reading current energy counter
    pub path: PathBuf,

    /// Type of the domain (e.g., "package", "core", "dram", "psys")
    pub domain_type: RaplDomainType,

    /// CPU socket index this domain belongs to
    pub socket: u32,

    /// Maximum energy range in microjoules (for overflow detection)
    pub max_energy_uj: u64,
}

impl RaplDomain {
    /// Create a new RAPL domain from the given path, name, socket, and max energy
    ///
    /// Converts the string name into a `RaplDomainType`
    pub fn try_new(path: PathBuf, name: &str, socket: u32, max_energy_uj: u64) -> Result<Self> {
        let domain = Self {
            path,
            domain_type: RaplDomainType::try_from(name)?,
            socket,
            max_energy_uj,
        };
        Ok(domain)
    }

    /// Get the RAPL domain name
    pub fn get_name(&self) -> String {
        self.domain_type.to_string_socket(self.socket)
    }
}

/// Retrieve the RAPL domains on the machine, filtered with spec if one is provided.
pub fn get_domains(base_path: &str, spec: Option<&HashSet<u32>>) -> Result<Vec<RaplDomain>> {
    let domains = discover_domains(base_path)?;
    let sockets = parse_or_all_sockets(&domains, spec);

    let filtered: Vec<RaplDomain> = domains
        .into_iter()
        .filter(|d| sockets.contains(&d.socket))
        .collect();

    Ok(filtered)
}

/// Discovers all available RAPL domains at the given base path.
fn discover_domains(base: &str) -> Result<Vec<RaplDomain>> {
    info!("Discovering RAPL domains in {base}");

    let mut domains = Vec::new();

    let entries = fs::read_dir(base).map_err(|err| {
        error!("Failed to read RAPL base directory: {err}");
        if err.kind() == ErrorKind::PermissionDenied {
            RaplError::InsufficientPermissions
        } else {
            RaplError::RaplReadError(format!("Failed to read {base}: {err}"))
        }
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            trace!("Skipping non-directory {}", path.display());
            continue;
        }

        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };

        if !name.starts_with("intel-rapl:") {
            trace!("Skipping unrelated directory {name}");
            continue;
        }

        add_domain_if_energy(&path, &mut domains)?;

        for sub in fs::read_dir(&path)? {
            let sub = sub?;
            let sub_path = sub.path();
            if sub_path.is_dir() {
                add_domain_if_energy(&sub_path, &mut domains)?;
            }
        }
    }

    if domains.is_empty() {
        warn!("No RAPL domains found");
        return Err(RaplError::NoDomains);
    }

    info!("Discovered {} RAPL domains", domains.len());
    Ok(domains)
}

/// Adds a RAPL domain to the output vector if it contains an `energy_uj` and a `max_energy_uj` files.
fn add_domain_if_energy(dir: &Path, out: &mut Vec<RaplDomain>) -> Result<()> {
    let path = dir.join("energy_uj");
    if !path.exists() {
        trace!("No energy_uj in {}", dir.display());
        return Ok(());
    }

    let name = match fs::read_to_string(dir.join("name")) {
        Ok(n) => n,
        Err(e) => match e.kind() {
            ErrorKind::NotFound => {
                warn!(
                    "Directory {} missing energy_uj file, ignored.",
                    dir.display()
                );
                return Ok(());
            }
            _ => return Err(e)?,
        },
    };
    let name_trimmed = name.trim();

    let socket = extract_socket_number(dir);

    let max_energy_uj_option = dir
        .join("max_energy_range_uj")
        .exists()
        .then(|| {
            fs::read_to_string(dir.join("max_energy_range_uj"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .flatten();

    if let Some(max_energy_uj) = max_energy_uj_option {
        debug!("Found domain: name={name_trimmed}, socket={socket}, max_energy_uj={max_energy_uj}");
        let domain = RaplDomain::try_new(path, name_trimmed, socket, max_energy_uj)?;
        out.push(domain);
    } else {
        warn!(
            "Domain {} missing max_energy_range_uj, ignored.",
            dir.display()
        );
    }

    Ok(())
}

/// Extracts the socket number from a RAPL domain path.
fn extract_socket_number(path: &Path) -> u32 {
    for comp in path.components() {
        if let Component::Normal(os) = comp
            && let Some(s) = os.to_str()
            && let Some(rest) = s.strip_prefix("intel-rapl:")
            && let Some(idx) = rest.split(':').next()
            && let Ok(n) = idx.parse::<u32>()
        {
            return n;
        }
    }
    0
}

/// Reads the current energy counter value from a RAPL domain.
pub fn read_energy(domain: &RaplDomain) -> Result<u64> {
    trace!("Reading energy for domain {}", domain.domain_type);
    let content = fs::read_to_string(&domain.path)?;

    let energy = content
        .trim()
        .parse()
        .map_err(RaplError::ParseEnergyError)?;

    trace!("Energy {} = {} µJ", domain.get_name(), energy);
    Ok(energy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};
    use tempfile::tempdir;

    fn make_domain_dir(
        base: &Path,
        name: &str,
        socket: u32,
        energy: u64,
        max_energy: u64,
    ) -> PathBuf {
        let dir = base.join(format!("intel-rapl:{socket}"));
        create_dir_all(&dir).unwrap();

        write(dir.join("name"), name).unwrap();
        write(dir.join("energy_uj"), energy.to_string()).unwrap();
        write(dir.join("max_energy_range_uj"), max_energy.to_string()).unwrap();
        dir
    }

    #[test]
    fn extract_socket_number_from_path() {
        let path = Path::new("/sys/devices/intel-rapl:2/intel-rapl:2:0");
        let socket = extract_socket_number(path);
        assert_eq!(socket, 2);
    }

    #[test]
    fn extract_socket_number_defaults_to_zero() {
        let path = Path::new("/weird/path/no-socket");
        let socket = extract_socket_number(path);
        assert_eq!(socket, 0);
    }

    #[test]
    fn discover_domains_finds_valid_domain() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        make_domain_dir(base, "package", 0, 100, 1_000);

        let domains = discover_domains(base.to_str().unwrap()).unwrap();

        assert_eq!(domains.len(), 1);
        let d = &domains[0];
        assert_eq!(d.get_name(), "PACKAGE-0");
        assert_eq!(d.socket, 0);
        assert_eq!(d.max_energy_uj, 1_000);
    }

    #[test]
    fn discover_domains_ignores_missing_max_energy() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        let domain = base.join("intel-rapl:0");
        create_dir_all(&domain).unwrap();
        write(domain.join("energy_uj"), "100").unwrap();

        let err = discover_domains(base.to_str().unwrap())
            .unwrap_err()
            .to_string();

        assert!(err.contains("No RAPL domains found"));
    }

    #[test]
    fn read_energy_reads_valid_value() {
        let dir = tempdir().unwrap();
        let energy_file = dir.path().join("energy_uj");
        write(&energy_file, "12345").unwrap();

        let domain = RaplDomain {
            path: energy_file,
            domain_type: RaplDomainType::Package,
            socket: 0,
            max_energy_uj: 1_000,
        };

        let energy = read_energy(&domain).unwrap();
        assert_eq!(energy, 12345);
    }

    #[test]
    fn read_energy_invalid_value_fails() {
        let dir = tempdir().unwrap();
        let energy_file = dir.path().join("energy_uj");
        write(&energy_file, "abc").unwrap();

        let domain = RaplDomain {
            path: energy_file,
            domain_type: RaplDomainType::Package,
            socket: 0,
            max_energy_uj: 1_000,
        };

        let err = read_energy(&domain).unwrap_err();
        assert!(matches!(err, RaplError::ParseEnergyError(_)));
    }
}
