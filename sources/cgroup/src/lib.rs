//! `cgroup` source for Joule Profiler.
//!
//! Reads the memory, CPU time and I/O of a cgroup v2, from the files of `/sys/fs/cgroup`. Every
//! process in the cgroup is measured, so the profiled program and everything it starts are
//! counted, even a process that leaves its process tree.
//!
//! Unlike the `procfs` source, the kernel does the accounting: short-lived and ended processes
//! are counted, and the cost does not grow with the number of processes.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_source_cgroup::Cgroup;
//!
//! let source = Cgroup::new("my.slice").global(true).build().unwrap();
//! ```

mod cgroup;
mod error;
mod layout;
mod processor;
mod sensor;

use std::{path::PathBuf, sync::Arc, time::Duration};

use joule_profiler_core::{
    Info, InfoValue, MemoryUnit, MetricUnit, Source,
    util::cgroup::{Cgroup as CgroupDir, ROOT_PATH},
};
use serde::Deserialize;

pub use error::CgroupError;

use crate::{
    cgroup::controllers,
    error::Result,
    layout::{AvailableValues, Layout},
    processor::CgroupProcessor,
    sensor::CgroupSensor,
};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Configuration of the cgroup source.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cgroup {
    name: PathBuf,

    #[serde(default)]
    root: Option<PathBuf>,

    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    process_poll_interval: Duration,

    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    global_poll_interval: Duration,

    #[serde(default)]
    global: bool,

    #[serde(default)]
    process_memory_unit: MemoryUnit,

    #[serde(default)]
    global_memory_unit: MemoryUnit,
}

fn default_poll_interval() -> Duration {
    DEFAULT_POLL_INTERVAL
}

impl Cgroup {
    /// Initializes a configuration for the cgroup with its `name`, relative to the root.
    pub fn new(name: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root: None,
            process_poll_interval: DEFAULT_POLL_INTERVAL,
            global_poll_interval: DEFAULT_POLL_INTERVAL,
            global: false,
            process_memory_unit: MemoryUnit::default(),
            global_memory_unit: MemoryUnit::default(),
        }
    }

    /// Sets the cgroup root. Defaults to `/sys/fs/cgroup`.
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Sets how often the memory of the cgroup is read. Defaults to 20ms.
    pub fn process_poll_interval(mut self, interval: Duration) -> Self {
        self.process_poll_interval = interval;
        self
    }

    /// Sets how often the memory of the root cgroup is read, when `global` is set. Defaults to
    /// 20ms.
    pub fn global_poll_interval(mut self, interval: Duration) -> Self {
        self.global_poll_interval = interval;
        self
    }

    /// Sets whether the root cgroup, that is the whole machine, is reported too. Defaults to
    /// `false`.
    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    /// Sets the unit of the memory of the cgroup. Defaults to bytes.
    pub fn process_memory_unit(mut self, unit: MemoryUnit) -> Self {
        self.process_memory_unit = unit;
        self
    }

    /// Sets the unit of the memory of the root cgroup, when `global` is set. Defaults to bytes.
    pub fn global_memory_unit(mut self, unit: MemoryUnit) -> Self {
        self.global_memory_unit = unit;
        self
    }

    /// Builds the source. The metrics depend on the files of the cgroup: if it does not exist
    /// yet, as when listing the metrics before a run, they are predicted from the controllers
    /// its parent gives it.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::NotACgroup`] if the root or the cgroup is a directory that is not a
    /// cgroup v2, and [`CgroupError::Read`] if one of its files cannot be read.
    pub fn build(self) -> Result<Source> {
        let root = self.root.clone().unwrap_or_else(|| ROOT_PATH.into());

        let proc = Arc::new(CgroupDir::at(root.join(&self.name)));
        let global = Arc::new(CgroupDir::at(root));

        for cgroup in [&global, &proc] {
            if cgroup.exists() && !cgroup.is_cgroup() {
                return Err(CgroupError::NotACgroup(cgroup.path().to_path_buf()));
            }
        }

        let proc_values = if proc.exists() {
            AvailableValues::probe(&proc)?
        } else {
            AvailableValues::expected(&controllers(&proc)?)
        };

        let layout = Layout {
            proc: proc_values,
            global: self
                .global
                .then(|| AvailableValues::probe(&global))
                .transpose()?,
            process_memory_unit: self.process_memory_unit,
            global_memory_unit: self.global_memory_unit,
        };

        log::debug!("cgroup: {} metrics reported", layout.metrics().len());

        let info = info(&proc, &self, layout.global.is_some());
        let layout = Arc::new(layout);
        Ok(Source::new(
            CgroupSensor::new(
                proc,
                global,
                Arc::clone(&layout),
                self.process_poll_interval,
                self.global_poll_interval,
            ),
            CgroupProcessor::new(layout),
        )
        .with_info(info))
    }
}

/// Returns the info of the source: the cgroup, its controllers and the configuration.
fn info(cgroup: &CgroupDir, config: &Cgroup, global: bool) -> Info {
    Info::new()
        .with("cgroup", cgroup.path().display().to_string())
        .with("controllers", controllers(cgroup).map(InfoValue::list))
        .with("global", global)
        .with("process_poll_interval", config.process_poll_interval)
        .with("global_poll_interval", config.global_poll_interval)
        .with(
            "process_memory_unit",
            MetricUnit::from(config.process_memory_unit).to_string(),
        )
        .with(
            "global_memory_unit",
            MetricUnit::from(config.global_memory_unit).to_string(),
        )
}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use super::*;

    #[test]
    fn not_a_cgroup() {
        let root = std::env::temp_dir().join(format!("joule-cgroup-not-one-{}", process::id()));
        fs::create_dir_all(&root).unwrap();

        let built = Cgroup::new("run").root(&root).build();
        fs::remove_dir_all(&root).unwrap();

        assert!(matches!(built, Err(CgroupError::NotACgroup(path)) if path == root));
    }
}
