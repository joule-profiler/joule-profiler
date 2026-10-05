use thiserror::Error;

pub type Result<T> = std::result::Result<T, AmdSmiError>;

#[derive(Debug, Error)]
pub enum AmdSmiError {
    #[error(
        "no driver found or loaded to access AMD SMI, check whether you have an AMD GPU or not"
    )]
    NoDriverLoaded,

    #[error(
        "AMD SMI library not found, check whether amd-smi-lib is installed and if you have an AMD GPU"
    )]
    LibraryNotFound,

    #[error("no GPU device detected")]
    NoDeviceDetected,

    #[error("GPU device {0} not found")]
    NoSuchDevice(String),

    #[error(transparent)]
    AmdSmi(#[from] amdsmi::error::AmdSmiError),
}
