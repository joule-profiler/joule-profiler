use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProcfsError>;

#[derive(Debug, Error)]
pub enum ProcfsError {
    #[error("reading /proc")]
    Procfs(#[from] procfs::ProcError),

    #[error("the source was never attached to a process")]
    NotAttached,
}
