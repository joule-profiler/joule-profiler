use joule_profiler_core::util::poller::PollerPanicked;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, NvmlError>;

/// An error returned by the nvml source.
#[derive(Debug, Error)]
pub enum NvmlError {
    #[error("the NVIDIA driver is not loaded, check that the machine has an NVIDIA GPU")]
    NoDriverLoaded,

    #[error("insufficient permissions to access NVML, try running with sudo")]
    NoPermission,

    #[error("no GPU device detected")]
    NoDeviceDetected,

    #[error(transparent)]
    Nvml(#[from] nvml_wrapper::error::NvmlError),

    #[error(transparent)]
    Poller(#[from] PollerPanicked),
}
