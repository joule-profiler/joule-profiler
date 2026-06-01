//! cgroup v2 management utilities.
//!
//! This module provides:
//! - controller activation (`cpu`, `memory`, `io`);
//! - child cgroup creation and cleanup;
//! - process attachment to cgroups;
//! - CPU, memory, and I/O statistics collection.

use std::{
    collections::HashSet,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
};

use log::{debug, warn};

use crate::{
    Result,
    error::CgroupError,
    snapshot::{CpuSnapshot, IoSnapshot, MemorySnapshot},
    util::{read_flat_keyed_file, read_io_stat, read_u64_opt},
};

/// Available cgroup v2 controllers.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Controller {
    /// I/O controller.
    Io,

    /// Memory controller.
    Memory,

    /// CPU controller.
    Cpu,
}

impl Display for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Controller::Io => "io",
            Controller::Memory => "memory",
            Controller::Cpu => "cpu",
        })
    }
}

/// Interface for reading cgroup statistics.
pub trait CgroupBackend: Send + Sync + 'static {
    /// Initializes the backend.
    fn initialize(
        &self,
        path: &Path,
        root: &Path,
        pid: i32,
        controllers: &HashSet<Controller>,
    ) -> Result<()>;

    /// Cleanup the backend.
    fn cleanup(&self, path: &Path, root: &Path) -> Result<()>;

    /// Returns memory statistics.
    fn memory(&self, path: &Path) -> Result<MemorySnapshot>;

    /// Returns CPU statistics.
    fn cpu(&self, path: &Path) -> Result<CpuSnapshot>;

    /// Returns I/O statistics.
    fn io(&self, path: &Path) -> Result<IoSnapshot>;
}

/// Cgroup sysfs backend.
pub struct SysFsBackend;

impl SysFsBackend {
    /// Enables a controller in `cgroup.subtree_control`.
    ///
    /// If the provided controller is already activated, then nothing happens.
    fn activate_controller(root_path: &Path, controller: Controller) -> Result<()> {
        debug!(
            "Activating controller for root cgroup `{}`.",
            root_path.display()
        );
        let subtree_control_path = root_path.join("cgroup.subtree_control");
        fs::write(&subtree_control_path, format!("+{controller}")).map_err(|err| {
            CgroupError::FailedToCreateController {
                controller,
                path: subtree_control_path,
                source: err,
            }
        })?;
        Ok(())
    }

    /// Attaches a process PID to the cgroup.
    fn attach_pid(path: &Path, pid: i32) -> Result<()> {
        let procs_path = path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string()).map_err(|err| CgroupError::FailedToAttachPid {
            pid,
            path: procs_path,
            source: err,
        })?;
        debug!("Attached PID {pid} to cgroup {}", path.display());
        Ok(())
    }

    /// Returns all PIDs attached to the cgroup.
    fn pids(path: &Path) -> Result<Vec<i32>> {
        debug!("Retrieving cgroup `{}` PIDs.", path.display());
        let path = path.join("cgroup.procs");
        let content = fs::read_to_string(&path)?;
        Ok(content
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .collect())
    }
}

impl CgroupBackend for SysFsBackend {
    /// Creates the cgroup directory on disk.
    fn initialize(
        &self,
        path: &Path,
        root: &Path,
        pid: i32,
        controllers: &HashSet<Controller>,
    ) -> Result<()> {
        debug!("Initializing cgroup at \"{}\"", path.display());

        fs::create_dir_all(path)?;

        for controller in controllers {
            Self::activate_controller(root, *controller)?;
        }

        Self::attach_pid(path, pid)?;

        Ok(())
    }

    /// Moves processes back to the root cgroup and removes the directory.
    fn cleanup(&self, path: &Path, root: &Path) -> Result<()> {
        debug!("Cleaning cgroup `{}`.", path.display());

        let root_procs = root.join("cgroup.procs");
        for pid in Self::pids(path)? {
            if let Err(e) = fs::write(&root_procs, pid.to_string()) {
                warn!("Could not move PID {pid} back to root cgroup: {e}");
            }
        }
        if path.exists() {
            if let Err(e) = fs::remove_dir(path) {
                warn!(
                    "Could not remove cgroup {} (may still have live tasks): {e}",
                    path.display()
                );
            } else {
                debug!("Removed cgroup {}", path.display());
            }
        }
        Ok(())
    }

    /// Reads memory statistics from cgroup memory files.
    fn memory(&self, path: &Path) -> Result<MemorySnapshot> {
        let mut stat = read_flat_keyed_file(&path.join("memory.stat"))?;
        Ok(MemorySnapshot {
            current: read_u64_opt(&path.join("memory.current"))?,
            swap_current: read_u64_opt(&path.join("memory.swap.current"))?,
            peak: read_u64_opt(&path.join("memory.peak"))?,
            anon: stat.remove("anon"),
            file: stat.remove("file"),
            shmem: stat.remove("shmem"),
            kernel: stat.remove("kernel"),
            kernel_stack: stat.remove("kernel_stack"),
            slab: stat.remove("slab"),
        })
    }

    /// Reads CPU statistics from `cpu.stat`.
    fn cpu(&self, path: &Path) -> Result<CpuSnapshot> {
        let mut stat = read_flat_keyed_file(&path.join("cpu.stat"))?;

        Ok(CpuSnapshot {
            usage_usec: stat
                .remove("usage_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("usage_usec"))?,
            user_usec: stat
                .remove("user_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("user_usec"))?,
            system_usec: stat
                .remove("system_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("system_usec"))?,
            nr_periods: stat.remove("nr_periods"),
            nr_throttled: stat.remove("nr_throttled"),
            throttled_usec: stat.remove("throttled_usec"),
            nr_bursts: stat.remove("nr_bursts"),
            burst_usec: stat.remove("burst_usec"),
        })
    }

    /// Reads I/O statistics from `io.stat`.
    fn io(&self, path: &Path) -> Result<IoSnapshot> {
        let (rbytes, wbytes) = read_io_stat(&path.join("io.stat"))?;
        Ok(IoSnapshot { rbytes, wbytes })
    }
}

/// Structure representing the root cgroup.
pub struct RootCgroup<B: CgroupBackend = SysFsBackend> {
    /// The path to the cgroup.
    path: PathBuf,

    /// The backend to use to query cgroup interface (used mainly for testing).
    backend: B,
}

impl RootCgroup {
    /// Builds a cgroup handle based on the provided directory.
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            backend: SysFsBackend,
        }
    }

    /// Gets a child handle of the current cgroup based on its name.
    pub fn child(&self, name: &str) -> ChildCgroup {
        let child_path = self.path.join(name);
        ChildCgroup::new(child_path.clone(), self.path.clone(), SysFsBackend)
    }
}

impl<B: CgroupBackend> RootCgroup<B> {
    pub fn new(path: PathBuf, backend: B) -> Self {
        Self { path, backend }
    }

    /// Get memory stats.
    pub fn memory(&self) -> Result<MemorySnapshot> {
        self.backend.memory(&self.path)
    }

    /// Get CPU stats.
    pub fn cpu(&self) -> Result<CpuSnapshot> {
        self.backend.cpu(&self.path)
    }

    /// Get I/O stats.
    pub fn io(&self) -> Result<IoSnapshot> {
        self.backend.io(&self.path)
    }
}

impl Default for RootCgroup {
    fn default() -> Self {
        Self::at(PathBuf::from("/sys/fs/cgroup"))
    }
}

/// Structure representing a child cgroup.
pub struct ChildCgroup<B: CgroupBackend = SysFsBackend> {
    /// The path to the cgroup.
    path: PathBuf,

    /// The path to the root cgroup.
    root: PathBuf,

    /// The backend to use to query cgroup interface (used mainly for testing).
    backend: B,
}

impl<B: CgroupBackend> ChildCgroup<B> {
    pub fn new(path: PathBuf, root: PathBuf, backend: B) -> Self {
        Self {
            path,
            root,
            backend,
        }
    }

    /// Get memory stats.
    pub fn memory(&self) -> Result<MemorySnapshot> {
        self.backend.memory(&self.path)
    }

    /// Get CPU stats.
    pub fn cpu(&self) -> Result<CpuSnapshot> {
        self.backend.cpu(&self.path)
    }

    /// Get I/O stats.
    pub fn io(&self) -> Result<IoSnapshot> {
        self.backend.io(&self.path)
    }

    /// Initializes the cgroup backend.
    pub fn initialize(&self, pid: i32, controllers: &HashSet<Controller>) -> Result<()> {
        self.backend
            .initialize(&self.path, &self.root, pid, controllers)
    }

    /// Cleanup the cgroup backend.
    pub fn cleanup(&self) -> Result<()> {
        self.backend.cleanup(&self.path, &self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockCgroupBackend {
        memory: MemorySnapshot,
        cpu: CpuSnapshot,
        io: IoSnapshot,
    }

    impl CgroupBackend for MockCgroupBackend {
        fn memory(&self, _path: &Path) -> Result<MemorySnapshot> {
            Ok(self.memory.clone())
        }

        fn cpu(&self, _path: &Path) -> Result<CpuSnapshot> {
            Ok(self.cpu.clone())
        }

        fn io(&self, _path: &Path) -> Result<IoSnapshot> {
            Ok(self.io.clone())
        }

        fn initialize(
            &self,
            _path: &Path,
            _root: &Path,
            _pid: i32,
            _controllers: &HashSet<Controller>,
        ) -> Result<()> {
            Ok(())
        }

        fn cleanup(&self, _path: &Path, _root: &Path) -> Result<()> {
            Ok(())
        }
    }

    fn mock_cgroup(name: &str, backend: MockCgroupBackend) -> ChildCgroup<MockCgroupBackend> {
        let path = PathBuf::from(name);
        ChildCgroup {
            path: path.clone(),
            root: path,
            backend,
        }
    }

    #[test]
    fn test_stats_reader_memory() {
        let backend = MockCgroupBackend {
            memory: MemorySnapshot {
                current: Some(123),
                swap_current: Some(456),
                peak: Some(789),
                anon: Some(100),
                file: Some(200),
                shmem: None,
                kernel: None,
                kernel_stack: None,
                slab: Some(300),
            },
            ..Default::default()
        };

        let cg = mock_cgroup("memory", backend);

        let stats = cg.memory().unwrap();

        assert_eq!(stats.current, Some(123));
        assert_eq!(stats.swap_current, Some(456));
        assert_eq!(stats.peak, Some(789));
        assert_eq!(stats.anon, Some(100));
        assert_eq!(stats.file, Some(200));
        assert_eq!(stats.slab, Some(300));
    }

    #[test]
    fn test_stats_reader_cpu() {
        let backend = MockCgroupBackend {
            cpu: CpuSnapshot {
                usage_usec: 1000,
                user_usec: 400,
                system_usec: 600,
                nr_periods: Some(10),
                nr_throttled: Some(2),
                throttled_usec: None,
                nr_bursts: None,
                burst_usec: None,
            },
            ..Default::default()
        };

        let cg = mock_cgroup("cpu", backend);

        let stats = cg.cpu().unwrap();

        assert_eq!(stats.usage_usec, 1000);
        assert_eq!(stats.user_usec, 400);
        assert_eq!(stats.system_usec, 600);
        assert_eq!(stats.nr_periods, Some(10));
        assert_eq!(stats.nr_throttled, Some(2));
    }

    #[test]
    fn test_stats_reader_io() {
        let backend = MockCgroupBackend {
            io: IoSnapshot {
                rbytes: Some(120),
                wbytes: Some(80),
            },
            ..Default::default()
        };

        let cg = mock_cgroup("io", backend);

        let stats = cg.io().unwrap();

        assert_eq!(stats.rbytes, Some(120));
        assert_eq!(stats.wbytes, Some(80));
    }
}
