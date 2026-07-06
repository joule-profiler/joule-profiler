use std::{
    collections::{HashMap, HashSet},
    fs,
};

use log::{debug, trace};
use perf_event::Group;

use crate::{Result, perf::domain::PerfRaplDomain};

/// Path to CPU sysfs directory.
const CPU_SYSFS_PATH: &str = "/sys/devices/system/cpu";

/// Relative path to the physical package (socket) ID in sysfs.
const CPU_TOPOLOGY_SOCKET_ID: &str = "/topology/physical_package_id";

/// Path to the online CPUs file in sysfs.
const ONLINE_CPU_SYSFS_PATH: &str = "/sys/devices/system/cpu/online";

/// Represents a discovered CPU socket that has not been opened yet: its id
/// and the CPUs it contains (see [`crate::perf::event::open_counters`] to
/// open it).
pub struct SocketInfo {
    /// The id of the socket.
    pub id: u32,

    /// List of CPU IDs associated with this socket.
    pub cpus: Vec<u32>,
}

/// Represents an opened CPU socket: its id, its `perf_event` group, and the
/// RAPL domains discovered on it.
pub struct Socket {
    /// The id of the socket.
    pub id: u32,

    /// The group of counters to be able to manage them all at once.
    pub group: Group,

    /// The RAPL domains supported by the hardware.
    pub domains: Vec<PerfRaplDomain>,
}

/// Discover the CPU socket topology of the system with an optional filter to discover only specific socket IDs.
///
/// It returns a list containing each discovered socket and its CPUs, not yet
/// opened (see [`crate::perf::event::open_counters`]).
/// An error occurs if reading sysfs files fails or parsing fails.
pub fn discover_socket_topology(
    sockets_to_discover: Option<&HashSet<u32>>,
) -> Result<Vec<SocketInfo>> {
    discover_socket_topology_from_path(CPU_SYSFS_PATH, ONLINE_CPU_SYSFS_PATH, sockets_to_discover)
}

pub(crate) fn discover_socket_topology_from_path(
    cpu_sysfs_path: &str,
    online_cpu_path: &str,
    sockets_to_discover: Option<&HashSet<u32>>,
) -> Result<Vec<SocketInfo>> {
    let mut sockets: HashMap<u32, Vec<u32>> = HashMap::new();
    let online_cpus = read_online_cpus(online_cpu_path)?;
    trace!("Found {} cpu(s)", online_cpus.len());

    for entry in fs::read_dir(cpu_sysfs_path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();

        if !name.starts_with("cpu") {
            continue;
        }

        let cpu_id: u32 = if let Ok(id) = name.chars().skip(3).collect::<String>().parse() {
            id
        } else {
            continue;
        };

        if !online_cpus.contains(&cpu_id) {
            continue;
        }

        let pkg_path = format!("{cpu_sysfs_path}/{name}{CPU_TOPOLOGY_SOCKET_ID}");
        let socket_id: u32 = fs::read_to_string(pkg_path)?.trim().parse()?;

        if let Some(sockets) = sockets_to_discover
            && !sockets.contains(&socket_id)
        {
            continue;
        }

        sockets.entry(socket_id).or_default().push(cpu_id);
    }

    let mut socket_topology: Vec<SocketInfo> = sockets
        .into_iter()
        .map(|(id, mut cpus)| {
            trace!("Found {cpus:?} cpus for socket {id}");
            cpus.sort_unstable();
            SocketInfo { id, cpus }
        })
        .collect();

    socket_topology.sort_unstable_by_key(|s| s.id);

    debug!("Discovered {} socket(s)", socket_topology.len());
    Ok(socket_topology)
}

/// Read the list of online CPUs from sysfs.
///
/// Returns a set containing the IDs of all online CPUs.
/// An error occurs if the sysfs file cannot be read or if parsing fails.
pub(crate) fn read_online_cpus(path: &str) -> Result<HashSet<u32>> {
    let content = fs::read_to_string(path)?;
    parse_online_cpus(&content)
}

pub(crate) fn parse_online_cpus(content: &str) -> Result<HashSet<u32>> {
    let mut cpus = HashSet::new();
    for part in content.trim().split(',') {
        if let Some((start, end)) = part.split_once('-') {
            let start: u32 = start.parse()?;
            let end: u32 = end.parse()?;
            cpus.extend(start..=end);
        } else {
            let cpu: u32 = part.parse()?;
            cpus.insert(cpu);
        }
    }
    Ok(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parse_online_cpus_single() {
        let cpus = parse_online_cpus("0").unwrap();
        assert_eq!(cpus, HashSet::from([0]));
    }

    #[test]
    fn parse_online_cpus_list() {
        let cpus = parse_online_cpus("0,1,2,3").unwrap();
        assert_eq!(cpus, HashSet::from([0, 1, 2, 3]));
    }

    #[test]
    fn parse_online_cpus_range() {
        let cpus = parse_online_cpus("0-3").unwrap();
        assert_eq!(cpus, HashSet::from([0, 1, 2, 3]));
    }

    #[test]
    fn parse_online_cpus_mixed_range_and_list() {
        let cpus = parse_online_cpus("0-2,4,6-7").unwrap();
        assert_eq!(cpus, HashSet::from([0, 1, 2, 4, 6, 7]));
    }

    #[test]
    fn parse_online_cpus_trailing_newline() {
        let cpus = parse_online_cpus("0-3\n").unwrap();
        assert_eq!(cpus, HashSet::from([0, 1, 2, 3]));
    }

    #[test]
    fn parse_online_cpus_invalid_content_returns_error() {
        assert!(parse_online_cpus("not-a-number").is_err());
        assert!(parse_online_cpus("0,abc,2").is_err());
    }

    struct DummySysfs {
        dir: TempDir,
    }

    impl DummySysfs {
        fn new(cpus: &[(u32, u32)]) -> Self {
            let dir = TempDir::new().unwrap();
            let base = dir.path();

            let online: String = cpus
                .iter()
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            fs::write(base.join("online"), format!("{}\n", online)).unwrap();

            for (cpu_id, socket_id) in cpus {
                let topo = base.join(format!("cpu{}/topology", cpu_id));
                fs::create_dir_all(&topo).unwrap();
                fs::write(topo.join("physical_package_id"), format!("{}\n", socket_id)).unwrap();
            }

            fs::create_dir_all(base.join("cpufreq")).unwrap();

            Self { dir }
        }

        fn cpu_path(&self) -> &str {
            self.dir.path().to_str().unwrap()
        }

        fn online_path(&self) -> String {
            self.dir.path().join("online").to_str().unwrap().to_owned()
        }
    }

    #[test]
    fn discover_single_socket() {
        let sysfs = DummySysfs::new(&[(0, 0), (1, 0), (2, 0), (3, 0)]);
        let topology =
            discover_socket_topology_from_path(sysfs.cpu_path(), &sysfs.online_path(), None)
                .unwrap();

        assert_eq!(topology.len(), 1);
        assert_eq!(topology[0].id, 0);
        assert_eq!(topology[0].cpus, vec![0, 1, 2, 3]);
    }

    #[test]
    fn discover_two_sockets() {
        // cpu0, cpu1 = socket 0 / cpu2, cpu3 = socket 1
        let sysfs = DummySysfs::new(&[(0, 0), (1, 0), (2, 1), (3, 1)]);
        let topology =
            discover_socket_topology_from_path(sysfs.cpu_path(), &sysfs.online_path(), None)
                .unwrap();

        assert_eq!(topology.len(), 2);
        assert_eq!(topology[0].id, 0);
        assert_eq!(topology[0].cpus, vec![0, 1]);
        assert_eq!(topology[1].id, 1);
        assert_eq!(topology[1].cpus, vec![2, 3]);
    }

    #[test]
    fn discover_cpu_ids_are_sorted() {
        let sysfs = DummySysfs::new(&[(3, 0), (1, 0), (0, 0), (2, 0)]);
        let topology =
            discover_socket_topology_from_path(sysfs.cpu_path(), &sysfs.online_path(), None)
                .unwrap();

        assert_eq!(topology[0].cpus, vec![0, 1, 2, 3]);
    }

    #[test]
    fn discover_with_socket_filter_keeps_only_requested() {
        let sysfs = DummySysfs::new(&[(0, 0), (1, 0), (2, 1), (3, 1)]);
        let filter = HashSet::from([1u32]);
        let topology = discover_socket_topology_from_path(
            sysfs.cpu_path(),
            &sysfs.online_path(),
            Some(&filter),
        )
        .unwrap();

        assert_eq!(topology.len(), 1);
        assert_eq!(topology[0].id, 1);
        assert_eq!(topology[0].cpus, vec![2, 3]);
    }

    #[test]
    fn discover_with_filter_matching_no_socket_returns_empty() {
        let sysfs = DummySysfs::new(&[(0, 0), (1, 0)]);
        let filter = HashSet::from([99u32]);
        let topology = discover_socket_topology_from_path(
            sysfs.cpu_path(),
            &sysfs.online_path(),
            Some(&filter),
        )
        .unwrap();

        assert!(topology.is_empty());
    }

    #[test]
    fn discover_offline_cpus_are_excluded() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // only cpu0 is online and cpu1 exists but must be ignored
        fs::write(base.join("online"), "0\n").unwrap();
        for (cpu_id, socket_id) in [(0u32, 0u32), (1, 0)] {
            let topo = base.join(format!("cpu{}/topology", cpu_id));
            fs::create_dir_all(&topo).unwrap();
            fs::write(topo.join("physical_package_id"), format!("{}\n", socket_id)).unwrap();
        }

        let online_path = base.join("online").to_str().unwrap().to_owned();
        let topology =
            discover_socket_topology_from_path(base.to_str().unwrap(), &online_path, None).unwrap();

        assert_eq!(topology[0].cpus, vec![0]);
    }

    #[test]
    fn discover_non_cpu_directories_are_ignored() {
        let sysfs = DummySysfs::new(&[(0, 0)]);
        let topology =
            discover_socket_topology_from_path(sysfs.cpu_path(), &sysfs.online_path(), None)
                .unwrap();

        assert_eq!(topology.len(), 1);
        assert_eq!(topology[0].cpus, vec![0]);
    }

    #[test]
    fn discover_missing_online_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let result = discover_socket_topology_from_path(
            dir.path().to_str().unwrap(),
            "/nonexistent/online",
            None,
        );
        assert!(result.is_err());
    }
}
