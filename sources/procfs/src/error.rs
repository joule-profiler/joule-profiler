use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProcfsError {
    #[error("Procfs not initialized, call init first")]
    NotInitialized,
    #[error(transparent)]
    Procfs(#[from] procfs::ProcError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
