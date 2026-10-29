use joule_profiler_core::util::poller::PollerPanicked;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, NvmlError>;

/// An error returned by the nvml source.
#[derive(Debug, Error)]
pub enum NvmlError {
    /// The NVIDIA driver is not loaded.
    #[error("the NVIDIA driver is not loaded, check that the machine has an NVIDIA GPU")]
    NoDriverLoaded,

    /// NVML refuses the access.
    #[error("insufficient permissions to access NVML, try running with sudo")]
    NoPermission,

    /// No GPU reports what is asked.
    #[error("no GPU device detected")]
    NoDeviceDetected,

    /// An NVML call failed.
    #[error(transparent)]
    Nvml(#[from] nvml_wrapper::error::NvmlError),

    /// A polling thread panicked.
    #[error(transparent)]
    Poller(#[from] PollerPanicked),
}
