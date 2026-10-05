use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::event::Event;

pub type Result<T> = std::result::Result<T, PerfEventError>;

#[derive(Debug, Error)]
pub enum PerfEventError {
    #[error(
        "permission denied on the perf counters: run as root, or `sudo sysctl kernel.perf_event_paranoid=0`"
    )]
    PermissionDenied(#[source] io::Error),

    #[error("opening the cgroup {0}")]
    OpenCgroup(PathBuf, #[source] io::Error),

    #[error("opening the perf group of cpu {0}")]
    OpenGroup(u32, #[source] io::Error),

    #[error("opening the perf group")]
    OpenLeader(#[source] io::Error),

    #[error("opening the {0} counter")]
    OpenCounter(Event, #[source] io::Error),

    #[error("reading the {0} counter")]
    ReadCounter(Event, #[source] io::Error),

    #[error("reading the perf counters of cpu {0}")]
    ReadGroup(u32, #[source] io::Error),

    #[error("reading the perf counters")]
    ReadCounters(#[source] io::Error),

    #[error("switching the perf counters")]
    Switch(#[source] io::Error),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("cpu {0} is not online")]
    OfflineCpu(u32),

    #[error("reading counters that were never opened")]
    NotOpened,
}
