use std::{io, path::PathBuf};

use thiserror::Error;

use crate::event::Event;

pub type Result<T> = std::result::Result<T, PerfEventError>;

/// An error returned by the `perf_event` source.
#[derive(Debug, Error)]
pub enum PerfEventError {
    #[error(
        "permission denied on the perf counters: run as root, or `sudo sysctl kernel.perf_event_paranoid=0`"
    )]
    PermissionDenied(#[source] io::Error),

    #[error("no event configured")]
    NoEvent,

    #[error("opening the cgroup {0}")]
    OpenCgroup(PathBuf, #[source] io::Error),

    #[error("opening the perf group of cpu {0}")]
    OpenCpuGroup(u32, #[source] io::Error),

    #[error("opening the perf group")]
    OpenGroup(#[source] io::Error),

    #[error("opening the {0} counter")]
    OpenCounter(Event, #[source] io::Error),

    #[error("reading the {0} counter")]
    ReadCounter(Event, #[source] io::Error),

    #[error("reading the perf counters of cpu {0}")]
    ReadCpuGroup(u32, #[source] io::Error),

    #[error("reading the perf counters")]
    ReadGroup(#[source] io::Error),

    #[error("reading {0}")]
    ReadOnlineCpus(PathBuf, #[source] io::Error),

    #[error("cpu {0} is not online")]
    UnavailableCpu(u32),

    #[error("reading counters that were never opened")]
    NotOpened,
}
