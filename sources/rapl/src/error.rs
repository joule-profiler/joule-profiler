use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, RaplError>;

#[derive(Debug, Error)]
pub enum RaplError {
    #[error("unknown domain \"{0}\"")]
    UnknownDomain(String),

    #[error("reading {0}")]
    Read(std::path::PathBuf, #[source] std::io::Error),

    #[error("reading the cpu topology")]
    Topology(#[source] std::io::Error),

    #[error("reading the `power` PMU")]
    Pmu(#[source] std::io::Error),

    #[error("no RAPL domain available")]
    NoDomain,

    #[error("reading {0} is not allowed: run as root, or `sudo chmod -R a+r /sys/class/powercap`")]
    PowercapDenied(std::path::PathBuf),

    #[error("{0} does not hold a number")]
    NotANumber(std::path::PathBuf, #[source] std::num::ParseIntError),

    #[cfg(feature = "backend-perf")]
    #[error("could not build the event")]
    Event(#[source] perf_event::events::error::MissingParameterError),

    #[error("socket {0} does not exist on this machine")]
    UnknownSocket(u32),

    #[error("could not read the package id of cpu {0}")]
    CpuPackage(u32),

    #[error("could not open the counter group for cpu {0}")]
    OpenGroup(u32, #[source] std::io::Error),

    #[error("could not open counter for {0}")]
    OpenCounter(String, #[source] std::io::Error),

    #[error("could not read the counters of cpu {0}")]
    ReadGroup(u32, #[source] std::io::Error),

    #[error("a counter was missing from its group reading")]
    MissingCounter,
}
