use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::event::Event;

/// A [`Result`](std::result::Result) with a [`PerfEventError`].
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

    /// The group of a CPU could not be opened, with the cgroup scope.
    #[error("opening the perf group of cpu {0}")]
    OpenGroup(u32, #[source] io::Error),

    /// The group could not be opened, with the pid scope.
    #[error("opening the perf group")]
    OpenLeader(#[source] io::Error),

    #[error("opening the {0} counter")]
    OpenCounter(Event, #[source] io::Error),

    #[error("reading the {0} counter")]
    ReadCounter(Event, #[source] io::Error),

    /// The group of a CPU could not be read, with the cgroup scope.
    #[error("reading the perf counters of cpu {0}")]
    ReadGroup(u32, #[source] io::Error),

    /// The group could not be read, with the pid scope.
    #[error("reading the perf counters")]
    ReadCounters(#[source] io::Error),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("cpu {0} is not online")]
    OfflineCpu(u32),

    /// The counters were read before being opened.
    #[error("reading counters that were never opened")]
    NotOpened,
}
