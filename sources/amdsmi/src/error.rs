use thiserror::Error;
use tokio::task::JoinError;

use crate::Processor;

#[derive(Debug, Error)]
pub enum AmdSmiError {
    #[error("Device {0:?} not found.")]
    NoSuchDevice(Processor),

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
    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),

    #[error("AMD SMI error: {0}")]
    AmdSmiError(#[from] amdsmi::error::AmdSmiError),
}
