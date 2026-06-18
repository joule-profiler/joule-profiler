use thiserror::Error;
use tokio::task::JoinError;

use crate::Processor;

#[derive(Debug, Error)]
pub enum AmdSmiError {
    /// AMD SMI could not find or load the driver.
    #[error(
        "No driver found or loaded to access AMD SMI, check whether you have an AMD GPU or not."
    )]
    NoDriverLoaded,

    /// Unable to find the AMD SMI library.
    #[error(
        "AMD SMI library not found, check whether amd-smi-lib is installed and if you have an AMD GPU device."
    )]
    LibraryNotFound,

    /// Unknown device.
    #[error("Device {0:?} not found.")]
    NoSuchDevice(Processor),

    /// No device with that index found in the devices list.
    #[error("Device with index {0} not found.")]
    NoSuchDeviceFromIndex(usize),

    /// Generic I/O error.
    #[error("I/O error")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// Tokio task join error (async execution failure).
    #[error("Failed to join tokio task: {0}.")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),

    /// AMD SMI library error.
    #[error("AMD SMI error: {0}.")]
    AmdSmiError(#[from] amdsmi::error::AmdSmiError),
}
