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
pub trait StatsReader {
    /// Returns memory statistics.
    fn memory(&self) -> Result<MemorySnapshot>;

    /// Returns CPU statistics.
    fn cpu(&self) -> Result<CpuSnapshot>;

    /// Returns I/O statistics.
    fn io(&self) -> Result<IoSnapshot>;
}

/// Root cgroup manager.
///
/// Used to enable controllers and create child cgroups.
pub struct RootCgroup {
    /// The path of the root cgroup, default is `/sys/fs/cgroup` (override for testing).
    path: PathBuf,

    /// List of controllers activated for the root cgroup.
    controllers: HashSet<Controller>,
}

impl Default for RootCgroup {
    fn default() -> Self {
        Self {
            path: PathBuf::from("/sys/fs/cgroup"),
            controllers: HashSet::new(),
        }
    }
}

impl RootCgroup {
    /// Creates a new root cgroup manager from a custom path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            ..Default::default()
        }
    }

    /// Enables a controller in `cgroup.subtree_control`.
    ///
    /// If the provided controller is already activated, then nothing happens.
    pub fn activate_controller(&self, controller: Controller) -> Result<()> {
        if !self.controllers.contains(&controller) {
            let subtree_control_path = self.path.join("cgroup.subtree_control");
            fs::write(&subtree_control_path, format!("+{controller}")).map_err(|err| {
                CgroupError::FailedToCreateController {
                    controller,
                    path: subtree_control_path,
                    source: err,
                }
            })?;
        }
        Ok(())
    }

    /// Creates a child cgroup handle.
    pub fn child(&self, name: &str) -> Cgroup {
        Cgroup {
            path: self.path.join(name),
            root: self.path.clone(),
        }
    }

    /// Returns a statistics reader for the root cgroup.
    pub fn stats(&self) -> CgroupStat<'_> {
        CgroupStat { path: &self.path }
    }
}

/// Represents a child cgroup.
pub struct Cgroup {
    /// The path of the cgroup, normally it is `/sys/fs/cgroup/{cgroup_name}` but it can be override for testing.
    path: PathBuf,

    /// The path of the root cgroup, default is `/sys/fs/cgroup` (override for testing).
    root: PathBuf,
}

impl Cgroup {
    /// Creates the cgroup directory on disk.
    pub fn initialize(&self) -> Result<()> {
        debug!("Creating cgroup at \"{}\"", self.path.display());
        fs::create_dir_all(&self.path)?;
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

    /// Returns a statistics reader for this cgroup.
    pub fn stats(&self) -> CgroupStat<'_> {
        CgroupStat { path: &self.path }
    }

    /// Returns all PIDs attached to the cgroup.
    fn pids(&self) -> Result<Vec<i32>> {
        let path = self.path.join("cgroup.procs");
        let content = fs::read_to_string(&path)?;
        Ok(content
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .collect())
    }

    /// Moves processes back to the root cgroup and removes the directory.
    pub fn cleanup(&self) -> Result<()> {
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
}

/// Cleans up the cgroup if not done already (cleanup not called or error).
impl Drop for Cgroup {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = self.cleanup();
        }
    }
}

/// cgroup statistics accessor.
pub struct CgroupStat<'a> {
    path: &'a Path,
}

impl StatsReader for CgroupStat<'_> {
    /// Reads memory statistics from cgroup memory files.
    fn memory(&self) -> Result<MemorySnapshot> {
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
        let (rbytes, wbytes) = read_io_stat(&self.path.join("io.stat"))?;
        Ok(IoSnapshot { rbytes, wbytes })
    }

    /// Reads CPU statistics from `cpu.stat`.
    fn cpu(&self) -> Result<CpuSnapshot> {
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
