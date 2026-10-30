use joule_profiler_core::util::poller::PollerPanicked;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, AmdSmiError>;

/// An error returned by the amdsmi source.
#[derive(Debug, Error)]
pub enum AmdSmiError {
    #[error("the AMD GPU driver is not loaded, check that the machine has an AMD GPU")]
    NoDriverLoaded,

    #[error("the AMD SMI library is not found, check that `amd-smi-lib` is installed")]
    LibraryNotFound,

    #[error("no GPU device detected")]
    NoDeviceDetected,

    #[error("GPU device {0} not found")]
    NoSuchDevice(String),

    #[error(transparent)]
    AmdSmi(#[from] amdsmi::error::AmdSmiError),

    #[error(transparent)]
    Poller(#[from] PollerPanicked),
}
