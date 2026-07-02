use std::{collections::HashSet, path::PathBuf, time::Duration};

use crate::cgroup::Controller;

/// Configuration for the cgroup metric source.
#[derive(Debug, Clone)]
pub struct CgroupConfig {
    /// Path to cgroup v2 hierarchy (usually `/sys/fs/cgroup`).
    pub cgroup_root: Option<PathBuf>,

    /// Name of the created cgroup for the monitored process.
    pub cgroup_name: String,

    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Enabled cgroup controllers (cpu, memory, io).
    pub controllers: HashSet<Controller>,

    /// Whether the source must attach the process pid to the cgroup or not.
    pub attach_pid: bool,

    /// Whether the cgroup is already created or if the source must create it itself.
    pub create_cgroup: bool,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            cgroup_root: None,
            cgroup_name: format!("joule-profiler-{}", std::process::id()),
            poll_interval: None,
            controllers: vec![Controller::Io, Controller::Memory, Controller::Cpu]
                .into_iter()
                .collect(),
            attach_pid: true,
            create_cgroup: true,
        }
    }
}
