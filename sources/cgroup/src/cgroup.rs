use std::{fmt::Display, fs, path::PathBuf};

use log::{debug, warn};

use crate::{
    Result,
    error::CgroupError::{self},
    snapshot::{CpuSnapshot, IoSnapshot, MemorySnapshot},
    util::{read_flat_keyed_file, read_io_stat, read_u64, read_u64_opt},
};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Controller {
    Io,
    Mem,
    Cpu,
}

impl Display for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Controller::Io => "io",
            Controller::Mem => "memory",
            Controller::Cpu => "cpu",
        })
    }
}

pub struct Cgroup {
    cgroup_root: PathBuf,
    cgroup_path: PathBuf,
}

impl Cgroup {
    pub fn new(cgroup_root: PathBuf, cgroup_name: &str) -> Result<Self> {
        let cgroup_path = cgroup_root.join(cgroup_name);
        if cgroup_path.exists() {
            return Err(CgroupError::AlreadyCreated(cgroup_name.to_owned()));
        }

        Ok(Self {
            cgroup_root,
            cgroup_path,
        })
    }

    pub fn initialize_cgroup(&self) -> Result<()> {
        debug!("Creating cgroup at \"{}\"", self.cgroup_path.display());
        fs::create_dir_all(&self.cgroup_path)?;
        Ok(())
    }

    pub fn activate_controller(&self, controller: Controller) -> Result<()> {
        let subtree_control_path = self.cgroup_root.join("cgroup.subtree_control");
        fs::write(&subtree_control_path, format!("+{controller}")).map_err(|err| {
            CgroupError::FailedToCreateController {
                controller,
                path: subtree_control_path,
                source: err,
            }
        })?;
        Ok(())
    }

    pub fn attach_pid(&self, pid: i32) -> Result<()> {
        let procs_path = self.cgroup_path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string()).map_err(|err| CgroupError::FailedToAttachPid {
            pid,
            path: procs_path,
            source: err,
        })?;
        debug!(
            "Attached PID {pid} to cgroup {}",
            self.cgroup_path.display()
        );
        Ok(())
    }

    pub fn read_memory(&self) -> Result<MemorySnapshot> {
        let mut memory_stat = read_flat_keyed_file(&self.cgroup_path.join("memory.stat"))?;
        let current = read_u64(&self.cgroup_path.join("memory.current"))?;
        let swap_current = read_u64_opt(&self.cgroup_path.join("memory.swap.current"))?;
        let peak = read_u64_opt(&self.cgroup_path.join("memory.peak"))?;

        let anon = memory_stat.remove("anon");
        let file = memory_stat.remove("file");
        let shmem = memory_stat.remove("shmem");
        let kernel = memory_stat.remove("kernel");
        let kernel_stack = memory_stat.remove("kernel_stack");
        let slab = memory_stat.remove("slab");

        Ok(MemorySnapshot {
            current,
            swap_current,
            anon,
            file,
            peak,
            shmem,
            kernel,
            kernel_stack,
            slab,
        })
    }

    pub fn read_io(&self) -> Result<IoSnapshot> {
        let (rbytes, wbytes) = read_io_stat(&self.cgroup_path.join("io.stat"))?;
        Ok(IoSnapshot { rbytes, wbytes })
    }

    pub fn read_cpu(&self) -> Result<CpuSnapshot> {
        let mut cpu_stat = read_flat_keyed_file(&self.cgroup_path.join("cpu.stat"))?;

        let usage_usec = cpu_stat
            .remove("usage_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("usage_usec"))?;
        let user_usec = cpu_stat
            .remove("user_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("user_usec"))?;
        let system_usec = cpu_stat
            .remove("system_usec")
            .ok_or(CgroupError::MissingAlwaysPresentMetric("system_usec"))?;

        let nr_periods = cpu_stat.remove("nr_periods");
        let nr_throttled = cpu_stat.remove("nr_throttled");
        let throttled_usec = cpu_stat.remove("throttled_usec");
        let nr_bursts = cpu_stat.remove("nr_bursts");
        let burst_usec = cpu_stat.remove("burst_usec");

        Ok(CpuSnapshot {
            usage_usec,
            user_usec,
            system_usec,
            nr_periods,
            nr_throttled,
            throttled_usec,
            nr_bursts,
            burst_usec,
        })
    }

    fn get_pids(&self) -> Result<Vec<i32>> {
        let path = self.cgroup_path.join("cgroup.procs");
        let content = fs::read_to_string(&path)?;
        let pids = content
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .collect();
        Ok(pids)
    }

    pub fn cleanup(&self) -> Result<()> {
        let root_procs = self.cgroup_path.join("cgroup.procs");
        for pid in self.get_pids()? {
            if let Err(e) = fs::write(&root_procs, pid.to_string()) {
                warn!("Could not move PID {pid} back to root cgroup: {e}");
            }
        }
        if self.cgroup_path.exists() {
            if let Err(e) = fs::remove_dir(&self.cgroup_path) {
                warn!(
                    "Could not remove cgroup {} (may still have live tasks): {e}",
                    self.cgroup_path.display()
                );
            } else {
                debug!("Removed cgroup {}", self.cgroup_path.display());
            }
        }
        Ok(())
    }
}
