use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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

use joule_profiler_core::util::cgroup::{Cgroup as CgroupDir, ROOT};

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
        let root = self.root.unwrap_or_else(|| ROOT.into());

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

        let layout = Arc::new(layout);
        Ok(Source::new(
            CgroupSensor::new(
                Arc::clone(&proc),
                global,
                Arc::clone(&layout),
                self.poll_interval,
            ),
            CgroupProcessor::new(layout, proc, self.poll_interval),
        ))
    }
}
