use thiserror::Error;

/// A [`Result`](std::result::Result) with a [`ProcfsError`].
pub type Result<T> = std::result::Result<T, ProcfsError>;

/// An error returned by the procfs source.
#[derive(Debug, Error)]
pub enum ProcfsError {
    #[error("reading /proc")]
    Procfs(#[from] procfs::ProcError),

    #[error("the source was never attached to a process")]
    NotAttached,
}
