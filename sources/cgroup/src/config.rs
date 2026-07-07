use std::{path::PathBuf, time::Duration};

use serde::Deserialize;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn default_poll_interval() -> Duration {
    DEFAULT_POLL_INTERVAL
}

fn default_true() -> bool {
    true
}

fn default_cgroup_name() -> String {
    format!("joule-profiler-{}", std::process::id())
}

/// Configuration for the cgroup metric source.
#[derive(Debug, Clone, Deserialize)]
pub struct CgroupConfig {
    /// Path to cgroup v2 hierarchy (usually `/sys/fs/cgroup`).
    pub cgroup_root: Option<PathBuf>,

    /// Name of the created cgroup for the monitored process.
    #[serde(default = "default_cgroup_name")]
    pub cgroup_name: String,

    /// Background polling interval.
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    pub poll_interval: Duration,

    /// Whether the source must attach the process pid to the cgroup or not.
    #[serde(default = "default_true")]
    pub attach_pid: bool,

    /// Whether the cgroup is already created or if the source must create it itself.
    #[serde(default = "default_true")]
    pub create_cgroup: bool,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            cgroup_root: None,
            cgroup_name: default_cgroup_name(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            attach_pid: true,
            create_cgroup: true,
        }
    }
}
