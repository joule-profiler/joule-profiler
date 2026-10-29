use std::io;
use std::path::PathBuf;

use joule_profiler_core::util::poller::PollerPanicked;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, RaplError>;

/// An error returned by the RAPL source.
#[derive(Debug, Error)]
pub enum RaplError {
    #[error("no RAPL domain available")]
    NoDomain,

    #[error("socket {0} does not exist on this machine")]
    UnknownSocket(u32),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("{0} does not hold a number")]
    NotANumber(PathBuf, #[source] std::num::ParseIntError),

    #[error("permission denied on {0}: run as root to read the powercap counters")]
    PowercapPermissionDenied(PathBuf),

    #[error("reading the cpu topology")]
    Topology(#[source] io::Error),

    #[error("reading the `power` PMU")]
    Pmu(#[source] io::Error),

    #[cfg(feature = "backend-perf")]
    #[error("could not build the event")]
    Event(#[source] perf_event::events::error::MissingParameterError),

    #[error("opening the {0} counter")]
    OpenCounter(String, #[source] io::Error),

    #[error("reading the {0} counter")]
    ReadCounter(String, #[source] io::Error),

    #[error(transparent)]
    Poller(#[from] PollerPanicked),
}
