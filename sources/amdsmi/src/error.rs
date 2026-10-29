use joule_profiler_core::util::poller::PollerPanicked;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, AmdSmiError>;

/// An error returned by the amdsmi source.
#[derive(Debug, Error)]
pub enum AmdSmiError {
    /// The AMD GPU driver is not loaded.
    #[error("the AMD GPU driver is not loaded, check that the machine has an AMD GPU")]
    NoDriverLoaded,

    /// The AMD SMI library is not installed.
    #[error("the AMD SMI library is not found, check that `amd-smi-lib` is installed")]
    LibraryNotFound,

    /// No GPU reports what is asked.
    #[error("no GPU device detected")]
    NoDeviceDetected,

    /// The GPU was not found by AMD SMI.
    #[error("GPU device {0} not found")]
    NoSuchDevice(String),

    /// An AMD SMI call failed.
    #[error(transparent)]
    AmdSmi(#[from] amdsmi::error::AmdSmiError),

    /// A polling thread panicked.
    #[error(transparent)]
    Poller(#[from] PollerPanicked),
}
