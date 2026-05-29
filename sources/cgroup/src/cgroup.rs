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
    pub path: PathBuf,

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
    pub path: PathBuf,

    /// The path of the root cgroup, default is `/sys/fs/cgroup` (override for testing).
    pub root: PathBuf,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, path::PathBuf};

    fn temp_cgroup_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("cgroup_test_{name}"));
        let _ = fs::create_dir_all(&path);
        path
    }

    fn write_file(path: &Path, content: &str) {
        let mut f = fs::File::create(path).unwrap();
        writeln!(f, "{content}").unwrap();
    }

    fn setup_root(name: &str) -> RootCgroup {
        let root = temp_cgroup_dir(name);
        RootCgroup::new(root)
    }

    #[test]
    fn test_initialize_and_attach_pid() {
        let root = setup_root("init");
        let cg = root.child("test_group");

        cg.initialize().unwrap();

        let pid_file = cg.path.join("cgroup.procs");
        write_file(&pid_file, "1234");

        cg.attach_pid(42).unwrap();

        let content = fs::read_to_string(&pid_file).unwrap();
        assert!(content.contains("42"));

        let _ = cg.cleanup();
    }

    #[test]
    fn test_pids_parsing() {
        let root = setup_root("pids");
        let cg = root.child("pids_group");

        cg.initialize().unwrap();

        write_file(&cg.path.join("cgroup.procs"), "1\n2\n3");

        let pids = cg.pids().unwrap();
        assert_eq!(pids, vec![1, 2, 3]);

        let _ = cg.cleanup();
    }

    #[test]
    fn test_cleanup_moves_pids() {
        let root = setup_root("cleanup");
        let cg = root.child("cleanup_group");

        cg.initialize().unwrap();

        let root_procs = root.path.join("cgroup.procs");
        write_file(&cg.path.join("cgroup.procs"), "99");

        cg.cleanup().unwrap();

        let moved = fs::read_to_string(root_procs).unwrap();
        assert!(moved.contains("99"));
    }

    #[test]
    fn test_memory_stats() {
        let root = setup_root("mem");
        let cg = root.child("mem_group");

        cg.initialize().unwrap();

        write_file(&cg.path.join("memory.stat"), "anon 100\nfile 200\nslab 300");
        write_file(&cg.path.join("memory.current"), "123");
        write_file(&cg.path.join("memory.swap.current"), "456");
        write_file(&cg.path.join("memory.peak"), "789");

        let stats = cg.stats().memory().unwrap();

        assert_eq!(stats.current, Some(123));
        assert_eq!(stats.swap_current, Some(456));
        assert_eq!(stats.peak, Some(789));
        assert_eq!(stats.anon, Some(100));
        assert_eq!(stats.file, Some(200));
        assert_eq!(stats.slab, Some(300));

        let _ = cg.cleanup();
    }

    #[test]
    fn test_cpu_stats() {
        let root = setup_root("cpu");
        let cg = root.child("cpu_group");

        cg.initialize().unwrap();

        write_file(
            &cg.path.join("cpu.stat"),
            "\
usage_usec 1000
user_usec 400
system_usec 600
nr_periods 10
nr_throttled 2
",
        );

        let stats = cg.stats().cpu().unwrap();

        assert_eq!(stats.usage_usec, 1000);
        assert_eq!(stats.user_usec, 400);
        assert_eq!(stats.system_usec, 600);
        assert_eq!(stats.nr_periods, Some(10));
        assert_eq!(stats.nr_throttled, Some(2));

        let _ = cg.cleanup();
    }

    #[test]
    fn test_io_stats() {
        let root = setup_root("io");
        let cg = root.child("io_group");

        cg.initialize().unwrap();

        write_file(
            &cg.path.join("io.stat"),
            "\
8:0 rbytes=100 wbytes=50
8:1 rbytes=20 wbytes=30
",
        );

        let stats = cg.stats().io().unwrap();

        assert_eq!(stats.rbytes, Some(120));
        assert_eq!(stats.wbytes, Some(80));

        let _ = cg.cleanup();
    }

    #[test]
    fn test_activate_controller_does_not_crash() {
        let root = setup_root("ctrl");

        let ctrl_file = root.path.join("cgroup.subtree_control");
        write_file(&ctrl_file, "");

        let res = root.activate_controller(Controller::Cpu);
        assert!(res.is_ok());
    }
}
