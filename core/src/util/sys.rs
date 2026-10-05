use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const CPU_ROOT: &str = "/sys/devices/system/cpu";
const PACKAGE_ID: &str = "topology/physical_package_id";
const CPUINFO: &str = "/proc/cpuinfo";
const MODEL_NAME: &str = "model name";

pub fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// A physical package and its logical CPUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socket {
    pub id: u32,
    pub cpus: Vec<u32>,
}

impl Socket {
    pub fn holds(&self, cpu: u32) -> bool {
        self.cpus.binary_search(&cpu).is_ok()
    }
}

/// The sockets of the machine. Offline CPUs are left out.
pub fn socket_topology() -> io::Result<Vec<Socket>> {
    read_topology(Path::new(CPU_ROOT))
}

fn read_topology(root: &Path) -> io::Result<Vec<Socket>> {
    let mut sockets: Vec<Socket> = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;

        let Some(cpu) = cpu_number(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Some(id) = read_package(&entry.path().join(PACKAGE_ID))? else {
            continue;
        };

        match sockets.iter_mut().find(|socket| socket.id == id) {
            Some(socket) => socket.cpus.push(cpu),
            None => sockets.push(Socket {
                id,
                cpus: vec![cpu],
            }),
        }
    }

    sockets.sort_unstable_by_key(|socket| socket.id);
    for socket in &mut sockets {
        socket.cpus.sort_unstable();
    }

    Ok(sockets)
}

fn cpu_number(name: &str) -> Option<u32> {
    name.strip_prefix("cpu")?.parse().ok()
}

/// Reads a file in the kernel CPU list format, such as `0-3,8,10-11`.
pub fn read_cpu_list(path: &Path) -> io::Result<Vec<u32>> {
    parse_cpu_list(&fs::read_to_string(path)?).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a CPU list", path.display()),
        )
    })
}

fn parse_cpu_list(list: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();

    for part in list.trim().split(',').filter(|part| !part.is_empty()) {
        match part.split_once('-') {
            Some((start, end)) => {
                cpus.extend(start.trim().parse::<u32>().ok()?..=end.trim().parse().ok()?);
            }
            None => cpus.push(part.trim().parse().ok()?),
        }
    }

    Some(cpus)
}

/// Writes CPUs in the kernel CPU list format, such as `0-3,8,10-11`.
pub fn cpu_list(cpus: &[u32]) -> String {
    let mut list = String::new();
    let mut cpus = cpus.iter().copied().peekable();

    while let Some(start) = cpus.next() {
        let mut end = start;
        while let Some(next) = cpus.next_if(|&cpu| end.checked_add(1) == Some(cpu)) {
            end = next;
        }

        if !list.is_empty() {
            list.push(',');
        }
        let _ = if start == end {
            write!(list, "{start}")
        } else {
            write!(list, "{start}-{end}")
        };
    }

    list
}

/// The model name of the first CPU in `/proc/cpuinfo`.
pub fn cpu_model() -> io::Result<String> {
    read_model(BufReader::new(File::open(CPUINFO)?))
}

fn read_model(cpuinfo: impl BufRead) -> io::Result<String> {
    for line in cpuinfo.lines() {
        if let Some((key, model)) = line?.split_once(':')
            && key.trim() == MODEL_NAME
        {
            return Ok(model.trim().to_owned());
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{CPUINFO} names no {MODEL_NAME}"),
    ))
}

fn read_package(path: &Path) -> io::Result<Option<u32>> {
    let id = match fs::read_to_string(path) {
        Ok(id) => id,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    id.trim().parse().map(Some).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad package id: {id:?}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_machine(name: &str, cpus: &[(u32, Option<u32>)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("joule-topo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        for (cpu, package) in cpus {
            let dir = root.join(format!("cpu{cpu}")).join("topology");
            fs::create_dir_all(&dir).unwrap();

            if let Some(package) = package {
                fs::write(dir.join("physical_package_id"), format!("{package}\n")).unwrap();
            }
        }

        fs::create_dir_all(root.join("cpufreq")).unwrap();
        root
    }

    #[test]
    fn cpus_are_grouped_by_package() {
        let root = fake_machine(
            "grouped",
            &[(0, Some(0)), (1, Some(1)), (2, Some(0)), (3, Some(1))],
        );

        let sockets = read_topology(&root).unwrap();

        assert_eq!(
            sockets,
            vec![
                Socket {
                    id: 0,
                    cpus: vec![0, 2]
                },
                Socket {
                    id: 1,
                    cpus: vec![1, 3]
                },
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cpu_without_a_package_is_left_out() {
        let root = fake_machine("offline", &[(0, Some(0)), (1, None)]);

        let sockets = read_topology(&root).unwrap();

        assert_eq!(
            sockets,
            vec![Socket {
                id: 0,
                cpus: vec![0]
            }]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn directories_that_are_not_cpus_are_ignored() {
        let root = fake_machine("neighbours", &[(0, Some(0))]);

        assert_eq!(read_topology(&root).unwrap().len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_socket_knows_which_cpus_it_holds() {
        let socket = Socket {
            id: 0,
            cpus: vec![0, 2, 4],
        };

        assert!(socket.holds(2));
        assert!(!socket.holds(3));
    }

    #[test]
    fn a_cpu_list_reads_its_ranges_and_single_cpus() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11\n"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(parse_cpu_list(""), Some(vec![]));
        assert_eq!(parse_cpu_list("0-x"), None);
    }

    #[test]
    fn consecutive_cpus_are_written_as_a_range() {
        assert_eq!(cpu_list(&[0, 1, 2, 3, 8, 10, 11]), "0-3,8,10-11");
        assert_eq!(cpu_list(&[5]), "5");
        assert_eq!(cpu_list(&[]), "");
    }

    #[test]
    fn the_model_is_that_of_the_first_processor() {
        let cpuinfo = "processor\t: 0\nvendor_id\t: GenuineIntel\n\
                       model name\t: Intel(R) Xeon(R) Gold 5220\n\n\
                       processor\t: 1\nmodel name\t: something else\n";

        assert_eq!(
            read_model(cpuinfo.as_bytes()).unwrap(),
            "Intel(R) Xeon(R) Gold 5220"
        );
    }

    #[test]
    fn a_cpuinfo_that_names_no_model_is_an_error() {
        let error = read_model("processor\t: 0\nCPU part\t: 0xd0c\n".as_bytes()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
