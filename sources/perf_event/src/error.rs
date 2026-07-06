use thiserror::Error;

use crate::event::Event;

/// Errors that can occur when using the `perf_event` source.
#[derive(Debug, Error)]
pub enum PerfEventError {
    /// I/O error from the underlying `perf_event` syscall.
    #[error("{0}")]
    IoError(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// Failed to read the value of a specific hardware counter.
    #[error("Error reading counter {0}")]
    ErrorReadingCounter(Event),

    /// Not enough snapshots have been taken to compute the delta between two measures.
    #[error("Not enough measures to compute perf counters differences")]
    NotEnoughSamples,

    /// Failed to parse the online CPU list from sysfs.
    #[error("Failed to parse online CPU list: {0}")]
    ParseCpuList(
        #[from]
        #[source]
        std::num::ParseIntError,
    ),

    /// A blocking task (opening or reading counters) panicked or was cancelled.
    #[error("Failed to join a blocking perf_event task: {0}")]
    JoinError(
        #[from]
        #[source]
        tokio::task::JoinError,
    ),
}
