use thiserror::Error;
use tokio::task::JoinError;

/// Errors that can occur when using the NVML source.
#[derive(Debug, Error)]
pub enum NvmlError {
    /// A device index present in the old snapshot was not found in the new one.
    #[error("Unknow metric \"{0}\" found in old snapshot")]
    UnknownMetricError(String),

    /// NVML could not find or load the NVIDIA driver.
    #[error(
        "No driver found or loaded to access NVML, check whether you have an Nvidia GPU or not"
    )]
    NoDriverLoaded,

    /// The process lacks the required permissions to access NVML.
    #[error("Insufficient permissions to access NVML. Try running with sudo")]
    NoPermission,

    /// The NVML library detected no device.
    #[error("No GPU device detected.")]
    NoDeviceDetected,

    /// Error propagated from the underlying `nvml_wrapper` library.
    #[error("NVML error: {0}")]
    NvmlError(
        #[from]
        #[source]
        nvml_wrapper::error::NvmlError,
    ),

    /// Generic I/O error.
    #[error("I/O error")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// No device with that index found in the devices list.
    #[error("Device with index {0} not found.")]
    NoSuchDeviceFromIndex(usize),

    /// Tokio task join error (async execution failure).
    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}
