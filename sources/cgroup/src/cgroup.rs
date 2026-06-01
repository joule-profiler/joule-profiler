//! cgroup v2 management utilities.
//!
//! This module provides:
//! - controller activation (`cpu`, `memory`, `io`);
//! - child cgroup creation and cleanup;
//! - process attachment to cgroups;
//! - CPU, memory, and I/O statistics collection.

use std::{collections::HashSet, fmt::Display, fs, path::PathBuf};

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
    fn initialize(&self, pid: i32, controllers: &HashSet<Controller>) -> Result<()>;

    /// Returns memory statistics.
    fn memory(&self) -> Result<MemorySnapshot>;

    /// Returns CPU statistics.
    fn cpu(&self) -> Result<CpuSnapshot>;

    /// Returns I/O statistics.
    fn io(&self) -> Result<IoSnapshot>;

    /// Cleanup the backend.
    fn cleanup(&self) -> Result<()>;
}

/// A cgroup node. If `parent` is `None`, this is the root cgroup.
pub struct Cgroup<B: CgroupBackend = SysFsBackend> {
    pub path: PathBuf,
    parent: Option<PathBuf>,
    backend: B,
}

impl Cgroup {
    /// Gets the root cgroup in sys fs `/sys/fs/cgroup`.
    pub fn root() -> Self {
        let path = PathBuf::from("/sys/fs/cgroup");
        Self::at(path)
    }

    /// Builds a cgroup handle based on the provided directory.
    pub fn at(path: PathBuf) -> Self {
        let backend = SysFsBackend {
            path: path.clone(),
            root: path.clone(),
        };
        Self {
            path,
            parent: None,
            backend,
        }
    }

    /// Gets a child handle of the current cgroup based on its name.
    pub fn child(&self, name: &str) -> Cgroup {
        let child_path = self.path.join(name);
        Cgroup {
            parent: Some(self.path.clone()),
            backend: SysFsBackend {
                root: self.path.clone(),
                path: child_path.clone(),
            },
            path: child_path,
        }
    }
}

impl<B: CgroupBackend> Cgroup<B> {
    pub fn new(path: PathBuf, parent: Option<PathBuf>, backend: B) -> Self {
        Self {
            path,
            parent,
            backend,
        }
    }

    /// True if the cgroup is the root cgroup, else false.
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Return the backend for stats querying.
    pub fn stats(&self) -> &B {
        &self.backend
    }

    /// Initializes the cgroup backend.
    pub fn initialize(&self, pid: i32, controllers: &HashSet<Controller>) -> Result<()> {
        self.backend.initialize(pid, controllers)
    }

    /// Cleanup the cgroup backend.
    pub fn cleanup(&self) -> Result<()> {
        self.backend.cleanup()
    }
}

/// Cleans up the cgroup if not done already (cleanup not called or error).
impl<B: CgroupBackend> Drop for Cgroup<B> {
    fn drop(&mut self) {
        if self.parent.is_some() && self.path.exists() {
            let _ = self.backend.cleanup();
        }
    }
}

/// cgroup statistics accessor.
pub struct SysFsBackend {
    path: PathBuf,
    root: PathBuf,
}

impl SysFsBackend {
    /// Enables a controller in `cgroup.subtree_control`.
    ///
    /// If the provided controller is already activated, then nothing happens.
    pub fn activate_controller(&self, controller: Controller) -> Result<()> {
        debug!(
            "Activating controller for root cgroup `{}`.",
            self.path.display()
        );
        let subtree_control_path = self.root.join("cgroup.subtree_control");
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
    pub fn attach_pid(&self, pid: i32) -> Result<()> {
        let procs_path = self.path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string()).map_err(|err| CgroupError::FailedToAttachPid {
            pid,
            path: procs_path,
            source: err,
        })?;
        debug!("Attached PID {pid} to cgroup {}", self.path.display());
        Ok(())
    }

    /// Returns all PIDs attached to the cgroup.
    fn pids(&self) -> Result<Vec<i32>> {
        debug!("Retrieving cgroup `{}` PIDs.", self.path.display());
        let path = self.path.join("cgroup.procs");
        let content = fs::read_to_string(&path)?;
        Ok(content
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .collect())
    }
}

impl CgroupBackend for SysFsBackend {
    /// Creates the cgroup directory on disk.
    fn initialize(&self, pid: i32, controllers: &HashSet<Controller>) -> Result<()> {
        debug!("Initializing cgroup at \"{}\"", self.path.display());

        fs::create_dir_all(&self.path)?;

        for controller in controllers {
            self.activate_controller(*controller)?;
        }

        self.attach_pid(pid)?;

        Ok(())
    }

    /// Moves processes back to the root cgroup and removes the directory.
    fn cleanup(&self) -> Result<()> {
        debug!("Cleaning cgroup `{}`.", self.path.display());

        let root_procs = self.root.join("cgroup.procs");
        for pid in self.pids()? {
            if let Err(e) = fs::write(&root_procs, pid.to_string()) {
                warn!("Could not move PID {pid} back to root cgroup: {e}");
            }
        }
        if self.path.exists() {
            if let Err(e) = fs::remove_dir(&self.path) {
                warn!(
                    "Could not remove cgroup {} (may still have live tasks): {e}",
                    self.path.display()
                );
            } else {
                debug!("Removed cgroup {}", self.path.display());
            }
        }
        Ok(())
    }

    /// Reads memory statistics from cgroup memory files.
    fn memory(&self) -> Result<MemorySnapshot> {
        debug!(
            "Reading memory metrics for cgroup `{}`.",
            self.path.display()
        );

        let mut memory_stat = read_flat_keyed_file(&self.path.join("memory.stat"))?;
        let current = read_u64_opt(&self.path.join("memory.current"))?;
        let swap_current = read_u64_opt(&self.path.join("memory.swap.current"))?;
        let peak = read_u64_opt(&self.path.join("memory.peak"))?;

        Ok(MemorySnapshot {
            current,
            swap_current,
            peak,
            anon: memory_stat.remove("anon"),
            file: memory_stat.remove("file"),
            shmem: memory_stat.remove("shmem"),
            kernel: memory_stat.remove("kernel"),
            kernel_stack: memory_stat.remove("kernel_stack"),
            slab: memory_stat.remove("slab"),
        })
    }

    /// Reads I/O statistics from `io.stat`.
    fn io(&self) -> Result<IoSnapshot> {
        debug!("Reading I/O metrics for cgroup `{}`.", self.path.display());
        let (rbytes, wbytes) = read_io_stat(&self.path.join("io.stat"))?;
        Ok(IoSnapshot { rbytes, wbytes })
    }

    /// Reads CPU statistics from `cpu.stat`.
    fn cpu(&self) -> Result<CpuSnapshot> {
        debug!("Reading cpu metrics for cgroup `{}`.", self.path.display());

        let mut cpu_stat = read_flat_keyed_file(&self.path.join("cpu.stat"))?;

        let usage_usec = cpu_stat
            .remove("usage_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("usage_usec"))?;
        let user_usec = cpu_stat
            .remove("user_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("user_usec"))?;
        let system_usec = cpu_stat
            .remove("system_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("system_usec"))?;

        Ok(CpuSnapshot {
            usage_usec,
            user_usec,
            system_usec,
            nr_periods: cpu_stat.remove("nr_periods"),
            nr_throttled: cpu_stat.remove("nr_throttled"),
            throttled_usec: cpu_stat.remove("throttled_usec"),
            nr_bursts: cpu_stat.remove("nr_bursts"),
            burst_usec: cpu_stat.remove("burst_usec"),
        })
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
        fn memory(&self) -> Result<MemorySnapshot> {
            Ok(self.memory.clone())
        }

        fn cpu(&self) -> Result<CpuSnapshot> {
            Ok(self.cpu.clone())
        }

        fn io(&self) -> Result<IoSnapshot> {
            Ok(self.io.clone())
        }

        fn initialize(&self, _pid: i32, _controllers: &HashSet<Controller>) -> Result<()> {
            Ok(())
        }

        fn cleanup(&self) -> Result<()> {
            Ok(())
        }
    }

    fn mock_cgroup(name: &str, backend: MockCgroupBackend) -> Cgroup<MockCgroupBackend> {
        let path = std::env::temp_dir().join(format!("cgroup_test_{name}"));
        Cgroup {
            path: path.clone(),
            parent: Some(path),
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

        let cg = mock_cgroup("mock_mem", backend);

        let stats = cg.stats().memory().unwrap();

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

        let cg = mock_cgroup("mock_cpu", backend);

        let stats = cg.stats().cpu().unwrap();

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

        let cg = mock_cgroup("mock_io", backend);

        let stats = cg.stats().io().unwrap();

        assert_eq!(stats.rbytes, Some(120));
        assert_eq!(stats.wbytes, Some(80));
    }
}
