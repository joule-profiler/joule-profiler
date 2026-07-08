use std::{collections::HashSet, path::PathBuf};

use serde::Deserialize;

use crate::event::Event;

#[derive(Debug, Default, Deserialize)]
pub struct PerfConfig {
    /// The events to monitor.
    pub events: Option<HashSet<Event>>,

    /// Name of a cgroup v2 directory under `/sys/fs/cgroup` to monitor
    /// instead of the profiled process's pid (e.g. `my-cgroup` or
    /// `parent/child` for a nested cgroup). When set, counters are scoped to
    /// every process inside that cgroup rather than a single pid and its
    /// children.
    /// It can be better to open counters with a cgroup rather than a pid,
    /// because it allows to open the counters before spawning the process.
    ///
    /// The cgroup must be created before opening the counters.
    pub cgroup_name: Option<PathBuf>,

    /// The root path of the cgroup if cgroup name is provided.
    pub cgroup_root: Option<PathBuf>,

    /// If the source uses the cgroup, then you can specify on
    /// which CPU cores you want to open the counters.
    /// It can be useful if your program has a configured CPU affinity (e.g., taskset)
    /// Not opening counters on all cores can improve the performance of the source.
    pub cpu_spec: Option<HashSet<u32>>,
}
