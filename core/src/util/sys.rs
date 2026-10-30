use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::Path,
};

const CPU_PATH: &str = "/sys/devices/system/cpu";
const PACKAGE_ID: &str = "topology/physical_package_id";
const DIE_ID: &str = "topology/die_id";
const CPUINFO_FILE_PATH: &str = "/proc/cpuinfo";
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
    pub dies: Vec<Die>,
}

/// A die of a package and its logical CPUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Die {
    pub id: u32,
    pub cpus: Vec<u32>,
}

impl Socket {
    pub fn has_cpu(&self, cpu: u32) -> bool {
        self.cpus.binary_search(&cpu).is_ok()
    }
}

/// Returns the sockets of the machine, without the offline CPUs.
pub fn socket_topology() -> io::Result<Vec<Socket>> {
    read_topology(Path::new(CPU_PATH))
}

fn read_topology(root: &Path) -> io::Result<Vec<Socket>> {
    let mut sockets: Vec<Socket> = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;

        let Some(cpu) = cpu_number(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Some(id) = read_id(&entry.path().join(PACKAGE_ID))? else {
            continue;
        };
        // Before Linux 5.3, there is no `die_id` and a package is a single die.
        let die_id = read_id(&entry.path().join(DIE_ID))?.unwrap_or(0);

        let index = if let Some(index) = sockets.iter().position(|socket| socket.id == id) {
            index
        } else {
            sockets.push(Socket {
                id,
                cpus: Vec::new(),
                dies: Vec::new(),
            });
            sockets.len() - 1
        };
        let socket = &mut sockets[index];
        socket.cpus.push(cpu);

        match socket.dies.iter_mut().find(|die| die.id == die_id) {
            Some(die) => die.cpus.push(cpu),
            None => socket.dies.push(Die {
                id: die_id,
                cpus: vec![cpu],
            }),
        }
    }

    sockets.sort_unstable_by_key(|socket| socket.id);
    for socket in &mut sockets {
        socket.cpus.sort_unstable();
        socket.dies.sort_unstable_by_key(|die| die.id);
        for die in &mut socket.dies {
            die.cpus.sort_unstable();
        }
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

/// Returns the model name of the first CPU.
pub fn cpu_model() -> io::Result<String> {
    read_model(BufReader::new(File::open(CPUINFO_FILE_PATH)?))
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
        format!("{CPUINFO_FILE_PATH} names no {MODEL_NAME}"),
    ))
}

fn read_id(path: &Path) -> io::Result<Option<u32>> {
    let id = match fs::read_to_string(path) {
        Ok(id) => id,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    id.trim().parse().map(Some).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad id in {}: {id:?}", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_machine(name: &str, cpus: &[(u32, Option<u32>, Option<u32>)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("joule-topo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        for (cpu, package, die) in cpus {
            let dir = root.join(format!("cpu{cpu}")).join("topology");
            fs::create_dir_all(&dir).unwrap();

            if let Some(package) = package {
                fs::write(dir.join("physical_package_id"), format!("{package}\n")).unwrap();
            }
            if let Some(die) = die {
                fs::write(dir.join("die_id"), format!("{die}\n")).unwrap();
            }
        }

        fs::create_dir_all(root.join("cpufreq")).unwrap();
        root
    }

    #[test]
    fn groups_cpus_by_package() {
        let root = fake_machine(
            "grouped",
            &[
                (0, Some(0), Some(0)),
                (1, Some(1), Some(0)),
                (2, Some(0), Some(0)),
                (3, Some(1), Some(0)),
            ],
        );

        let sockets = read_topology(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let cpus: Vec<(u32, Vec<u32>)> = sockets
            .into_iter()
            .map(|socket| (socket.id, socket.cpus))
            .collect();
        assert_eq!(cpus, [(0, vec![0, 2]), (1, vec![1, 3])]);
    }

    #[test]
    fn groups_cpus_by_die() {
        let root = fake_machine(
            "dies",
            &[
                (0, Some(0), Some(0)),
                (1, Some(0), Some(1)),
                (2, Some(0), Some(0)),
                (3, Some(0), Some(1)),
            ],
        );

        let sockets = read_topology(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(
            sockets[0].dies,
            [
                Die {
                    id: 0,
                    cpus: vec![0, 2]
                },
                Die {
                    id: 1,
                    cpus: vec![1, 3]
                },
            ]
        );
    }

    #[test]
    fn single_die_before_linux_5_3() {
        let root = fake_machine("nodie", &[(0, Some(0), None), (1, Some(0), None)]);

        let sockets = read_topology(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(
            sockets[0].dies,
            [Die {
                id: 0,
                cpus: vec![0, 1]
            }]
        );
    }

    #[test]
    fn cpu_without_package_is_skipped() {
        let root = fake_machine("offline", &[(0, Some(0), Some(0)), (1, None, None)]);

        let sockets = read_topology(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(sockets.len(), 1);
        assert_eq!(sockets[0].cpus, [0]);
    }

    #[test]
    fn ignores_other_directories() {
        let root = fake_machine("neighbours", &[(0, Some(0), Some(0))]);

        assert_eq!(read_topology(&root).unwrap().len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cpu_list_ranges() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11\n"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(parse_cpu_list(""), Some(vec![]));
        assert_eq!(parse_cpu_list("0-x"), None);
    }

    #[test]
    fn format_cpu_list() {
        assert_eq!(cpu_list(&[0, 1, 2, 3, 8, 10, 11]), "0-3,8,10-11");
        assert_eq!(cpu_list(&[5]), "5");
        assert_eq!(cpu_list(&[]), "");
    }

    #[test]
    fn model_of_first_cpu() {
        let cpuinfo = "processor\t: 0\nvendor_id\t: GenuineIntel\n\
                       model name\t: Intel(R) Xeon(R) Gold 5220\n\n\
                       processor\t: 1\nmodel name\t: something else\n";

        assert_eq!(
            read_model(cpuinfo.as_bytes()).unwrap(),
            "Intel(R) Xeon(R) Gold 5220"
        );
    }

    #[test]
    fn missing_model() {
        let error = read_model("processor\t: 0\nCPU part\t: 0xd0c\n".as_bytes()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
