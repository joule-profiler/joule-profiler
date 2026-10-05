use thiserror::Error;

pub type Result<T> = std::result::Result<T, NvmlError>;

#[derive(Debug, Error)]
pub enum NvmlError {
    #[error(
        "no driver found or loaded to access NVML, check whether you have an Nvidia GPU or not"
    )]
    NoDriverLoaded,

    #[error("insufficient permissions to access NVML, try running with sudo")]
    NoPermission,

    #[error("no GPU device detected")]
    NoDeviceDetected,

    #[error(transparent)]
    Nvml(#[from] nvml_wrapper::error::NvmlError),
}
