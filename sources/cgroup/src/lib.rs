use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::source::Source;
use serde::Deserialize;

pub mod cgroup;
pub mod error;
pub mod processor;
pub mod sensor;

mod layout;

pub use error::CgroupError;
pub use processor::CgroupProcessor;
pub use sensor::{CgroupSensor, Snapshot};

use joule_profiler_core::util::cgroup::{Cgroup as CgroupDir, ROOT_PATH};

use crate::cgroup::controllers;
use crate::error::Result;
use crate::layout::Layout;

const POLL: Duration = Duration::from_millis(10);

/// Memory, CPU and I/O of a cgroup. Both the builder and the `[sources.cgroup]` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cgroup {
    name: PathBuf,

    /// Default: `/sys/fs/cgroup`.
    #[serde(default)]
    root: Option<PathBuf>,

    #[serde(default = "poll", with = "humantime_serde")]
    poll_interval: Duration,

    /// Also reports the whole hierarchy.
    #[serde(default)]
    global: bool,
}

fn poll() -> Duration {
    POLL
}

impl Cgroup {
    pub fn new(name: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root: None,
            poll_interval: POLL,
            global: false,
        }
    }

    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    pub fn poll(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    /// The metrics depend on the files of the cgroup. If it does not exist yet, as when listing
    /// before a run, they are predicted from the controllers it will get.
    pub fn build(self) -> Result<Source> {
        let root = self.root.unwrap_or_else(|| ROOT_PATH.into());

        let proc = Arc::new(CgroupDir::at(root.join(self.name)));
        let global = Arc::new(CgroupDir::at(root));

        let layout = if proc.exists() {
            Layout::probe(&proc, self.global)?
        } else {
            Layout::expected(&controllers(&proc)?, self.global)
        };

        if layout.is_empty() {
            return Err(CgroupError::NothingToRead(proc.path().to_path_buf()));
        }

        log::debug!("cgroup: {} metrics reported", layout.metrics().len());

        let info = info(&proc, layout.global, self.poll_interval);
        let layout = Arc::new(layout);
        Ok(Source::new(
            CgroupSensor::new(proc, global, Arc::clone(&layout), self.poll_interval),
            CgroupProcessor::new(layout),
        )
        .with_info(info))
    }
}

pub(crate) fn info(
    cgroup: &joule_profiler_core::util::cgroup::Cgroup,
    global: bool,
    poll_interval: Duration,
) -> Info {
    Info::new()
        .with("cgroup", cgroup.path().display().to_string())
        .with("controllers", controllers(cgroup).map(InfoValue::list))
        .with("global", global)
        .with("poll_interval", poll_interval)
}

#[cfg(test)]
mod tests {
    use std::{fs, process, time::Duration};

    use joule_profiler_core::info::InfoValue;

    use crate::info;

    #[test]
    fn the_cgroup_and_its_controllers_are_in_the_info() {
        let path = std::env::temp_dir().join(format!("joule-cgroup-info-{}", process::id()));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("cgroup.controllers"), "cpu io memory\n").unwrap();

        let info = info(
            &joule_profiler_core::util::cgroup::Cgroup::at(&path),
            false,
            Duration::from_millis(10),
        );
        fs::remove_dir_all(&path).unwrap();

        assert_eq!(info.get("cgroup"), Some(&path.display().to_string().into()));
        assert_eq!(
            info.get("controllers"),
            Some(&InfoValue::list(["cpu", "io", "memory"]))
        );
        assert_eq!(info.get("poll_interval"), Some(&"10ms".into()));
    }
}
